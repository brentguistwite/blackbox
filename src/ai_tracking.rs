use std::path::PathBuf;

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

/// Poll all registered AI tool detectors.
pub fn poll_all_ai_sessions(conn: &Connection, watched_repos: &[PathBuf]) {
    let detectors: Vec<Box<dyn AiToolDetector>> = vec![
        Box::new(ClaudeDetector::default()),
    ];
    for detector in &detectors {
        detector.poll(conn, watched_repos);
    }
}
