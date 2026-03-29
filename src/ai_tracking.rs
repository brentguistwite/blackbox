use std::path::PathBuf;
use std::process::Command;

use rusqlite::Connection;

use crate::claude_tracking;

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

/// Poll all registered AI tool detectors.
pub fn poll_all_ai_sessions(conn: &Connection, watched_repos: &[PathBuf]) {
    let detectors: Vec<Box<dyn AiToolDetector>> = vec![
        Box::new(ClaudeDetector::default()),
    ];
    for detector in &detectors {
        detector.poll(conn, watched_repos);
    }
}
