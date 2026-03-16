use blackbox::claude_tracking;
use blackbox::db;
use std::path::PathBuf;
use tempfile::TempDir;

fn setup_db(tmp: &TempDir) -> rusqlite::Connection {
    let db_path = tmp.path().join("test.db");
    db::open_db(&db_path).unwrap()
}

#[test]
fn test_encode_project_path() {
    assert_eq!(
        claude_tracking::encode_project_path("/Users/brent/Documents/flosports/blackbox"),
        "-Users-brent-Documents-flosports-blackbox"
    );
}

#[test]
fn test_encode_project_path_simple() {
    assert_eq!(claude_tracking::encode_project_path("/tmp/repo"), "-tmp-repo");
}

#[test]
fn test_poll_sessions_discovers_active_session() {
    let tmp = TempDir::new().unwrap();
    let conn = setup_db(&tmp);

    // Create fake sessions dir with a session file
    let sessions_dir = tmp.path().join("sessions");
    std::fs::create_dir_all(&sessions_dir).unwrap();

    // Use current PID so it appears "running"
    let pid = std::process::id();
    let repo_path = tmp.path().join("myrepo");
    std::fs::create_dir_all(&repo_path).unwrap();

    let session_json = format!(
        r#"{{"pid":{},"sessionId":"abc-123","cwd":"{}","startedAt":1773674448026}}"#,
        pid,
        repo_path.to_string_lossy()
    );
    std::fs::write(sessions_dir.join(format!("{}.json", pid)), &session_json).unwrap();

    let projects_dir = tmp.path().join("projects");
    std::fs::create_dir_all(&projects_dir).unwrap();

    let watched = vec![repo_path.clone()];
    claude_tracking::poll_claude_sessions_with_paths(
        &conn,
        &watched,
        Some(&sessions_dir),
        Some(&projects_dir),
    );

    // Session should be in DB
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM ai_sessions WHERE session_id = 'abc-123'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);

    // Should NOT have ended_at (still running)
    let ended: Option<String> = conn
        .query_row(
            "SELECT ended_at FROM ai_sessions WHERE session_id = 'abc-123'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(ended.is_none());
}

#[test]
fn test_poll_sessions_dedup() {
    let tmp = TempDir::new().unwrap();
    let conn = setup_db(&tmp);

    let sessions_dir = tmp.path().join("sessions");
    std::fs::create_dir_all(&sessions_dir).unwrap();

    let pid = std::process::id();
    let session_json = format!(
        r#"{{"pid":{},"sessionId":"dedup-test","cwd":"/tmp/repo","startedAt":1773674448026}}"#,
        pid
    );
    std::fs::write(sessions_dir.join(format!("{}.json", pid)), &session_json).unwrap();

    let projects_dir = tmp.path().join("projects");
    std::fs::create_dir_all(&projects_dir).unwrap();

    let watched: Vec<PathBuf> = vec![];

    // Poll twice
    claude_tracking::poll_claude_sessions_with_paths(&conn, &watched, Some(&sessions_dir), Some(&projects_dir));
    claude_tracking::poll_claude_sessions_with_paths(&conn, &watched, Some(&sessions_dir), Some(&projects_dir));

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM ai_sessions WHERE session_id = 'dedup-test'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn test_poll_sessions_marks_ended_when_pid_dead() {
    let tmp = TempDir::new().unwrap();
    let conn = setup_db(&tmp);

    let sessions_dir = tmp.path().join("sessions");
    std::fs::create_dir_all(&sessions_dir).unwrap();
    let projects_dir = tmp.path().join("projects");
    std::fs::create_dir_all(&projects_dir).unwrap();

    // First: create a session file with current PID
    let pid = std::process::id();
    let session_json = format!(
        r#"{{"pid":{},"sessionId":"end-test","cwd":"/tmp/repo","startedAt":1773674448026}}"#,
        pid
    );
    std::fs::write(sessions_dir.join(format!("{}.json", pid)), &session_json).unwrap();

    let watched: Vec<PathBuf> = vec![];
    claude_tracking::poll_claude_sessions_with_paths(&conn, &watched, Some(&sessions_dir), Some(&projects_dir));

    // Now remove the session file (simulating session ended) and replace with dead PID
    std::fs::remove_file(sessions_dir.join(format!("{}.json", pid))).unwrap();
    let dead_json = r#"{"pid":999999,"sessionId":"end-test","cwd":"/tmp/repo","startedAt":1773674448026}"#;
    std::fs::write(sessions_dir.join("999999.json"), dead_json).unwrap();

    claude_tracking::poll_claude_sessions_with_paths(&conn, &watched, Some(&sessions_dir), Some(&projects_dir));

    // Should now have ended_at
    let ended: Option<String> = conn
        .query_row(
            "SELECT ended_at FROM ai_sessions WHERE session_id = 'end-test'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(ended.is_some(), "Session should be marked as ended");
}

#[test]
fn test_poll_sessions_counts_turns() {
    let tmp = TempDir::new().unwrap();
    let conn = setup_db(&tmp);

    let sessions_dir = tmp.path().join("sessions");
    std::fs::create_dir_all(&sessions_dir).unwrap();

    let cwd = "/tmp/myrepo";
    let encoded = claude_tracking::encode_project_path(cwd);
    let projects_dir = tmp.path().join("projects");
    let project_subdir = projects_dir.join(&encoded);
    std::fs::create_dir_all(&project_subdir).unwrap();

    // Write a JSONL file with 5 turns
    let jsonl_content = (0..5)
        .map(|i| format!(r#"{{"type":"turn","index":{}}}"#, i))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(project_subdir.join("turn-test.jsonl"), &jsonl_content).unwrap();

    // Create session with current PID
    let pid = std::process::id();
    let session_json = format!(
        r#"{{"pid":{},"sessionId":"turn-test","cwd":"{}","startedAt":1773674448026}}"#,
        pid, cwd
    );
    std::fs::write(sessions_dir.join(format!("{}.json", pid)), &session_json).unwrap();

    let watched: Vec<PathBuf> = vec![];
    claude_tracking::poll_claude_sessions_with_paths(&conn, &watched, Some(&sessions_dir), Some(&projects_dir));

    // Now simulate session end with dead PID
    std::fs::remove_file(sessions_dir.join(format!("{}.json", pid))).unwrap();
    let dead_json = format!(
        r#"{{"pid":999999,"sessionId":"turn-test","cwd":"{}","startedAt":1773674448026}}"#,
        cwd
    );
    std::fs::write(sessions_dir.join("999999.json"), &dead_json).unwrap();

    claude_tracking::poll_claude_sessions_with_paths(&conn, &watched, Some(&sessions_dir), Some(&projects_dir));

    let turns: Option<i64> = conn
        .query_row(
            "SELECT turns FROM ai_sessions WHERE session_id = 'turn-test'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(turns, Some(5));
}

#[test]
fn test_poll_sessions_maps_cwd_to_watched_repo() {
    let tmp = TempDir::new().unwrap();
    let conn = setup_db(&tmp);

    let sessions_dir = tmp.path().join("sessions");
    std::fs::create_dir_all(&sessions_dir).unwrap();
    let projects_dir = tmp.path().join("projects");
    std::fs::create_dir_all(&projects_dir).unwrap();

    // Session cwd is a subdirectory of a watched repo
    let repo_root = PathBuf::from("/Users/dev/myrepo");
    let session_cwd = "/Users/dev/myrepo/src/subdir";

    let pid = std::process::id();
    let session_json = format!(
        r#"{{"pid":{},"sessionId":"map-test","cwd":"{}","startedAt":1773674448026}}"#,
        pid, session_cwd
    );
    std::fs::write(sessions_dir.join(format!("{}.json", pid)), &session_json).unwrap();

    let watched = vec![repo_root.clone()];
    claude_tracking::poll_claude_sessions_with_paths(&conn, &watched, Some(&sessions_dir), Some(&projects_dir));

    let repo_path: String = conn
        .query_row(
            "SELECT repo_path FROM ai_sessions WHERE session_id = 'map-test'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(repo_path, repo_root.to_string_lossy());
}

#[test]
fn test_poll_sessions_no_sessions_dir_no_crash() {
    let tmp = TempDir::new().unwrap();
    let conn = setup_db(&tmp);

    let nonexistent = tmp.path().join("nonexistent");
    let projects_dir = tmp.path().join("projects");

    let watched: Vec<PathBuf> = vec![];
    // Should not panic
    claude_tracking::poll_claude_sessions_with_paths(&conn, &watched, Some(&nonexistent), Some(&projects_dir));
}

#[test]
fn test_poll_sessions_malformed_json_skipped() {
    let tmp = TempDir::new().unwrap();
    let conn = setup_db(&tmp);

    let sessions_dir = tmp.path().join("sessions");
    std::fs::create_dir_all(&sessions_dir).unwrap();
    let projects_dir = tmp.path().join("projects");
    std::fs::create_dir_all(&projects_dir).unwrap();

    // Write invalid JSON
    std::fs::write(sessions_dir.join("bad.json"), "not valid json").unwrap();

    // Write valid JSON alongside
    let pid = std::process::id();
    let valid_json = format!(
        r#"{{"pid":{},"sessionId":"valid-one","cwd":"/tmp/repo","startedAt":1773674448026}}"#,
        pid
    );
    std::fs::write(sessions_dir.join(format!("{}.json", pid)), &valid_json).unwrap();

    let watched: Vec<PathBuf> = vec![];
    claude_tracking::poll_claude_sessions_with_paths(&conn, &watched, Some(&sessions_dir), Some(&projects_dir));

    // Valid one should be recorded, bad one skipped
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM ai_sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
}
