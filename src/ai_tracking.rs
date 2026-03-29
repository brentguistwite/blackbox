use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::Utc;
use rusqlite::Connection;
use serde::Deserialize;
use walkdir::WalkDir;

use crate::claude_tracking;
use crate::db;

/// Shared interface for all AI tool session detectors.
/// Object-safe so detectors can be stored as `Vec<Box<dyn AiToolDetector>>`.
pub trait AiToolDetector {
    fn tool_name(&self) -> &'static str;
    fn poll(&self, conn: &Connection, watched_repos: &[PathBuf]);
}

/// Wraps existing Claude Code session tracking logic.
#[derive(Default)]
pub struct ClaudeDetector;

impl AiToolDetector for ClaudeDetector {
    fn tool_name(&self) -> &'static str {
        "claude-code"
    }

    fn poll(&self, conn: &Connection, watched_repos: &[PathBuf]) {
        claude_tracking::poll_claude_sessions(conn, watched_repos);
    }
}

/// Return PIDs of running processes whose name matches `name_pattern` (case-insensitive).
/// Uses `pgrep -i` subprocess. Returns empty vec on any failure.
fn processes_matching(name_pattern: &str) -> Vec<u32> {
    let pattern = name_pattern.to_string();
    let handle = std::thread::spawn(move || {
        Command::new("pgrep")
            .args(["-i", &pattern])
            .output()
    });
    match handle.join() {
        Ok(Ok(out)) if out.status.success() => {
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter_map(|l| l.trim().parse::<u32>().ok())
                .collect()
        }
        _ => vec![],
    }
}

/// Check if any process matching `name_pattern` is currently running.
fn is_any_process_running(name_pattern: &str) -> bool {
    !processes_matching(name_pattern).is_empty()
}

/// Codex CLI session detector.
/// Scans ~/.codex/sessions/ recursively for rollout-*.jsonl files.
pub struct CodexDetector {
    sessions_dir: Option<PathBuf>,
}

impl Default for CodexDetector {
    fn default() -> Self {
        Self { sessions_dir: None }
    }
}

impl CodexDetector {
    pub fn with_sessions_dir(dir: PathBuf) -> Self {
        Self { sessions_dir: Some(dir) }
    }

    fn resolve_sessions_dir(&self) -> Option<PathBuf> {
        if let Some(dir) = &self.sessions_dir {
            return Some(dir.clone());
        }
        let home = etcetera::home_dir().ok()?;
        Some(home.join(".codex").join("sessions"))
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexSessionMeta {
    cwd: String,
    created_at: String,
    updated_at: String,
}

/// Map a session's cwd to a watched repo path. Returns the repo path if cwd is
/// within a watched repo, or the cwd itself as a fallback.
fn map_to_repo(session_cwd: &str, watched_repos: &[PathBuf]) -> String {
    let cwd = Path::new(session_cwd);
    for repo in watched_repos {
        if cwd.starts_with(repo) || cwd == repo.as_path() {
            return repo.to_string_lossy().to_string();
        }
    }
    session_cwd.to_string()
}

/// Derive session ID from a codex JSONL file path.
/// Path like `.../2024/03/15/rollout-abc123.jsonl` → `2024-03-15-rollout-abc123`
fn codex_session_id(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    // Walk up: file → DD dir → MM dir → YYYY dir
    let day_dir = path.parent()?;
    let dd = day_dir.file_name()?.to_str()?;
    let mm = day_dir.parent()?.file_name()?.to_str()?;
    let yyyy = day_dir.parent()?.parent()?.file_name()?.to_str()?;
    Some(format!("{yyyy}-{mm}-{dd}-{stem}"))
}

/// Count non-empty lines in a file.
fn count_lines(path: &Path) -> Option<i64> {
    let content = std::fs::read_to_string(path).ok()?;
    Some(content.lines().filter(|l| !l.trim().is_empty()).count() as i64)
}

impl AiToolDetector for CodexDetector {
    fn tool_name(&self) -> &'static str {
        "codex"
    }

    fn poll(&self, conn: &Connection, watched_repos: &[PathBuf]) {
        let sessions_dir = match self.resolve_sessions_dir() {
            Some(d) if d.exists() => d,
            _ => {
                log::debug!("codex data dir not found, skipping");
                return;
            }
        };

        // Phase 1: scan for rollout-*.jsonl files recursively, insert new sessions
        for entry in WalkDir::new(&sessions_dir).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let fname = match path.file_name().and_then(|f| f.to_str()) {
                Some(f) if f.starts_with("rollout-") && f.ends_with(".jsonl") => f,
                _ => continue,
            };
            let _ = fname; // used for filter only

            // Parse first line as CodexSessionMeta
            let content = match std::fs::read_to_string(path) {
                Ok(c) => c,
                Err(_) => {
                    log::debug!("codex: failed to read {}", path.display());
                    continue;
                }
            };
            let first_line = match content.lines().next() {
                Some(l) if !l.trim().is_empty() => l,
                _ => continue,
            };
            let meta: CodexSessionMeta = match serde_json::from_str(first_line) {
                Ok(m) => m,
                Err(_) => {
                    log::debug!("codex: malformed first line in {}", path.display());
                    continue;
                }
            };

            let session_id = match codex_session_id(path) {
                Some(id) => id,
                None => continue,
            };

            let repo_path = map_to_repo(&meta.cwd, watched_repos);

            match db::insert_ai_session(conn, "codex", &repo_path, &session_id, &meta.created_at) {
                Ok(true) => log::debug!("Recorded codex session: {}", session_id),
                Ok(false) => {} // already exists
                Err(e) => log::warn!("Failed to insert codex session {}: {}", session_id, e),
            }

            // Update turns
            let turns = count_lines(path);
            if let Some(t) = turns {
                let _ = conn.execute(
                    "UPDATE ai_sessions SET turns = ?1 WHERE session_id = ?2",
                    rusqlite::params![t, session_id],
                );
            }

            // Check if session should be marked ended:
            // codex process not running OR updated_at older than 5 minutes
            let codex_running = is_any_process_running("codex");
            let is_stale = chrono::DateTime::parse_from_rfc3339(&meta.updated_at)
                .map(|dt| Utc::now().signed_duration_since(dt) > chrono::Duration::minutes(5))
                .unwrap_or(true);

            if !codex_running || is_stale {
                let _ = db::update_session_ended(conn, &session_id, &meta.updated_at, turns);
            }
        }
    }
}

/// Poll all registered AI tool detectors.
pub fn poll_all_ai_sessions(conn: &Connection, watched_repos: &[PathBuf]) {
    let detectors: Vec<Box<dyn AiToolDetector>> = vec![
        Box::new(ClaudeDetector::default()),
        Box::new(CodexDetector::default()),
    ];
    for detector in &detectors {
        detector.poll(conn, watched_repos);
    }
}
