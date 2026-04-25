use blackbox::commit_quality::{is_vague, score_message};
use chrono::Datelike;

// --- score_message tests (US-016-01, preserved) ---

#[test]
fn score_empty_is_zero() {
    assert_eq!(score_message(""), 0);
    assert_eq!(score_message("   "), 0);
}

#[test]
fn score_single_word_low() {
    // Single-word msgs score well below 50 (base 50 - 20 short - 5 lowercase = 25)
    assert!(score_message("fix") < 50);
    assert!(score_message("wip") < 50);
    assert!(score_message("update") < 50);
}

#[test]
fn score_conventional_commit_high() {
    assert!(score_message("feat(auth): add OAuth2 login flow") >= 70);
    assert!(score_message("fix: resolve null pointer in parser") >= 70);
}

#[test]
fn score_body_bonus() {
    let with_body = "feat: add login\n\nDetailed body explaining the change.";
    let without_body = "feat: add login";
    assert!(score_message(with_body) > score_message(without_body));
}

#[test]
fn score_long_subject_penalised() {
    let long_msg = "a".repeat(80);
    let normal_msg = "fix: resolve the broken authentication flow in prod";
    assert!(score_message(&long_msg) < score_message(normal_msg));
}

#[test]
fn score_short_subject_penalised() {
    assert!(score_message("oops") < 50);
}

#[test]
fn score_merge_fixed_50() {
    assert_eq!(score_message("Merge branch 'main' into feature"), 50);
    assert_eq!(score_message("Merge pull request #42"), 50);
}

#[test]
fn score_revert_fixed_50() {
    assert_eq!(score_message("Revert \"feat: add login\""), 50);
}

#[test]
fn score_deterministic() {
    let msg = "feat(core): implement caching layer";
    assert_eq!(score_message(msg), score_message(msg));
}

#[test]
fn score_all_lowercase_no_punct_penalised() {
    // "add caching" is 11 chars, all lowercase, no punctuation → -5
    let s = score_message("add caching");
    let with_cap = score_message("Add caching");
    assert!(s < with_cap);
}

// --- is_vague tests (US-016-02) ---

#[test]
fn vague_empty() {
    assert!(is_vague(""));
    assert!(is_vague("   "));
    assert!(is_vague("\n\t"));
}

#[test]
fn vague_single_word_patterns() {
    for pattern in &[
        "wip", "fix", "fixes", "fixed", "update", "updates", "updated", "misc", "stuff",
        "changes", "cleanup", "refactor", "test", "tests", "temp", "tmp", "asdf",
    ] {
        assert!(is_vague(pattern), "expected vague: {pattern}");
    }
}

#[test]
fn vague_case_insensitive() {
    assert!(is_vague("WIP"));
    assert!(is_vague("Fix"));
    assert!(is_vague("UPDATES"));
    assert!(is_vague("TMP"));
}

#[test]
fn vague_punctuation_patterns() {
    assert!(is_vague("..."));
    assert!(is_vague("."));
    assert!(is_vague("!!"));
}

#[test]
fn vague_multi_word_up_to_3() {
    assert!(is_vague("fix stuff"));
    assert!(is_vague("wip test changes"));
    assert!(is_vague("temp fix"));
}

#[test]
fn vague_trimmed() {
    assert!(is_vague("  wip  "));
    assert!(is_vague("\tfix\n"));
}

#[test]
fn not_vague_merge_commit() {
    assert!(!is_vague("Merge branch 'main' into feature"));
    assert!(!is_vague("Merge pull request #42"));
}

#[test]
fn not_vague_revert_commit() {
    // Revert scores 50, so not vague
    assert!(!is_vague("Revert \"feat: add login\""));
}

#[test]
fn not_vague_conventional_commit() {
    assert!(!is_vague("feat(auth): add OAuth2 login flow"));
    assert!(!is_vague("fix: resolve null pointer in parser"));
}

#[test]
fn not_vague_descriptive_message() {
    assert!(!is_vague("Add caching layer to reduce API latency"));
}

#[test]
fn not_vague_non_ascii_majority() {
    // Japanese text — non-ASCII majority → not vague
    assert!(!is_vague("修正バグ"));
    // Chinese
    assert!(!is_vague("更新配置"));
}

#[test]
fn not_vague_four_words_with_vague_tokens() {
    // > 3 words, even if all are vague tokens → not vague (word count gate)
    assert!(!is_vague("fix update test changes"));
}

#[test]
fn not_vague_score_above_50() {
    // A decent message that scores >= 50 should never be vague
    let msg = "Add validation for user input fields";
    assert!(score_message(msg) >= 50);
    assert!(!is_vague(msg));
}

#[test]
fn vague_clean_up_two_words() {
    assert!(is_vague("clean up"));
}

// --- DB storage tests (US-016-03) ---

#[test]
fn insert_commit_quality_stores_and_deduplicates() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = blackbox::db::open_db(&db_path).unwrap();

    // First insert succeeds
    let inserted = blackbox::db::insert_commit_quality(&conn, "/repo", "abc123", 75, false).unwrap();
    assert!(inserted);

    // Duplicate returns false
    let dup = blackbox::db::insert_commit_quality(&conn, "/repo", "abc123", 80, true).unwrap();
    assert!(!dup);

    // Original values preserved (not overwritten by dup)
    let score: i64 = conn
        .query_row(
            "SELECT score FROM commit_quality WHERE repo_path = ?1 AND commit_hash = ?2",
            rusqlite::params!["/repo", "abc123"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(score, 75);
}

#[test]
fn insert_commit_quality_vague_flag() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = blackbox::db::open_db(&db_path).unwrap();

    blackbox::db::insert_commit_quality(&conn, "/repo", "hash1", 25, true).unwrap();
    blackbox::db::insert_commit_quality(&conn, "/repo", "hash2", 80, false).unwrap();

    let vague: i64 = conn
        .query_row(
            "SELECT is_vague FROM commit_quality WHERE commit_hash = 'hash1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(vague, 1);

    let not_vague: i64 = conn
        .query_row(
            "SELECT is_vague FROM commit_quality WHERE commit_hash = 'hash2'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(not_vague, 0);
}

#[test]
fn commit_quality_row_created_after_activity_insert() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = blackbox::db::open_db(&db_path).unwrap();

    // Simulate what git_ops does: insert activity then score
    let msg = "feat: add new endpoint";
    blackbox::db::insert_activity(
        &conn, "/repo", "commit", Some("main"), None, Some("deadbeef"), Some("dev"), Some(msg),
        "2026-03-29T12:00:00+00:00",
    )
    .unwrap();

    let score = score_message(msg);
    let vague = is_vague(msg);
    blackbox::db::insert_commit_quality(&conn, "/repo", "deadbeef", score, vague).unwrap();

    // Verify quality row exists with correct score
    let stored_score: i64 = conn
        .query_row(
            "SELECT score FROM commit_quality WHERE repo_path = '/repo' AND commit_hash = 'deadbeef'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored_score, score as i64);
}

#[test]
fn migration_is_additive() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("test.db");

    // Open DB, insert activity in pre-existing table
    let conn = blackbox::db::open_db(&db_path).unwrap();
    blackbox::db::insert_activity(
        &conn, "/repo", "commit", Some("main"), None, Some("aaa"), Some("dev"), Some("test msg"),
        "2026-03-29T10:00:00+00:00",
    )
    .unwrap();
    drop(conn);

    // Reopen — migration should not destroy existing data
    let conn2 = blackbox::db::open_db(&db_path).unwrap();
    let count: i64 = conn2
        .query_row("SELECT COUNT(*) FROM git_activity", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);

    // commit_quality table should exist and be usable
    blackbox::db::insert_commit_quality(&conn2, "/repo", "aaa", 60, false).unwrap();
}

// --- Trend calculation tests (US-016-04) ---

#[test]
fn commit_quality_trend_empty_db() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = blackbox::db::open_db(&db_path).unwrap();

    let trend = blackbox::query::commit_quality_trend(&conn, 4).unwrap();
    assert_eq!(trend.len(), 4);
    assert!(trend.iter().all(|w| w.commit_count == 0));
    assert!(trend.iter().all(|w| w.avg_score == 0.0));
}

#[test]
fn commit_quality_trend_with_data() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = blackbox::db::open_db(&db_path).unwrap();

    // Insert activity + quality for current week
    let now = chrono::Utc::now();
    let ts = now.to_rfc3339();
    blackbox::db::insert_activity(
        &conn, "/repo", "commit", Some("main"), None, Some("aaa111"), Some("dev"), Some("feat: good msg"),
        &ts,
    ).unwrap();
    blackbox::db::insert_commit_quality(&conn, "/repo", "aaa111", 80, false).unwrap();

    blackbox::db::insert_activity(
        &conn, "/repo", "commit", Some("main"), None, Some("bbb222"), Some("dev"), Some("fix"),
        &ts,
    ).unwrap();
    blackbox::db::insert_commit_quality(&conn, "/repo", "bbb222", 25, true).unwrap();

    let trend = blackbox::query::commit_quality_trend(&conn, 3).unwrap();
    assert_eq!(trend.len(), 3);

    // Last week should have the data
    let last = trend.last().unwrap();
    assert_eq!(last.commit_count, 2);
    assert!((last.avg_score - 52.5).abs() < 0.1);
    assert_eq!(last.vague_count, 1);
    assert!((last.vague_pct - 50.0).abs() < 0.1);
}

// --- Revert correlation tests (US-016-05) ---

#[test]
fn find_reverted_commits_empty() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = blackbox::db::open_db(&db_path).unwrap();

    let reverts = blackbox::query::find_reverted_commits(&conn).unwrap();
    assert!(reverts.is_empty());
}

#[test]
fn find_reverted_commits_with_revert() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = blackbox::db::open_db(&db_path).unwrap();

    let ts = chrono::Utc::now().to_rfc3339();
    let orig_hash = "a".repeat(40);

    // Original commit
    blackbox::db::insert_activity(
        &conn, "/repo", "commit", Some("main"), None, Some(&orig_hash), Some("dev"),
        Some("feat: original change"), &ts,
    ).unwrap();
    blackbox::db::insert_commit_quality(&conn, "/repo", &orig_hash, 70, false).unwrap();

    // Revert commit
    let revert_msg = format!("Revert \"feat: original change\"\n\nThis reverts commit {}.", orig_hash);
    blackbox::db::insert_activity(
        &conn, "/repo", "commit", Some("main"), None, Some("bbbb"), Some("dev"),
        Some(&revert_msg), &ts,
    ).unwrap();

    let reverts = blackbox::query::find_reverted_commits(&conn).unwrap();
    assert_eq!(reverts.len(), 1);
    assert_eq!(reverts[0].original_hash, orig_hash);
    assert_eq!(reverts[0].score, Some(70));
    assert_eq!(reverts[0].original_message, "feat: original change");
}

// --- Multi-week trend test (US-016-08) ---

#[test]
fn commit_quality_trend_multi_week() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("test.db");
    let conn = blackbox::db::open_db(&db_path).unwrap();

    let now = chrono::Local::now();
    let today = now.date_naive();
    let dow = today.weekday().num_days_from_monday();
    let this_monday = today - chrono::Duration::days(dow as i64);

    // Week 0 (current): 1 commit, score 90
    let w0 = this_monday.and_hms_opt(12, 0, 0).unwrap()
        .and_local_timezone(chrono::Local).unwrap().to_rfc3339();
    blackbox::db::insert_activity(
        &conn, "/repo", "commit", Some("main"), None, Some("w0hash1"), Some("dev"),
        Some("feat(auth): add OAuth2 login flow"), &w0,
    ).unwrap();
    blackbox::db::insert_commit_quality(&conn, "/repo", "w0hash1", 90, false).unwrap();

    // Week -1: 2 commits, scores 60 and 40 → avg 50, 1 vague
    let w1 = (this_monday - chrono::Duration::days(7)).and_hms_opt(12, 0, 0).unwrap()
        .and_local_timezone(chrono::Local).unwrap().to_rfc3339();
    blackbox::db::insert_activity(
        &conn, "/repo", "commit", Some("main"), None, Some("w1hash1"), Some("dev"),
        Some("Add some config changes"), &w1,
    ).unwrap();
    blackbox::db::insert_commit_quality(&conn, "/repo", "w1hash1", 60, false).unwrap();

    blackbox::db::insert_activity(
        &conn, "/repo", "commit", Some("main"), None, Some("w1hash2"), Some("dev"),
        Some("misc updates"), &w1,
    ).unwrap();
    blackbox::db::insert_commit_quality(&conn, "/repo", "w1hash2", 40, true).unwrap();

    // Week -2: 1 commit, score 30, vague
    let w2 = (this_monday - chrono::Duration::days(14)).and_hms_opt(12, 0, 0).unwrap()
        .and_local_timezone(chrono::Local).unwrap().to_rfc3339();
    blackbox::db::insert_activity(
        &conn, "/repo", "commit", Some("main"), None, Some("w2hash1"), Some("dev"),
        Some("stuff"), &w2,
    ).unwrap();
    blackbox::db::insert_commit_quality(&conn, "/repo", "w2hash1", 30, true).unwrap();

    let trend = blackbox::query::commit_quality_trend(&conn, 4).unwrap();
    assert_eq!(trend.len(), 4);

    // Find weeks with data by commit_count > 0
    let with_data: Vec<_> = trend.iter().filter(|w| w.commit_count > 0).collect();
    assert_eq!(with_data.len(), 3, "expected data in 3 of 4 weeks");

    // Verify scores are in expected ranges
    let scores: Vec<f64> = with_data.iter().map(|w| w.avg_score).collect();
    assert!(scores.iter().any(|&s| (s - 90.0).abs() < 0.1), "should have week with avg 90");
    assert!(scores.iter().any(|&s| (s - 50.0).abs() < 0.1), "should have week with avg 50");
    assert!(scores.iter().any(|&s| (s - 30.0).abs() < 0.1), "should have week with avg 30");

    // Total vague across all weeks = 2
    let total_vague: usize = trend.iter().map(|w| w.vague_count).sum();
    assert_eq!(total_vague, 2);
}

// --- CLI integration tests (US-016-06 / US-016-08) ---

#[test]
fn cli_commit_quality_exits_0_empty_db() {
    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");
    let bb_config = config_dir.join("blackbox");
    let bb_data = data_dir.join("blackbox");
    std::fs::create_dir_all(&bb_config).unwrap();
    std::fs::create_dir_all(&bb_data).unwrap();
    std::fs::write(
        bb_config.join("config.toml"),
        "watch_dirs = [\"/tmp\"]\npoll_interval_secs = 30\n",
    ).unwrap();
    blackbox::db::open_db(&bb_data.join("blackbox.db")).unwrap();

    assert_cmd::Command::cargo_bin("blackbox")
        .unwrap()
        .arg("commit-quality")
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .assert()
        .success();
}

#[test]
fn cli_commit_quality_json_format_valid() {
    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");
    let bb_config = config_dir.join("blackbox");
    let bb_data = data_dir.join("blackbox");
    std::fs::create_dir_all(&bb_config).unwrap();
    std::fs::create_dir_all(&bb_data).unwrap();
    std::fs::write(
        bb_config.join("config.toml"),
        "watch_dirs = [\"/tmp\"]\npoll_interval_secs = 30\n",
    ).unwrap();
    blackbox::db::open_db(&bb_data.join("blackbox.db")).unwrap();

    let output = assert_cmd::Command::cargo_bin("blackbox")
        .unwrap()
        .args(["commit-quality", "--weeks", "4", "--format", "json"])
        .env("XDG_CONFIG_HOME", &config_dir)
        .env("XDG_DATA_HOME", &data_dir)
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .expect("stdout should be valid JSON");
    assert!(parsed.get("trend").is_some(), "JSON should have 'trend' key");
    assert!(parsed.get("reverts").is_some(), "JSON should have 'reverts' key");
    assert!(parsed["trend"].as_array().unwrap().len() == 4, "should have 4 weeks");
}
