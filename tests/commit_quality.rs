use blackbox::commit_quality::{is_vague, score_message};

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
