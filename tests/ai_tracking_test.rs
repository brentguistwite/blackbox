use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use blackbox::ai_tracking::{AiToolDetector, ClaudeDetector};
use blackbox::db;
use rusqlite::Connection;
use tempfile::TempDir;

fn setup_db() -> (TempDir, Connection) {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();
    (tmp, conn)
}

// --- US-011: AiToolDetector trait + ClaudeDetector tests ---

/// TestDetector records whether poll() was called via shared AtomicBool.
struct TestDetector {
    called: Arc<AtomicBool>,
}

impl AiToolDetector for TestDetector {
    fn tool_name(&self) -> &'static str {
        "test-tool"
    }

    fn poll(&self, _conn: &Connection, _watched_repos: &[PathBuf]) {
        self.called.store(true, Ordering::SeqCst);
    }
}

#[test]
fn test_detector_trait_object_safe_and_callable() {
    let (_tmp, conn) = setup_db();
    let called = Arc::new(AtomicBool::new(false));

    let detectors: Vec<Box<dyn AiToolDetector>> = vec![
        Box::new(TestDetector { called: called.clone() }),
    ];

    for d in &detectors {
        d.poll(&conn, &[]);
    }

    assert!(called.load(Ordering::SeqCst), "TestDetector.poll() should have been called");
}

#[test]
fn test_claude_detector_empty_sessions_dir_no_panic() {
    let (_tmp, conn) = setup_db();
    let sessions_tmp = TempDir::new().unwrap();
    let empty_sessions = sessions_tmp.path().join("sessions");
    std::fs::create_dir_all(&empty_sessions).unwrap();
    let empty_projects = sessions_tmp.path().join("projects");
    std::fs::create_dir_all(&empty_projects).unwrap();

    // Poll with empty sessions dir — should not panic
    blackbox::claude_tracking::poll_claude_sessions_with_paths(
        &conn,
        &[],
        Some(&empty_sessions),
        Some(&empty_projects),
    );

    // No rows inserted
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM ai_sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn test_claude_detector_valid_session_inserts_row() {
    let (_tmp, conn) = setup_db();
    let sessions_tmp = TempDir::new().unwrap();
    let sessions_dir = sessions_tmp.path().join("sessions");
    std::fs::create_dir_all(&sessions_dir).unwrap();
    let projects_dir = sessions_tmp.path().join("projects");
    std::fs::create_dir_all(&projects_dir).unwrap();

    // Write a valid session JSON file
    let session_json = serde_json::json!({
        "pid": 99999,
        "sessionId": "test-session-abc",
        "cwd": "/tmp/test-repo",
        "startedAt": 1711929600000_u64
    });
    std::fs::write(
        sessions_dir.join("test-session-abc.json"),
        serde_json::to_string(&session_json).unwrap(),
    )
    .unwrap();

    blackbox::claude_tracking::poll_claude_sessions_with_paths(
        &conn,
        &[PathBuf::from("/tmp/test-repo")],
        Some(&sessions_dir),
        Some(&projects_dir),
    );

    // Should have inserted one row with tool='claude-code'
    let (tool, repo, sid): (String, String, String) = conn
        .query_row(
            "SELECT tool, repo_path, session_id FROM ai_sessions LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(tool, "claude-code");
    assert_eq!(repo, "/tmp/test-repo");
    assert_eq!(sid, "test-session-abc");
}

#[test]
fn test_claude_detector_tool_name() {
    let detector = ClaudeDetector::default();
    assert_eq!(detector.tool_name(), "claude-code");
}
