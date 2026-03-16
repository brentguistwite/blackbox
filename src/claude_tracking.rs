use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde::Deserialize;

use crate::db;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionFile {
    pub pid: u64,
    pub session_id: String,
    pub cwd: String,
    pub started_at: u64, // Unix timestamp in milliseconds
}

/// Encode a path the way Claude Code does: /Users/foo/bar → -Users-foo-bar
pub fn encode_project_path(path: &str) -> String {
    path.replace('/', "-")
}

/// Check if a process is still running (same pattern as daemon.rs stale PID detection).
fn is_process_running(pid: u64) -> bool {
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), None).is_ok()
}

/// Read all session files from a sessions directory.
fn read_session_files(sessions_dir: &Path) -> Vec<SessionFile> {
    let entries = match std::fs::read_dir(sessions_dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut sessions = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "json")
            && let Ok(content) = std::fs::read_to_string(&path)
            && let Ok(session) = serde_json::from_str::<SessionFile>(&content)
        {
            sessions.push(session);
        }
    }
    sessions
}

/// Convert millis timestamp to RFC3339 string.
fn millis_to_rfc3339(millis: u64) -> String {
    chrono::DateTime::from_timestamp_millis(millis as i64)
        .unwrap_or_default()
        .to_rfc3339()
}

/// Count turns in a JSONL conversation log file. Each line is a turn.
fn count_turns(jsonl_path: &Path) -> Option<i64> {
    let content = std::fs::read_to_string(jsonl_path).ok()?;
    Some(content.lines().filter(|l| !l.trim().is_empty()).count() as i64)
}

/// Find the JSONL conversation log for a session in the projects directory.
fn find_session_log(projects_dir: &Path, session_cwd: &str, session_id: &str) -> Option<PathBuf> {
    let encoded = encode_project_path(session_cwd);
    let project_dir = projects_dir.join(&encoded);
    let jsonl = project_dir.join(format!("{}.jsonl", session_id));
    if jsonl.exists() {
        Some(jsonl)
    } else {
        None
    }
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
    // Fallback: use the cwd directly
    session_cwd.to_string()
}

/// Main entry point: poll Claude Code sessions and record to DB.
/// Reads ~/.claude/sessions/ for active sessions, detects ended sessions,
/// counts turns from conversation logs.
pub fn poll_claude_sessions(
    conn: &Connection,
    watched_repos: &[PathBuf],
) {
    poll_claude_sessions_with_paths(conn, watched_repos, None, None);
}

/// Testable version with explicit paths for claude_dir components.
pub fn poll_claude_sessions_with_paths(
    conn: &Connection,
    watched_repos: &[PathBuf],
    sessions_dir: Option<&Path>,
    projects_dir: Option<&Path>,
) {
    let home = match etcetera::home_dir() {
        Ok(h) => h,
        Err(_) => return,
    };

    let default_claude = home.join(".claude");
    let default_sessions = default_claude.join("sessions");
    let default_projects = default_claude.join("projects");
    let sessions_path = sessions_dir.unwrap_or(&default_sessions);
    let projects_path = projects_dir.unwrap_or(&default_projects);

    if !sessions_path.exists() {
        log::debug!("Claude sessions dir not found, skipping AI session tracking");
        return;
    }

    // Phase 1: Read active session files and record new ones
    let session_files = read_session_files(sessions_path);
    let mut active_pids: std::collections::HashMap<String, u64> = std::collections::HashMap::new();

    for session in &session_files {
        active_pids.insert(session.session_id.clone(), session.pid);

        let repo_path = map_to_repo(&session.cwd, watched_repos);
        let started_at = millis_to_rfc3339(session.started_at);

        match db::insert_ai_session(conn, &repo_path, &session.session_id, &started_at) {
            Ok(true) => log::debug!("Recorded new AI session: {} in {}", session.session_id, repo_path),
            Ok(false) => {} // already exists
            Err(e) => log::warn!("Failed to insert AI session {}: {}", session.session_id, e),
        }
    }

    // Phase 2: Check DB sessions that are still "active" (no ended_at)
    // If their PID is no longer running and not in current session files, mark ended
    let active_session_ids = match db::get_active_sessions(conn) {
        Ok(ids) => ids,
        Err(e) => {
            log::warn!("Failed to query active sessions: {}", e);
            return;
        }
    };

    for session_id in &active_session_ids {
        let still_running = active_pids
            .get(session_id)
            .is_some_and(|&pid| is_process_running(pid));

        if !still_running {
            let ended_at = chrono::Utc::now().to_rfc3339();

            // Try to get turn count from conversation log
            // We need the cwd for the session to find the log — query it from session files
            let turns = session_files
                .iter()
                .find(|s| s.session_id == *session_id)
                .and_then(|s| find_session_log(projects_path, &s.cwd, session_id))
                .and_then(|path| count_turns(&path));

            match db::update_session_ended(conn, session_id, &ended_at, turns) {
                Ok(true) => log::debug!("Marked AI session {} as ended (turns: {:?})", session_id, turns),
                Ok(false) => {} // already ended
                Err(e) => log::warn!("Failed to update AI session {}: {}", session_id, e),
            }
        }
    }
}
