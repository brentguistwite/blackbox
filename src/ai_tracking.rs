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
pub fn processes_matching(name_pattern: &str) -> Vec<u32> {
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
pub fn is_any_process_running(name_pattern: &str) -> bool {
    !processes_matching(name_pattern).is_empty()
}

/// Codex CLI session detector.
/// Scans ~/.codex/sessions/ recursively for rollout-*.jsonl files.
#[derive(Default)]
pub struct CodexDetector {
    sessions_dir: Option<PathBuf>,
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

            // Update turns + last_active_at from updated_at
            let turns = count_lines(path);
            if let Some(t) = turns {
                let _ = conn.execute(
                    "UPDATE ai_sessions SET turns = ?1 WHERE session_id = ?2",
                    rusqlite::params![t, session_id],
                );
            }
            let _ = db::update_session_last_active(conn, &session_id, &meta.updated_at);

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

/// Copilot CLI session detector.
/// Scans ~/.copilot/session-state/<uuid>/workspace.yaml files.
#[derive(Default)]
pub struct CopilotDetector {
    session_state_dir: Option<PathBuf>,
}

impl CopilotDetector {
    pub fn with_session_state_dir(dir: PathBuf) -> Self {
        Self { session_state_dir: Some(dir) }
    }

    fn resolve_session_state_dir(&self) -> Option<PathBuf> {
        if let Some(dir) = &self.session_state_dir {
            return Some(dir.clone());
        }
        let home = etcetera::home_dir().ok()?;
        Some(home.join(".copilot").join("session-state"))
    }
}

#[derive(Deserialize)]
struct CopilotWorkspace {
    cwd: Option<String>,
}

/// Get file mtime as RFC3339 string. Returns None if metadata unavailable.
fn mtime_rfc3339(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    let dt: chrono::DateTime<Utc> = modified.into();
    Some(dt.to_rfc3339())
}

/// Check if file was modified within the last `minutes` minutes.
fn modified_within_minutes(path: &Path, minutes: i64) -> bool {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return false,
    };
    let modified = match meta.modified() {
        Ok(m) => m,
        Err(_) => return false,
    };
    let dt: chrono::DateTime<Utc> = modified.into();
    Utc::now().signed_duration_since(dt) < chrono::Duration::minutes(minutes)
}

impl AiToolDetector for CopilotDetector {
    fn tool_name(&self) -> &'static str {
        "copilot-cli"
    }

    fn poll(&self, conn: &Connection, watched_repos: &[PathBuf]) {
        let state_dir = match self.resolve_session_state_dir() {
            Some(d) if d.exists() => d,
            _ => {
                log::debug!("copilot-cli data dir not found, skipping");
                return;
            }
        };

        let entries = match std::fs::read_dir(&state_dir) {
            Ok(e) => e,
            Err(_) => {
                log::debug!("copilot-cli: failed to read session-state dir");
                return;
            }
        };

        let copilot_running = is_any_process_running("copilot");

        for entry in entries.filter_map(|e| e.ok()) {
            let subdir = entry.path();
            if !subdir.is_dir() {
                continue;
            }

            let session_id = match subdir.file_name().and_then(|n| n.to_str()) {
                Some(name) => name.to_string(),
                None => continue,
            };

            let workspace_path = subdir.join("workspace.yaml");
            if !workspace_path.exists() {
                continue;
            }

            // Parse workspace.yaml
            let yaml_content = match std::fs::read_to_string(&workspace_path) {
                Ok(c) => c,
                Err(_) => {
                    log::debug!("copilot-cli: failed to read {}", workspace_path.display());
                    continue;
                }
            };
            let workspace: CopilotWorkspace = match serde_yaml::from_str(&yaml_content) {
                Ok(w) => w,
                Err(_) => {
                    log::debug!("copilot-cli: malformed workspace.yaml in {}", subdir.display());
                    continue;
                }
            };
            let cwd = match workspace.cwd {
                Some(c) if !c.is_empty() => c,
                _ => continue,
            };

            let repo_path = map_to_repo(&cwd, watched_repos);
            let started_at = mtime_rfc3339(&workspace_path)
                .unwrap_or_else(|| Utc::now().to_rfc3339());

            match db::insert_ai_session(conn, "copilot-cli", &repo_path, &session_id, &started_at) {
                Ok(true) => log::debug!("Recorded copilot-cli session: {}", session_id),
                Ok(false) => {} // already exists
                Err(e) => log::warn!("Failed to insert copilot-cli session {}: {}", session_id, e),
            }

            // Update turns from events.jsonl + last_active_at from workspace mtime
            let events_path = subdir.join("events.jsonl");
            if let Some(turns) = count_lines(&events_path) {
                let _ = conn.execute(
                    "UPDATE ai_sessions SET turns = ?1 WHERE session_id = ?2",
                    rusqlite::params![turns, session_id],
                );
            }
            if let Some(mtime) = mtime_rfc3339(&workspace_path) {
                let _ = db::update_session_last_active(conn, &session_id, &mtime);
            }

            // Check if session should be marked ended:
            // copilot process not running OR workspace.yaml mtime older than 5 minutes
            let is_recent = modified_within_minutes(&workspace_path, 5);

            if !copilot_running || !is_recent {
                let ended_at = mtime_rfc3339(&workspace_path)
                    .unwrap_or_else(|| Utc::now().to_rfc3339());
                let turns = count_lines(&events_path);
                let _ = db::update_session_ended(conn, &session_id, &ended_at, turns);
            }
        }
    }
}

/// Cursor workspace.json schema: `{"folder": "/path/to/project"}`
#[derive(Deserialize)]
struct CursorWorkspace {
    folder: Option<String>,
}

/// Cursor session detector.
/// Scans Cursor's workspaceStorage for workspace.json files.
#[derive(Default)]
pub struct CursorDetector {
    workspace_dir: Option<PathBuf>,
}

impl CursorDetector {
    pub fn with_workspace_dir(dir: PathBuf) -> Self {
        Self { workspace_dir: Some(dir) }
    }

    fn resolve_workspace_dir(&self) -> Option<PathBuf> {
        if let Some(dir) = &self.workspace_dir {
            return Some(dir.clone());
        }
        let home = etcetera::home_dir().ok()?;
        #[cfg(target_os = "macos")]
        let base = home.join("Library/Application Support/Cursor/User/workspaceStorage");
        #[cfg(not(target_os = "macos"))]
        let base = home.join(".config/Cursor/User/workspaceStorage");
        Some(base)
    }
}

impl AiToolDetector for CursorDetector {
    fn tool_name(&self) -> &'static str {
        "cursor"
    }

    fn poll(&self, conn: &Connection, watched_repos: &[PathBuf]) {
        let ws_dir = match self.resolve_workspace_dir() {
            Some(d) if d.exists() => d,
            _ => {
                log::debug!("cursor data dir not found, skipping");
                return;
            }
        };

        let entries = match std::fs::read_dir(&ws_dir) {
            Ok(e) => e,
            Err(_) => {
                log::debug!("cursor: failed to read workspaceStorage dir");
                return;
            }
        };

        let cursor_running = is_any_process_running("Cursor");

        for entry in entries.filter_map(|e| e.ok()) {
            let subdir = entry.path();
            if !subdir.is_dir() {
                continue;
            }

            let hash = match subdir.file_name().and_then(|n| n.to_str()) {
                Some(name) => name.to_string(),
                None => continue,
            };

            let workspace_path = subdir.join("workspace.json");
            if !workspace_path.exists() {
                continue;
            }

            // Parse workspace.json
            let json_content = match std::fs::read_to_string(&workspace_path) {
                Ok(c) => c,
                Err(_) => {
                    log::debug!("cursor: failed to read {}", workspace_path.display());
                    continue;
                }
            };
            let workspace: CursorWorkspace = match serde_json::from_str(&json_content) {
                Ok(w) => w,
                Err(_) => {
                    log::debug!("cursor: malformed workspace.json in {}", subdir.display());
                    continue;
                }
            };
            let folder = match workspace.folder {
                Some(f) if !f.is_empty() && !f.contains("://") => f,
                _ => continue,
            };

            let session_id = format!("cursor-{hash}");
            let repo_path = map_to_repo(&folder, watched_repos);
            let started_at = mtime_rfc3339(&workspace_path)
                .unwrap_or_else(|| Utc::now().to_rfc3339());

            match db::insert_ai_session(conn, "cursor", &repo_path, &session_id, &started_at) {
                Ok(true) => log::debug!("Recorded cursor session: {}", session_id),
                Ok(false) => {} // already exists
                Err(e) => log::warn!("Failed to insert cursor session {}: {}", session_id, e),
            }

            // Update last_active_at from workspace.json mtime
            if let Some(mtime) = mtime_rfc3339(&workspace_path) {
                let _ = db::update_session_last_active(conn, &session_id, &mtime);
            }

            // Still running: Cursor process running AND workspace.json mtime within 30 min
            let is_recent = modified_within_minutes(&workspace_path, 30);
            if !cursor_running || !is_recent {
                let ended_at = Utc::now().to_rfc3339();
                let _ = db::update_session_ended(conn, &session_id, &ended_at, None);
            }
        }
    }
}

/// Windsurf session detector.
/// Tries workspaceStorage (same layout as Cursor), falls back to process-only detection.
#[derive(Default)]
pub struct WindsurfDetector {
    workspace_dir: Option<PathBuf>,
}

impl WindsurfDetector {
    pub fn with_workspace_dir(dir: PathBuf) -> Self {
        Self { workspace_dir: Some(dir) }
    }

    fn resolve_workspace_dir(&self) -> Option<PathBuf> {
        if let Some(dir) = &self.workspace_dir {
            return Some(dir.clone());
        }
        let home = etcetera::home_dir().ok()?;
        #[cfg(target_os = "macos")]
        let base = home.join("Library/Application Support/Windsurf/User/workspaceStorage");
        #[cfg(not(target_os = "macos"))]
        let base = home.join(".config/Windsurf/User/workspaceStorage");
        Some(base)
    }

    /// Workspace-based detection (same structure as Cursor).
    /// Returns true if workspace dir existed and was scanned.
    fn poll_workspace_mode(&self, conn: &Connection, watched_repos: &[PathBuf]) -> bool {
        let ws_dir = match self.resolve_workspace_dir() {
            Some(d) if d.exists() => d,
            _ => return false,
        };

        let entries = match std::fs::read_dir(&ws_dir) {
            Ok(e) => e,
            Err(_) => {
                log::debug!("windsurf: failed to read workspaceStorage dir");
                return false;
            }
        };

        let windsurf_running = is_any_process_running("Windsurf");

        for entry in entries.filter_map(|e| e.ok()) {
            let subdir = entry.path();
            if !subdir.is_dir() {
                continue;
            }

            let hash = match subdir.file_name().and_then(|n| n.to_str()) {
                Some(name) => name.to_string(),
                None => continue,
            };

            let workspace_path = subdir.join("workspace.json");
            if !workspace_path.exists() {
                continue;
            }

            let json_content = match std::fs::read_to_string(&workspace_path) {
                Ok(c) => c,
                Err(_) => {
                    log::debug!("windsurf: failed to read {}", workspace_path.display());
                    continue;
                }
            };
            let workspace: CursorWorkspace = match serde_json::from_str(&json_content) {
                Ok(w) => w,
                Err(_) => {
                    log::debug!("windsurf: malformed workspace.json in {}", subdir.display());
                    continue;
                }
            };
            let folder = match workspace.folder {
                Some(f) if !f.is_empty() && !f.contains("://") => f,
                _ => continue,
            };

            let session_id = format!("windsurf-{hash}");
            let repo_path = map_to_repo(&folder, watched_repos);
            let started_at = mtime_rfc3339(&workspace_path)
                .unwrap_or_else(|| Utc::now().to_rfc3339());

            match db::insert_ai_session(conn, "windsurf", &repo_path, &session_id, &started_at) {
                Ok(true) => log::debug!("Recorded windsurf session: {}", session_id),
                Ok(false) => {}
                Err(e) => log::warn!("Failed to insert windsurf session {}: {}", session_id, e),
            }

            // Update last_active_at from workspace.json mtime
            if let Some(mtime) = mtime_rfc3339(&workspace_path) {
                let _ = db::update_session_last_active(conn, &session_id, &mtime);
            }

            let is_recent = modified_within_minutes(&workspace_path, 30);
            if !windsurf_running || !is_recent {
                let ended_at = Utc::now().to_rfc3339();
                let _ = db::update_session_ended(conn, &session_id, &ended_at, None);
            }
        }

        true
    }

    /// Process-only fallback: create synthetic sessions for watched repos with recent git activity.
    fn poll_process_mode(&self, conn: &Connection, watched_repos: &[PathBuf]) {
        if !is_any_process_running("Windsurf") {
            // Not running → mark all open windsurf sessions as ended
            if let Ok(active) = db::get_active_sessions_by_tool(conn, "windsurf") {
                let now = Utc::now().to_rfc3339();
                for sid in active {
                    let _ = db::update_session_ended(conn, &sid, &now, None);
                }
            }
            return;
        }

        // Windsurf is running — create synthetic sessions for repos with recent activity
        let cutoff = (Utc::now() - chrono::Duration::minutes(30)).to_rfc3339();
        let recent_repos: Vec<String> = conn
            .prepare(
                "SELECT DISTINCT repo_path FROM git_activity WHERE timestamp > ?1",
            )
            .and_then(|mut stmt| {
                let rows = stmt
                    .query_map(rusqlite::params![cutoff], |r| r.get(0))?
                    .filter_map(|r| r.ok())
                    .collect();
                Ok(rows)
            })
            .unwrap_or_default();

        let today = Utc::now().format("%Y-%m-%d").to_string();
        let midnight = format!("{today}T00:00:00Z");

        for repo in &recent_repos {
            // Only create sessions for watched repos
            let repo_path = Path::new(repo);
            let is_watched = watched_repos.iter().any(|w| repo_path.starts_with(w) || repo_path == w.as_path());
            if !is_watched && !watched_repos.is_empty() {
                continue;
            }

            let slug = Path::new(repo)
                .file_name()
                .and_then(|f| f.to_str())
                .unwrap_or("unknown");
            let session_id = format!("windsurf-{slug}-{today}");

            match db::insert_ai_session(conn, "windsurf", repo, &session_id, &midnight) {
                Ok(true) => log::debug!("Recorded synthetic windsurf session: {}", session_id),
                Ok(false) => {}
                Err(e) => log::warn!("Failed to insert windsurf session {}: {}", session_id, e),
            }
        }
    }
}

impl AiToolDetector for WindsurfDetector {
    fn tool_name(&self) -> &'static str {
        "windsurf"
    }

    fn poll(&self, conn: &Connection, watched_repos: &[PathBuf]) {
        // Try workspace-based detection first; fall back to process-only
        if !self.poll_workspace_mode(conn, watched_repos) {
            self.poll_process_mode(conn, watched_repos);
        }
    }
}

/// Poll all registered AI tool detectors.
pub fn poll_all_ai_sessions(conn: &Connection, watched_repos: &[PathBuf]) {
    let detectors: Vec<Box<dyn AiToolDetector>> = vec![
        Box::new(ClaudeDetector),
        Box::new(CodexDetector::default()),
        Box::new(CopilotDetector::default()),
        Box::new(CursorDetector::default()),
        Box::new(WindsurfDetector::default()),
    ];
    for detector in &detectors {
        detector.poll(conn, watched_repos);
    }
}
