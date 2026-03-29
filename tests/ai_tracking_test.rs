use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use blackbox::ai_tracking::{AiToolDetector, ClaudeDetector, CodexDetector};
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

// --- US-012: Codex detector parsing tests ---

/// Helper: create a temp dir mimicking ~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl
fn create_codex_session(base: &std::path::Path, date_parts: (&str, &str, &str), stem: &str, lines: &[&str]) -> std::path::PathBuf {
    let dir = base.join(date_parts.0).join(date_parts.1).join(date_parts.2);
    std::fs::create_dir_all(&dir).unwrap();
    let file_path = dir.join(format!("{stem}.jsonl"));
    std::fs::write(&file_path, lines.join("\n")).unwrap();
    file_path
}

#[test]
fn test_codex_detector_valid_session_inserts_row() {
    let (_tmp, conn) = setup_db();
    let sessions_tmp = TempDir::new().unwrap();
    let sessions_dir = sessions_tmp.path().to_path_buf();

    let meta_line = serde_json::json!({
        "cwd": "/tmp/test-repo",
        "createdAt": "2024-03-15T10:00:00Z",
        "updatedAt": "2024-03-15T10:05:00Z"
    }).to_string();

    create_codex_session(
        &sessions_dir,
        ("2024", "03", "15"),
        "rollout-abc",
        &[&meta_line, r#"{"type":"input"}"#, r#"{"type":"output"}"#, r#"{"type":"input"}"#],
    );

    let detector = CodexDetector::with_sessions_dir(sessions_dir.clone());
    detector.poll(&conn, &[PathBuf::from("/tmp/test-repo")]);

    let (tool, repo, sid, turns): (String, String, String, i64) = conn
        .query_row(
            "SELECT tool, repo_path, session_id, turns FROM ai_sessions LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();

    assert_eq!(tool, "codex");
    assert_eq!(repo, "/tmp/test-repo");
    assert!(sid.contains("2024-03-15") && sid.contains("rollout-abc"), "session_id={sid}");
    assert_eq!(turns, 4); // 1 meta + 3 data lines
}

#[test]
fn test_codex_detector_no_duplicate_on_second_poll() {
    let (_tmp, conn) = setup_db();
    let sessions_tmp = TempDir::new().unwrap();
    let sessions_dir = sessions_tmp.path().to_path_buf();

    let meta_line = serde_json::json!({
        "cwd": "/tmp/test-repo",
        "createdAt": "2024-03-15T10:00:00Z",
        "updatedAt": "2024-03-15T10:05:00Z"
    }).to_string();

    create_codex_session(
        &sessions_dir,
        ("2024", "03", "15"),
        "rollout-dup",
        &[&meta_line, r#"{"type":"input"}"#],
    );

    let detector = CodexDetector::with_sessions_dir(sessions_dir.clone());
    detector.poll(&conn, &[]);
    detector.poll(&conn, &[]);

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM ai_sessions WHERE tool = 'codex'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1, "INSERT OR IGNORE should prevent duplicates");
}

#[test]
fn test_codex_detector_invalid_first_line_skipped() {
    let (_tmp, conn) = setup_db();
    let sessions_tmp = TempDir::new().unwrap();
    let sessions_dir = sessions_tmp.path().to_path_buf();

    create_codex_session(
        &sessions_dir,
        ("2024", "03", "15"),
        "rollout-bad",
        &["NOT VALID JSON", r#"{"type":"input"}"#],
    );

    let detector = CodexDetector::with_sessions_dir(sessions_dir.clone());
    detector.poll(&conn, &[]); // should not panic

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM ai_sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0, "malformed first line should be skipped");
}

#[test]
fn test_codex_detector_missing_sessions_dir_no_error() {
    let (_tmp, conn) = setup_db();
    let nonexistent = PathBuf::from("/tmp/definitely-does-not-exist-codex-sessions");

    let detector = CodexDetector::with_sessions_dir(nonexistent);
    detector.poll(&conn, &[]); // should not panic or error

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM ai_sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn test_codex_detector_tool_name() {
    let detector = CodexDetector::default();
    assert_eq!(detector.tool_name(), "codex");
}
