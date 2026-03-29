use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use blackbox::ai_tracking::{AiToolDetector, ClaudeDetector, CodexDetector, CopilotDetector, CursorDetector, WindsurfDetector, processes_matching, is_any_process_running};
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

// --- US-013: Copilot detector parsing tests ---

/// Helper: create a copilot session dir with workspace.yaml and optional events.jsonl
fn create_copilot_session(
    base: &std::path::Path,
    uuid: &str,
    yaml_content: &str,
    events_lines: Option<&[&str]>,
) -> std::path::PathBuf {
    let dir = base.join(uuid);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("workspace.yaml"), yaml_content).unwrap();
    if let Some(lines) = events_lines {
        std::fs::write(dir.join("events.jsonl"), lines.join("\n")).unwrap();
    }
    dir
}

#[test]
fn test_copilot_detector_valid_session_inserts_row() {
    let (_tmp, conn) = setup_db();
    let state_tmp = TempDir::new().unwrap();
    let state_dir = state_tmp.path().to_path_buf();

    let uuid = "a1b2c3d4-e5f6-7890-abcd-ef1234567890";
    create_copilot_session(
        &state_dir,
        uuid,
        "cwd: /tmp/test-repo\n",
        Some(&[
            r#"{"type":"input"}"#,
            r#"{"type":"output"}"#,
            r#"{"type":"input"}"#,
            r#"{"type":"output"}"#,
            r#"{"type":"input"}"#,
        ]),
    );

    let detector = CopilotDetector::with_session_state_dir(state_dir);
    detector.poll(&conn, &[PathBuf::from("/tmp/test-repo")]);

    let (tool, repo, sid, turns): (String, String, String, i64) = conn
        .query_row(
            "SELECT tool, repo_path, session_id, turns FROM ai_sessions LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();

    assert_eq!(tool, "copilot-cli");
    assert_eq!(repo, "/tmp/test-repo");
    assert_eq!(sid, uuid);
    assert_eq!(turns, 5);
}

#[test]
fn test_copilot_detector_missing_cwd_skipped() {
    let (_tmp, conn) = setup_db();
    let state_tmp = TempDir::new().unwrap();
    let state_dir = state_tmp.path().to_path_buf();

    // workspace.yaml without cwd field
    create_copilot_session(
        &state_dir,
        "no-cwd-uuid",
        "some_other_field: value\n",
        Some(&[r#"{"type":"input"}"#]),
    );

    let detector = CopilotDetector::with_session_state_dir(state_dir);
    detector.poll(&conn, &[]);

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM ai_sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0, "session without cwd should be skipped");
}

#[test]
fn test_copilot_detector_no_events_jsonl_turns_none() {
    let (_tmp, conn) = setup_db();
    let state_tmp = TempDir::new().unwrap();
    let state_dir = state_tmp.path().to_path_buf();

    let uuid = "no-events-uuid-1234";
    create_copilot_session(
        &state_dir,
        uuid,
        "cwd: /tmp/test-repo\n",
        None, // no events.jsonl
    );

    let detector = CopilotDetector::with_session_state_dir(state_dir);
    detector.poll(&conn, &[PathBuf::from("/tmp/test-repo")]);

    // Row should exist but turns should be NULL
    let (tool, turns_null): (String, bool) = conn
        .query_row(
            "SELECT tool, turns IS NULL FROM ai_sessions WHERE session_id = ?1",
            [uuid],
            |r| Ok((r.get(0)?, r.get::<_, bool>(1)?)),
        )
        .unwrap();

    assert_eq!(tool, "copilot-cli");
    assert!(turns_null, "turns should be NULL when no events.jsonl exists");
}

#[test]
fn test_copilot_detector_missing_state_dir_no_error() {
    let (_tmp, conn) = setup_db();
    let nonexistent = PathBuf::from("/tmp/definitely-does-not-exist-copilot-state");

    let detector = CopilotDetector::with_session_state_dir(nonexistent);
    detector.poll(&conn, &[]); // should not panic

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM ai_sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn test_copilot_detector_tool_name() {
    let detector = CopilotDetector::default();
    assert_eq!(detector.tool_name(), "copilot-cli");
}

// --- US-014: Cursor detector parsing tests ---

/// Helper: create a cursor workspaceStorage dir with hash subdirs and workspace.json
fn create_cursor_workspace(
    base: &std::path::Path,
    hash: &str,
    json_content: &str,
) -> std::path::PathBuf {
    let dir = base.join(hash);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("workspace.json"), json_content).unwrap();
    dir
}

#[test]
fn test_cursor_detector_valid_local_folder_inserts_row() {
    let (_tmp, conn) = setup_db();
    let ws_tmp = TempDir::new().unwrap();
    let ws_dir = ws_tmp.path().to_path_buf();

    create_cursor_workspace(
        &ws_dir,
        "abc123hash",
        r#"{"folder": "/tmp/test-repo"}"#,
    );

    let detector = CursorDetector::with_workspace_dir(ws_dir);
    detector.poll(&conn, &[PathBuf::from("/tmp/test-repo")]);

    let (tool, repo, sid): (String, String, String) = conn
        .query_row(
            "SELECT tool, repo_path, session_id FROM ai_sessions LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();

    assert_eq!(tool, "cursor");
    assert_eq!(repo, "/tmp/test-repo");
    assert_eq!(sid, "cursor-abc123hash");
}

#[test]
fn test_cursor_detector_vscode_remote_uri_skipped() {
    let (_tmp, conn) = setup_db();
    let ws_tmp = TempDir::new().unwrap();
    let ws_dir = ws_tmp.path().to_path_buf();

    create_cursor_workspace(
        &ws_dir,
        "remote-hash",
        r#"{"folder": "vscode-remote://ssh-remote+myhost/path/to/project"}"#,
    );

    let detector = CursorDetector::with_workspace_dir(ws_dir);
    detector.poll(&conn, &[]);

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM ai_sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0, "vscode-remote:// URIs should be skipped");
}

#[test]
fn test_cursor_detector_missing_folder_key_skipped() {
    let (_tmp, conn) = setup_db();
    let ws_tmp = TempDir::new().unwrap();
    let ws_dir = ws_tmp.path().to_path_buf();

    create_cursor_workspace(
        &ws_dir,
        "no-folder-hash",
        r#"{"someOtherKey": "value"}"#,
    );

    let detector = CursorDetector::with_workspace_dir(ws_dir);
    detector.poll(&conn, &[]);

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM ai_sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0, "missing folder key should be skipped");
}

#[test]
fn test_cursor_detector_missing_workspace_dir_no_error() {
    let (_tmp, conn) = setup_db();
    let nonexistent = PathBuf::from("/tmp/definitely-does-not-exist-cursor-ws");

    let detector = CursorDetector::with_workspace_dir(nonexistent);
    detector.poll(&conn, &[]); // should not panic

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM ai_sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn test_cursor_detector_turns_are_none() {
    let (_tmp, conn) = setup_db();
    let ws_tmp = TempDir::new().unwrap();
    let ws_dir = ws_tmp.path().to_path_buf();

    create_cursor_workspace(
        &ws_dir,
        "turns-hash",
        r#"{"folder": "/tmp/test-repo"}"#,
    );

    let detector = CursorDetector::with_workspace_dir(ws_dir);
    detector.poll(&conn, &[PathBuf::from("/tmp/test-repo")]);

    let turns_null: bool = conn
        .query_row(
            "SELECT turns IS NULL FROM ai_sessions WHERE session_id = 'cursor-turns-hash'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(turns_null, "Cursor sessions should have NULL turns");
}

#[test]
fn test_cursor_detector_tool_name() {
    let detector = CursorDetector::default();
    assert_eq!(detector.tool_name(), "cursor");
}

// --- US-007: Windsurf detector tests ---

#[test]
fn test_windsurf_detector_tool_name() {
    let detector = WindsurfDetector::default();
    assert_eq!(detector.tool_name(), "windsurf");
}

#[test]
fn test_windsurf_detector_workspace_mode_inserts_row() {
    let (_tmp, conn) = setup_db();
    let ws_tmp = TempDir::new().unwrap();
    let ws_dir = ws_tmp.path().to_path_buf();

    // Same structure as Cursor: hash/workspace.json with folder field
    create_cursor_workspace(&ws_dir, "ws-hash-1", r#"{"folder": "/tmp/test-repo"}"#);

    let detector = WindsurfDetector::with_workspace_dir(ws_dir);
    detector.poll(&conn, &[PathBuf::from("/tmp/test-repo")]);

    let (tool, repo, sid): (String, String, String) = conn
        .query_row(
            "SELECT tool, repo_path, session_id FROM ai_sessions LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();

    assert_eq!(tool, "windsurf");
    assert_eq!(repo, "/tmp/test-repo");
    assert_eq!(sid, "windsurf-ws-hash-1");
}

#[test]
fn test_windsurf_detector_workspace_skips_remote_uri() {
    let (_tmp, conn) = setup_db();
    let ws_tmp = TempDir::new().unwrap();
    let ws_dir = ws_tmp.path().to_path_buf();

    create_cursor_workspace(
        &ws_dir,
        "remote-ws",
        r#"{"folder": "vscode-remote://ssh-remote+host/path"}"#,
    );

    let detector = WindsurfDetector::with_workspace_dir(ws_dir);
    detector.poll(&conn, &[]);

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM ai_sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0, "remote URIs should be skipped");
}

#[test]
fn test_windsurf_detector_missing_workspace_dir_no_panic() {
    let (_tmp, conn) = setup_db();
    let nonexistent = PathBuf::from("/tmp/definitely-does-not-exist-windsurf-ws");

    // Process-only fallback — no Windsurf process running, so no sessions created
    let detector = WindsurfDetector::with_workspace_dir(nonexistent);
    detector.poll(&conn, &[]);

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM ai_sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn test_windsurf_detector_turns_are_none() {
    let (_tmp, conn) = setup_db();
    let ws_tmp = TempDir::new().unwrap();
    let ws_dir = ws_tmp.path().to_path_buf();

    create_cursor_workspace(&ws_dir, "turns-ws", r#"{"folder": "/tmp/test-repo"}"#);

    let detector = WindsurfDetector::with_workspace_dir(ws_dir);
    detector.poll(&conn, &[PathBuf::from("/tmp/test-repo")]);

    let turns_null: bool = conn
        .query_row(
            "SELECT turns IS NULL FROM ai_sessions WHERE session_id = 'windsurf-turns-ws'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(turns_null, "Windsurf sessions should have NULL turns");
}

#[test]
fn test_windsurf_detector_ends_sessions_when_process_not_running() {
    let (_tmp, conn) = setup_db();

    // Pre-insert an active windsurf session
    db::insert_ai_session(&conn, "windsurf", "/tmp/repo", "windsurf-test-2026-03-29", "2026-03-29T00:00:00Z").unwrap();

    // Verify it's active
    let active = db::get_active_sessions_by_tool(&conn, "windsurf").unwrap();
    assert_eq!(active.len(), 1);

    // Poll with nonexistent workspace dir — falls to process-only mode.
    // Windsurf process not running → should mark session ended.
    let detector = WindsurfDetector::with_workspace_dir(PathBuf::from("/tmp/nonexistent"));
    detector.poll(&conn, &[]);

    let active_after = db::get_active_sessions_by_tool(&conn, "windsurf").unwrap();
    assert_eq!(active_after.len(), 0, "session should be marked ended when Windsurf not running");
}

#[test]
fn test_windsurf_detector_synthetic_session_id_format() {
    // Verify the synthetic ID generation helper
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let repo_path = "/Users/foo/projects/my-app";
    let slug = std::path::Path::new(repo_path)
        .file_name()
        .unwrap()
        .to_str()
        .unwrap();
    let session_id = format!("windsurf-{slug}-{today}");
    assert!(session_id.starts_with("windsurf-my-app-"));
    assert!(session_id.contains(&today));
}

// --- US-015: Process detection helper tests ---

#[test]
fn test_processes_matching_nonexistent_returns_empty_vec() {
    let pids = processes_matching("definitely-not-a-real-process-name-xyz");
    assert!(pids.is_empty(), "nonexistent process should return empty vec");
}

#[test]
fn test_is_any_process_running_nonexistent_returns_false() {
    assert!(
        !is_any_process_running("definitely-not-a-real-process-name-xyz"),
        "nonexistent process should return false"
    );
}

#[test]
fn test_processes_matching_empty_pattern_does_not_panic() {
    // Empty pattern may match processes or not — just must not panic
    let _pids = processes_matching("");
}
