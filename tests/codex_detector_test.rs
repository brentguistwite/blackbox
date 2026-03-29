use blackbox::ai_tracking::{AiToolDetector, CodexDetector};
use blackbox::db;
use tempfile::TempDir;

fn setup_db(tmp: &TempDir) -> rusqlite::Connection {
    let db_path = tmp.path().join("test.db");
    db::open_db(&db_path).unwrap()
}

#[test]
fn codex_nonexistent_sessions_dir_no_panic() {
    let tmp = TempDir::new().unwrap();
    let conn = setup_db(&tmp);
    let nonexistent = tmp.path().join("no-such-dir");

    let detector = CodexDetector::with_sessions_dir(nonexistent);
    detector.poll(&conn, &[]);

    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM ai_sessions WHERE tool = 'codex'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn codex_valid_session_inserts_db_row() {
    let tmp = TempDir::new().unwrap();
    let conn = setup_db(&tmp);

    // Create ~/.codex/sessions/2024/03/15/rollout-abc123.jsonl
    let sessions_dir = tmp.path().join("sessions");
    let day_dir = sessions_dir.join("2024").join("03").join("15");
    std::fs::create_dir_all(&day_dir).unwrap();

    let meta = r#"{"cwd":"/tmp/myrepo","createdAt":"2024-03-15T10:00:00Z","updatedAt":"2024-03-15T10:05:00Z"}"#;
    let jsonl = format!("{}\n{}\n{}\n", meta, r#"{"type":"turn"}"#, r#"{"type":"turn"}"#);
    std::fs::write(day_dir.join("rollout-abc123.jsonl"), &jsonl).unwrap();

    let repo = tmp.path().join("myrepo");
    std::fs::create_dir_all(&repo).unwrap();

    let detector = CodexDetector::with_sessions_dir(sessions_dir);
    detector.poll(&conn, &[repo]);

    // Verify row exists with correct fields
    let (tool, session_id, started_at): (String, String, String) = conn
        .query_row(
            "SELECT tool, session_id, started_at FROM ai_sessions WHERE tool = 'codex'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();

    assert_eq!(tool, "codex");
    assert_eq!(session_id, "2024-03-15-rollout-abc123");
    assert_eq!(started_at, "2024-03-15T10:00:00Z");

    // Turns = 3 non-empty lines (meta + 2 turn lines)
    let turns: Option<i64> = conn
        .query_row(
            "SELECT turns FROM ai_sessions WHERE session_id = '2024-03-15-rollout-abc123'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(turns, Some(3));
}

#[test]
fn codex_dedup_on_second_poll() {
    let tmp = TempDir::new().unwrap();
    let conn = setup_db(&tmp);

    let sessions_dir = tmp.path().join("sessions");
    let day_dir = sessions_dir.join("2024").join("01").join("01");
    std::fs::create_dir_all(&day_dir).unwrap();

    let meta = r#"{"cwd":"/tmp/repo","createdAt":"2024-01-01T00:00:00Z","updatedAt":"2024-01-01T00:01:00Z"}"#;
    std::fs::write(day_dir.join("rollout-dup.jsonl"), format!("{}\n", meta)).unwrap();

    let detector = CodexDetector::with_sessions_dir(sessions_dir);
    detector.poll(&conn, &[]);
    detector.poll(&conn, &[]);

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM ai_sessions WHERE tool = 'codex'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn codex_malformed_first_line_skipped() {
    let tmp = TempDir::new().unwrap();
    let conn = setup_db(&tmp);

    let sessions_dir = tmp.path().join("sessions");
    let day_dir = sessions_dir.join("2024").join("06").join("01");
    std::fs::create_dir_all(&day_dir).unwrap();

    // Bad first line
    std::fs::write(day_dir.join("rollout-bad.jsonl"), "not valid json\n").unwrap();

    // Valid file alongside
    let meta = r#"{"cwd":"/tmp/repo","createdAt":"2024-06-01T00:00:00Z","updatedAt":"2024-06-01T00:01:00Z"}"#;
    std::fs::write(day_dir.join("rollout-good.jsonl"), format!("{}\n", meta)).unwrap();

    let detector = CodexDetector::with_sessions_dir(sessions_dir);
    detector.poll(&conn, &[]);

    // Only the valid one should be inserted
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM ai_sessions WHERE tool = 'codex'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);

    let sid: String = conn
        .query_row("SELECT session_id FROM ai_sessions WHERE tool = 'codex'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(sid, "2024-06-01-rollout-good");
}

#[test]
fn codex_maps_cwd_to_watched_repo() {
    let tmp = TempDir::new().unwrap();
    let conn = setup_db(&tmp);

    let repo_root = tmp.path().join("project");
    std::fs::create_dir_all(&repo_root).unwrap();
    // Session cwd is a subdir of the watched repo
    let session_cwd = repo_root.join("src").join("lib");
    std::fs::create_dir_all(&session_cwd).unwrap();

    let sessions_dir = tmp.path().join("sessions");
    let day_dir = sessions_dir.join("2024").join("07").join("20");
    std::fs::create_dir_all(&day_dir).unwrap();

    let meta = format!(
        r#"{{"cwd":"{}","createdAt":"2024-07-20T00:00:00Z","updatedAt":"2024-07-20T00:01:00Z"}}"#,
        session_cwd.to_string_lossy()
    );
    std::fs::write(day_dir.join("rollout-map.jsonl"), format!("{}\n", meta)).unwrap();

    let detector = CodexDetector::with_sessions_dir(sessions_dir);
    detector.poll(&conn, &[repo_root.clone()]);

    let repo_path: String = conn
        .query_row("SELECT repo_path FROM ai_sessions WHERE tool = 'codex'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(repo_path, repo_root.to_string_lossy());
}
