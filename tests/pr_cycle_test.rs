use blackbox::db::{open_db, upsert_pr_snapshot};
use blackbox::enrichment::{collect_pr_snapshots, GhCommit, GhPrDetail, GhReview, GhReviewAuthor};
use blackbox::query::query_pr_cycle_stats;
use chrono::{TimeZone, Utc};
use std::path::PathBuf;
use tempfile::NamedTempFile;

fn setup_db() -> (rusqlite::Connection, NamedTempFile) {
    let f = NamedTempFile::new().unwrap();
    let conn = open_db(f.path()).unwrap();
    (conn, f)
}

#[test]
fn pr_snapshots_table_exists_and_accepts_inserts() {
    let (conn, _tmp) = setup_db();

    conn.execute(
        "INSERT INTO pr_snapshots
         (repo_path, pr_number, title, url, state, head_ref, base_ref,
          author_login, created_at_gh, merged_at, closed_at, first_review_at,
          additions, deletions, commits, iteration_count)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
        rusqlite::params![
            "/repo/test",
            1_i64,
            "Test PR",
            "https://github.com/test/repo/pull/1",
            "MERGED",
            "feature/test",
            "main",
            "testuser",
            "2025-01-15T10:00:00Z",
            "2025-01-15T12:00:00Z",
            rusqlite::types::Null,
            "2025-01-15T10:30:00Z",
            100_i64,
            20_i64,
            5_i64,
            1_i64,
        ],
    )
    .unwrap();

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM pr_snapshots", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn pr_snapshots_unique_index_replaces_on_conflict() {
    let (conn, _tmp) = setup_db();

    // Insert initial row
    conn.execute(
        "INSERT INTO pr_snapshots
         (repo_path, pr_number, title, url, state, head_ref, base_ref)
         VALUES (?1,?2,?3,?4,?5,?6,?7)",
        rusqlite::params!["/repo/test", 1_i64, "Original", "https://url", "OPEN", "feat", "main"],
    )
    .unwrap();

    // INSERT OR REPLACE with same (repo_path, pr_number) — should replace
    conn.execute(
        "INSERT OR REPLACE INTO pr_snapshots
         (repo_path, pr_number, title, url, state, head_ref, base_ref)
         VALUES (?1,?2,?3,?4,?5,?6,?7)",
        rusqlite::params!["/repo/test", 1_i64, "Updated", "https://url", "MERGED", "feat", "main"],
    )
    .unwrap();

    let (title, state): (String, String) = conn
        .query_row(
            "SELECT title, state FROM pr_snapshots WHERE repo_path = ?1 AND pr_number = ?2",
            rusqlite::params!["/repo/test", 1_i64],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(title, "Updated");
    assert_eq!(state, "MERGED");

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM pr_snapshots", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1, "should have 1 row after replace, not 2");
}

// --- US-003: upsert_pr_snapshot tests ---

fn make_pr(number: u64, state: &str, reviews: Vec<GhReview>, merged_at: Option<&str>) -> GhPrDetail {
    GhPrDetail {
        number,
        title: format!("PR #{number}"),
        url: format!("https://github.com/test/repo/pull/{number}"),
        state: state.to_string(),
        head_ref_name: "feature/test".to_string(),
        base_ref_name: "main".to_string(),
        author: Some(GhReviewAuthor { login: "testuser".to_string() }),
        created_at: Some("2025-01-15T10:00:00Z".to_string()),
        merged_at: merged_at.map(|s| s.to_string()),
        closed_at: None,
        reviews,
        additions: Some(100),
        deletions: Some(20),
        commits: vec![GhCommit { oid: "abc123".to_string(), committed_date: "2025-01-15T09:00:00Z".to_string() }],
    }
}

#[test]
fn upsert_pr_snapshot_inserts_and_queries() {
    let (conn, _tmp) = setup_db();
    let pr = make_pr(1, "MERGED", vec![], Some("2025-01-15T12:00:00Z"));

    upsert_pr_snapshot(&conn, "/repo/test", &pr).unwrap();

    let (title, state, commits): (String, String, i64) = conn
        .query_row(
            "SELECT title, state, commits FROM pr_snapshots WHERE repo_path = ?1 AND pr_number = ?2",
            rusqlite::params!["/repo/test", 1_i64],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(title, "PR #1");
    assert_eq!(state, "MERGED");
    assert_eq!(commits, 1);
}

#[test]
fn upsert_pr_snapshot_replaces_on_same_repo_pr() {
    let (conn, _tmp) = setup_db();
    let pr1 = make_pr(1, "OPEN", vec![], None);
    upsert_pr_snapshot(&conn, "/repo/test", &pr1).unwrap();

    let pr2 = make_pr(1, "MERGED", vec![], Some("2025-01-15T14:00:00Z"));
    upsert_pr_snapshot(&conn, "/repo/test", &pr2).unwrap();

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM pr_snapshots WHERE repo_path = '/repo/test' AND pr_number = 1", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1, "should be 1 row after replace");

    let state: String = conn
        .query_row(
            "SELECT state FROM pr_snapshots WHERE repo_path = '/repo/test' AND pr_number = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(state, "MERGED");
}

#[test]
fn upsert_pr_snapshot_empty_reviews_sets_null_first_review_and_zero_iterations() {
    let (conn, _tmp) = setup_db();
    let pr = make_pr(1, "OPEN", vec![], None);
    upsert_pr_snapshot(&conn, "/repo/test", &pr).unwrap();

    let (first_review, iteration_count): (Option<String>, i64) = conn
        .query_row(
            "SELECT first_review_at, iteration_count FROM pr_snapshots WHERE repo_path = '/repo/test' AND pr_number = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert!(first_review.is_none());
    assert_eq!(iteration_count, 0);
}

#[test]
fn upsert_pr_snapshot_computes_first_review_and_iteration_count() {
    let (conn, _tmp) = setup_db();
    let reviews = vec![
        GhReview {
            author: GhReviewAuthor { login: "reviewer1".to_string() },
            state: "CHANGES_REQUESTED".to_string(),
            submitted_at: "2025-01-15T11:00:00Z".to_string(),
        },
        GhReview {
            author: GhReviewAuthor { login: "reviewer2".to_string() },
            state: "APPROVED".to_string(),
            submitted_at: "2025-01-15T12:00:00Z".to_string(),
        },
        GhReview {
            author: GhReviewAuthor { login: "reviewer1".to_string() },
            state: "CHANGES_REQUESTED".to_string(),
            submitted_at: "2025-01-15T13:00:00Z".to_string(),
        },
        GhReview {
            author: GhReviewAuthor { login: "bot".to_string() },
            state: "PENDING".to_string(),
            submitted_at: "2025-01-15T09:00:00Z".to_string(),
        },
    ];
    let pr = make_pr(1, "MERGED", reviews, Some("2025-01-15T14:00:00Z"));
    upsert_pr_snapshot(&conn, "/repo/test", &pr).unwrap();

    let (first_review, iteration_count): (Option<String>, i64) = conn
        .query_row(
            "SELECT first_review_at, iteration_count FROM pr_snapshots WHERE repo_path = '/repo/test' AND pr_number = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(first_review.as_deref(), Some("2025-01-15T11:00:00Z"));
    assert_eq!(iteration_count, 2);
}

#[test]
fn pr_snapshots_nullable_fields_accept_null() {
    let (conn, _tmp) = setup_db();

    // All optional fields as NULL
    conn.execute(
        "INSERT INTO pr_snapshots
         (repo_path, pr_number, title, url, state, head_ref, base_ref,
          author_login, created_at_gh, merged_at, closed_at, first_review_at,
          additions, deletions, commits, iteration_count)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
        rusqlite::params![
            "/repo/test",
            1_i64,
            "PR with nulls",
            "https://url",
            "OPEN",
            "feat",
            "main",
            rusqlite::types::Null,
            rusqlite::types::Null,
            rusqlite::types::Null,
            rusqlite::types::Null,
            rusqlite::types::Null,
            rusqlite::types::Null,
            rusqlite::types::Null,
            rusqlite::types::Null,
            rusqlite::types::Null,
        ],
    )
    .unwrap();

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM pr_snapshots", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

// --- US-004: collect_pr_snapshots tests ---

#[test]
fn collect_pr_snapshots_handles_empty_repo_list() {
    let (conn, _tmp) = setup_db();
    collect_pr_snapshots(&[], &conn);

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM pr_snapshots", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn collect_pr_snapshots_handles_nonexistent_repo_path() {
    let (conn, _tmp) = setup_db();
    let paths = vec![PathBuf::from("/nonexistent/repo/path")];
    // Should silently skip — no panic, no error
    collect_pr_snapshots(&paths, &conn);

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM pr_snapshots", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

// --- US-006: query_pr_cycle_stats tests ---

fn insert_pr_snapshot(
    conn: &rusqlite::Connection,
    repo: &str,
    pr_number: i64,
    title: &str,
    state: &str,
    created_at: &str,
    merged_at: Option<&str>,
    closed_at: Option<&str>,
    first_review_at: Option<&str>,
    additions: Option<i64>,
    deletions: Option<i64>,
    commits: Option<i64>,
    iteration_count: Option<i64>,
) {
    conn.execute(
        "INSERT INTO pr_snapshots
         (repo_path, pr_number, title, url, state, head_ref, base_ref,
          author_login, created_at_gh, merged_at, closed_at, first_review_at,
          additions, deletions, commits, iteration_count)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
        rusqlite::params![
            repo,
            pr_number,
            title,
            format!("https://github.com/test/repo/pull/{pr_number}"),
            state,
            "feature/test",
            "main",
            "testuser",
            created_at,
            merged_at,
            closed_at,
            first_review_at,
            additions,
            deletions,
            commits,
            iteration_count,
        ],
    )
    .unwrap();
}

#[test]
fn query_pr_cycle_stats_empty_table() {
    let (conn, _tmp) = setup_db();
    let from = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 12, 31, 23, 59, 59).unwrap();

    let stats = query_pr_cycle_stats(&conn, None, from, to).unwrap();
    assert_eq!(stats.total_prs, 0);
    assert_eq!(stats.merged_prs, 0);
    assert!(stats.median_cycle_time_hours.is_none());
    assert!(stats.median_time_to_first_review_hours.is_none());
    assert!(stats.median_pr_size_lines.is_none());
    assert!(stats.median_iteration_count.is_none());
    assert!(stats.prs.is_empty());
}

#[test]
fn query_pr_cycle_stats_single_merged_pr() {
    let (conn, _tmp) = setup_db();
    // Created at 10:00, merged at 20:00 = 10h cycle time
    // First review at 12:00 = 2h to first review
    insert_pr_snapshot(
        &conn, "/repo/test", 1, "PR #1", "MERGED",
        "2025-01-15T10:00:00Z", Some("2025-01-15T20:00:00Z"), None,
        Some("2025-01-15T12:00:00Z"),
        Some(80), Some(20), Some(5), Some(1),
    );

    let from = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 12, 31, 23, 59, 59).unwrap();
    let stats = query_pr_cycle_stats(&conn, None, from, to).unwrap();

    assert_eq!(stats.total_prs, 1);
    assert_eq!(stats.merged_prs, 1);
    assert!((stats.median_cycle_time_hours.unwrap() - 10.0).abs() < 0.01);
    assert!((stats.median_time_to_first_review_hours.unwrap() - 2.0).abs() < 0.01);
    assert!((stats.median_pr_size_lines.unwrap() - 100.0).abs() < 0.01);
    assert!((stats.median_iteration_count.unwrap() - 1.0).abs() < 0.01);
}

#[test]
fn query_pr_cycle_stats_two_merged_prs_median() {
    let (conn, _tmp) = setup_db();
    // PR1: created 10:00, merged 14:00 = 4h
    insert_pr_snapshot(
        &conn, "/repo/test", 1, "PR #1", "MERGED",
        "2025-01-15T10:00:00Z", Some("2025-01-15T14:00:00Z"), None,
        None, Some(50), Some(10), Some(3), Some(0),
    );
    // PR2: created 10:00, merged 18:00 = 8h
    insert_pr_snapshot(
        &conn, "/repo/test", 2, "PR #2", "MERGED",
        "2025-01-15T10:00:00Z", Some("2025-01-15T18:00:00Z"), None,
        None, Some(90), Some(30), Some(5), Some(2),
    );

    let from = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 12, 31, 23, 59, 59).unwrap();
    let stats = query_pr_cycle_stats(&conn, None, from, to).unwrap();

    assert_eq!(stats.total_prs, 2);
    assert_eq!(stats.merged_prs, 2);
    // median of [4, 8] = 6.0
    assert!((stats.median_cycle_time_hours.unwrap() - 6.0).abs() < 0.01);
    // no reviews → None
    assert!(stats.median_time_to_first_review_hours.is_none());
    // median size: [60, 120] = 90.0
    assert!((stats.median_pr_size_lines.unwrap() - 90.0).abs() < 0.01);
}

#[test]
fn query_pr_cycle_stats_open_pr_no_cycle_time() {
    let (conn, _tmp) = setup_db();
    insert_pr_snapshot(
        &conn, "/repo/test", 1, "Open PR", "OPEN",
        "2025-01-15T10:00:00Z", None, None,
        Some("2025-01-15T11:00:00Z"),
        Some(30), Some(10), Some(2), Some(0),
    );

    let from = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 12, 31, 23, 59, 59).unwrap();
    let stats = query_pr_cycle_stats(&conn, None, from, to).unwrap();

    assert_eq!(stats.total_prs, 1);
    assert_eq!(stats.merged_prs, 0);
    assert!(stats.median_cycle_time_hours.is_none());
    // first review exists → 1h
    assert!((stats.median_time_to_first_review_hours.unwrap() - 1.0).abs() < 0.01);
    assert!(stats.prs[0].cycle_time_hours.is_none());
}

#[test]
fn query_pr_cycle_stats_date_filter_excludes() {
    let (conn, _tmp) = setup_db();
    // PR created in January
    insert_pr_snapshot(
        &conn, "/repo/test", 1, "Jan PR", "MERGED",
        "2025-01-15T10:00:00Z", Some("2025-01-15T20:00:00Z"), None,
        None, Some(50), Some(10), Some(3), Some(0),
    );
    // PR created in March — outside filter
    insert_pr_snapshot(
        &conn, "/repo/test", 2, "Mar PR", "MERGED",
        "2025-03-15T10:00:00Z", Some("2025-03-15T20:00:00Z"), None,
        None, Some(50), Some(10), Some(3), Some(0),
    );

    let from = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 1, 31, 23, 59, 59).unwrap();
    let stats = query_pr_cycle_stats(&conn, None, from, to).unwrap();

    assert_eq!(stats.total_prs, 1);
    assert_eq!(stats.prs[0].pr_number, 1);
}

#[test]
fn query_pr_cycle_stats_repo_filter() {
    let (conn, _tmp) = setup_db();
    insert_pr_snapshot(
        &conn, "/repo/alpha", 1, "Alpha PR", "MERGED",
        "2025-01-15T10:00:00Z", Some("2025-01-15T20:00:00Z"), None,
        None, Some(50), Some(10), Some(3), Some(0),
    );
    insert_pr_snapshot(
        &conn, "/repo/beta", 2, "Beta PR", "MERGED",
        "2025-01-15T10:00:00Z", Some("2025-01-15T20:00:00Z"), None,
        None, Some(50), Some(10), Some(3), Some(0),
    );

    let from = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 12, 31, 23, 59, 59).unwrap();
    let stats = query_pr_cycle_stats(&conn, Some("/repo/alpha"), from, to).unwrap();

    assert_eq!(stats.total_prs, 1);
    assert_eq!(stats.prs[0].title, "Alpha PR");
}

#[test]
fn query_pr_cycle_stats_null_additions_deletions() {
    let (conn, _tmp) = setup_db();
    insert_pr_snapshot(
        &conn, "/repo/test", 1, "No size", "MERGED",
        "2025-01-15T10:00:00Z", Some("2025-01-15T20:00:00Z"), None,
        None, None, None, Some(3), Some(0),
    );

    let from = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 12, 31, 23, 59, 59).unwrap();
    let stats = query_pr_cycle_stats(&conn, None, from, to).unwrap();

    assert!(stats.prs[0].size_lines.is_none());
    assert!(stats.median_pr_size_lines.is_none());
}
