use blackbox::db::{open_db, upsert_pr_snapshot};
use blackbox::enrichment::{collect_pr_snapshots, GhCommit, GhPrDetail, GhReview, GhReviewAuthor};
use blackbox::output::{render_pr_cycle_stats, render_pr_cycle_json};
use blackbox::query::{query_pr_cycle_stats, PrCycleStats, PrMetrics};
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

    let (state, merged_at): (String, Option<String>) = conn
        .query_row(
            "SELECT state, merged_at FROM pr_snapshots WHERE repo_path = '/repo/test' AND pr_number = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(state, "MERGED");
    assert_eq!(merged_at.as_deref(), Some("2025-01-15T14:00:00Z"));
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

// --- US-015: edge case: repos with no PRs / gh not installed ---

#[test]
fn collect_pr_snapshots_real_non_github_dir_skipped_silently() {
    let (conn, _tmp) = setup_db();
    let tmpdir = tempfile::tempdir().unwrap();
    let paths = vec![tmpdir.path().to_path_buf()];
    // Real dir but not a GitHub repo — fetch_pr_details returns None, skip silently
    collect_pr_snapshots(&paths, &conn);

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM pr_snapshots", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn query_pr_cycle_stats_repo_filter_no_match_returns_empty() {
    let (conn, _tmp) = setup_db();
    // Insert data for repo-a
    insert_pr_snapshot(
        &conn, "/repo/alpha", 1, "Alpha PR", "MERGED",
        "2025-01-15T10:00:00Z", Some("2025-01-15T20:00:00Z"), None,
        None, Some(50), Some(10), Some(3), Some(0),
    );

    let from = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 12, 31, 23, 59, 59).unwrap();
    // Filter to a repo with no data
    let stats = query_pr_cycle_stats(&conn, Some("/repo/nonexistent"), from, to).unwrap();

    assert_eq!(stats.total_prs, 0);
    assert_eq!(stats.merged_prs, 0);
    assert!(stats.median_cycle_time_hours.is_none());
    assert!(stats.prs.is_empty());
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

// --- US-007: render_pr_cycle_stats tests ---

#[test]
fn render_pr_cycle_stats_empty() {
    let stats = PrCycleStats {
        prs: vec![],
        median_cycle_time_hours: None,
        median_time_to_first_review_hours: None,
        median_pr_size_lines: None,
        median_iteration_count: None,
        total_prs: 0,
        merged_prs: 0,
    };
    let output = render_pr_cycle_stats(&stats);
    assert!(output.contains("No PR data available for this period"));
}

#[test]
fn render_pr_cycle_stats_header_and_summary() {
    let stats = PrCycleStats {
        prs: vec![PrMetrics {
            pr_number: 1,
            title: "Add feature".to_string(),
            url: "https://github.com/test/repo/pull/1".to_string(),
            state: "MERGED".to_string(),
            cycle_time_hours: Some(10.0),
            time_to_first_review_hours: Some(2.0),
            size_lines: Some(120),
            iteration_count: Some(1),
            created_at: Some(Utc.with_ymd_and_hms(2025, 1, 15, 10, 0, 0).unwrap()),
            merged_at: Some(Utc.with_ymd_and_hms(2025, 1, 15, 20, 0, 0).unwrap()),
        }],
        median_cycle_time_hours: Some(10.0),
        median_time_to_first_review_hours: Some(2.0),
        median_pr_size_lines: Some(120.0),
        median_iteration_count: Some(1.0),
        total_prs: 1,
        merged_prs: 1,
    };
    let output = render_pr_cycle_stats(&stats);
    assert!(output.contains("PR Cycle Time"));
    assert!(output.contains("1 PRs opened"));
    assert!(output.contains("1 merged"));
    assert!(output.contains("10h 0m"));
    assert!(output.contains("2h 0m"));
    assert!(output.contains("120 lines"));
    assert!(output.contains("1.0"));
    assert!(output.contains("Add feature"));
    assert!(output.contains("MERGED"));
}

#[test]
fn render_pr_cycle_stats_na_for_none_medians() {
    let stats = PrCycleStats {
        prs: vec![PrMetrics {
            pr_number: 1,
            title: "Open PR".to_string(),
            url: "https://github.com/test/repo/pull/1".to_string(),
            state: "OPEN".to_string(),
            cycle_time_hours: None,
            time_to_first_review_hours: None,
            size_lines: Some(40),
            iteration_count: Some(0),
            created_at: Some(Utc.with_ymd_and_hms(2025, 1, 15, 10, 0, 0).unwrap()),
            merged_at: None,
        }],
        median_cycle_time_hours: None,
        median_time_to_first_review_hours: None,
        median_pr_size_lines: Some(40.0),
        median_iteration_count: Some(0.0),
        total_prs: 1,
        merged_prs: 0,
    };
    let output = render_pr_cycle_stats(&stats);
    // cycle time and time to first review should show n/a
    let lines: Vec<&str> = output.lines().collect();
    let cycle_line = lines.iter().find(|l| l.contains("Cycle time")).unwrap();
    assert!(cycle_line.contains("n/a"), "expected n/a for cycle time");
    let review_line = lines.iter().find(|l| l.contains("first review")).unwrap();
    assert!(review_line.contains("n/a"), "expected n/a for first review");
}

#[test]
fn render_pr_cycle_stats_sort_order_merged_open_closed() {
    let stats = PrCycleStats {
        prs: vec![
            PrMetrics {
                pr_number: 1,
                title: "Closed PR".to_string(),
                url: "https://url/1".to_string(),
                state: "CLOSED".to_string(),
                cycle_time_hours: None,
                time_to_first_review_hours: None,
                size_lines: Some(10),
                iteration_count: Some(0),
                created_at: Some(Utc.with_ymd_and_hms(2025, 1, 10, 0, 0, 0).unwrap()),
                merged_at: None,
            },
            PrMetrics {
                pr_number: 2,
                title: "Open PR".to_string(),
                url: "https://url/2".to_string(),
                state: "OPEN".to_string(),
                cycle_time_hours: None,
                time_to_first_review_hours: None,
                size_lines: Some(20),
                iteration_count: Some(0),
                created_at: Some(Utc.with_ymd_and_hms(2025, 1, 12, 0, 0, 0).unwrap()),
                merged_at: None,
            },
            PrMetrics {
                pr_number: 3,
                title: "Merged PR".to_string(),
                url: "https://url/3".to_string(),
                state: "MERGED".to_string(),
                cycle_time_hours: Some(5.0),
                time_to_first_review_hours: None,
                size_lines: Some(30),
                iteration_count: Some(0),
                created_at: Some(Utc.with_ymd_and_hms(2025, 1, 14, 0, 0, 0).unwrap()),
                merged_at: Some(Utc.with_ymd_and_hms(2025, 1, 14, 5, 0, 0).unwrap()),
            },
        ],
        median_cycle_time_hours: Some(5.0),
        median_time_to_first_review_hours: None,
        median_pr_size_lines: Some(20.0),
        median_iteration_count: Some(0.0),
        total_prs: 3,
        merged_prs: 1,
    };
    let output = render_pr_cycle_stats(&stats);
    // Find positions of PR titles in the output — merged should come first
    let merged_pos = output.find("Merged PR").expect("should contain Merged PR");
    let open_pos = output.find("Open PR").expect("should contain Open PR");
    let closed_pos = output.find("Closed PR").expect("should contain Closed PR");
    assert!(merged_pos < open_pos, "merged before open");
    assert!(open_pos < closed_pos, "open before closed");
}

#[test]
fn render_pr_cycle_stats_truncates_long_title() {
    let long_title = "A".repeat(60);
    let stats = PrCycleStats {
        prs: vec![PrMetrics {
            pr_number: 1,
            title: long_title.clone(),
            url: "https://url/1".to_string(),
            state: "MERGED".to_string(),
            cycle_time_hours: Some(1.0),
            time_to_first_review_hours: None,
            size_lines: Some(10),
            iteration_count: Some(0),
            created_at: Some(Utc.with_ymd_and_hms(2025, 1, 15, 10, 0, 0).unwrap()),
            merged_at: Some(Utc.with_ymd_and_hms(2025, 1, 15, 11, 0, 0).unwrap()),
        }],
        median_cycle_time_hours: Some(1.0),
        median_time_to_first_review_hours: None,
        median_pr_size_lines: Some(10.0),
        median_iteration_count: Some(0.0),
        total_prs: 1,
        merged_prs: 1,
    };
    let output = render_pr_cycle_stats(&stats);
    // Should not contain the full 60-char title
    assert!(!output.contains(&long_title));
    // Should contain truncated version (40 chars + ...)
    assert!(output.contains(&"A".repeat(40)));
}

// --- US-009: JSON output tests ---

#[test]
fn render_pr_cycle_json_valid_json_with_merged_pr() {
    let stats = PrCycleStats {
        prs: vec![PrMetrics {
            pr_number: 1,
            title: "Add feature".to_string(),
            url: "https://github.com/test/repo/pull/1".to_string(),
            state: "MERGED".to_string(),
            cycle_time_hours: Some(10.0),
            time_to_first_review_hours: Some(2.0),
            size_lines: Some(120),
            iteration_count: Some(1),
            created_at: Some(Utc.with_ymd_and_hms(2025, 1, 15, 10, 0, 0).unwrap()),
            merged_at: Some(Utc.with_ymd_and_hms(2025, 1, 15, 20, 0, 0).unwrap()),
        }],
        median_cycle_time_hours: Some(10.0),
        median_time_to_first_review_hours: Some(2.0),
        median_pr_size_lines: Some(120.0),
        median_iteration_count: Some(1.0),
        total_prs: 1,
        merged_prs: 1,
    };
    let json_str = render_pr_cycle_json(&stats);
    let v: serde_json::Value = serde_json::from_str(&json_str).expect("must be valid JSON");

    assert_eq!(v["total_prs"], 1);
    assert_eq!(v["merged_prs"], 1);
    assert_eq!(v["median_cycle_time_hours"], 10.0);
    assert_eq!(v["median_time_to_first_review_hours"], 2.0);
    assert_eq!(v["median_pr_size_lines"], 120.0);
    assert_eq!(v["median_iteration_count"], 1.0);

    let pr = &v["prs"][0];
    assert_eq!(pr["pr_number"], 1);
    assert_eq!(pr["title"], "Add feature");
    assert_eq!(pr["state"], "MERGED");
    assert_eq!(pr["cycle_time_hours"], 10.0);
    assert_eq!(pr["size_lines"], 120);
    assert_eq!(pr["iteration_count"], 1);

    // DateTime fields as RFC3339 strings
    let created = pr["created_at"].as_str().unwrap();
    assert!(created.contains("2025-01-15"), "created_at should be RFC3339: {created}");
    let merged = pr["merged_at"].as_str().unwrap();
    assert!(merged.contains("2025-01-15"), "merged_at should be RFC3339: {merged}");
}

#[test]
fn render_pr_cycle_json_skips_none_fields() {
    let stats = PrCycleStats {
        prs: vec![PrMetrics {
            pr_number: 2,
            title: "Open PR".to_string(),
            url: "https://github.com/test/repo/pull/2".to_string(),
            state: "OPEN".to_string(),
            cycle_time_hours: None,
            time_to_first_review_hours: None,
            size_lines: Some(40),
            iteration_count: Some(0),
            created_at: Some(Utc.with_ymd_and_hms(2025, 1, 15, 10, 0, 0).unwrap()),
            merged_at: None,
        }],
        median_cycle_time_hours: None,
        median_time_to_first_review_hours: None,
        median_pr_size_lines: Some(40.0),
        median_iteration_count: Some(0.0),
        total_prs: 1,
        merged_prs: 0,
    };
    let json_str = render_pr_cycle_json(&stats);
    let v: serde_json::Value = serde_json::from_str(&json_str).expect("must be valid JSON");

    // Top-level None medians should be absent
    assert!(v.get("median_cycle_time_hours").is_none(), "None median should be skipped");
    assert!(v.get("median_time_to_first_review_hours").is_none(), "None median should be skipped");

    // PR-level None fields should be absent
    let pr = &v["prs"][0];
    assert!(pr.get("cycle_time_hours").is_none(), "None cycle_time should be skipped");
    assert!(pr.get("time_to_first_review_hours").is_none(), "None review time should be skipped");
    assert!(pr.get("merged_at").is_none(), "None merged_at should be skipped");

    // Present fields should exist
    assert_eq!(pr["size_lines"], 40);
    assert_eq!(pr["state"], "OPEN");
}

#[test]
fn render_pr_cycle_json_empty_stats() {
    let stats = PrCycleStats {
        prs: vec![],
        median_cycle_time_hours: None,
        median_time_to_first_review_hours: None,
        median_pr_size_lines: None,
        median_iteration_count: None,
        total_prs: 0,
        merged_prs: 0,
    };
    let json_str = render_pr_cycle_json(&stats);
    let v: serde_json::Value = serde_json::from_str(&json_str).expect("must be valid JSON");

    assert_eq!(v["total_prs"], 0);
    assert_eq!(v["merged_prs"], 0);
    assert!(v["prs"].as_array().unwrap().is_empty());
}

// --- US-014: closed-without-merge edge case ---

#[test]
fn closed_without_merge_stored_with_null_merged_at() {
    let (conn, _tmp) = setup_db();
    let pr = make_pr(1, "CLOSED", vec![], None);
    upsert_pr_snapshot(&conn, "/repo/test", &pr).unwrap();

    let (state, merged_at): (String, Option<String>) = conn
        .query_row(
            "SELECT state, merged_at FROM pr_snapshots WHERE repo_path = '/repo/test' AND pr_number = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(state, "CLOSED");
    assert!(merged_at.is_none(), "closed-without-merge should have NULL merged_at");
}

#[test]
fn closed_without_merge_cycle_time_none_and_counts() {
    let (conn, _tmp) = setup_db();

    // 1 merged PR: created 10:00, merged 20:00 = 10h cycle
    insert_pr_snapshot(
        &conn, "/repo/test", 1, "Merged PR", "MERGED",
        "2025-01-15T10:00:00Z", Some("2025-01-15T20:00:00Z"), None,
        Some("2025-01-15T12:00:00Z"),
        Some(80), Some(20), Some(5), Some(1),
    );

    // 1 closed-without-merge PR: no merged_at, has closed_at
    insert_pr_snapshot(
        &conn, "/repo/test", 2, "Closed PR", "CLOSED",
        "2025-01-15T10:00:00Z", None, Some("2025-01-15T18:00:00Z"),
        None,
        Some(30), Some(10), Some(2), Some(0),
    );

    let from = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 12, 31, 23, 59, 59).unwrap();
    let stats = query_pr_cycle_stats(&conn, None, from, to).unwrap();

    assert_eq!(stats.total_prs, 2);
    assert_eq!(stats.merged_prs, 1, "closed PR should not count as merged");

    let closed_pr = stats.prs.iter().find(|p| p.state == "CLOSED").unwrap();
    assert!(closed_pr.cycle_time_hours.is_none(), "closed-without-merge should have no cycle time");

    let merged_pr = stats.prs.iter().find(|p| p.state == "MERGED").unwrap();
    assert!(merged_pr.cycle_time_hours.is_some());

    // Median cycle time should only reflect the merged PR (10h)
    assert!((stats.median_cycle_time_hours.unwrap() - 10.0).abs() < 0.01);
}

#[test]
fn closed_without_merge_render_shows_closed_state() {
    let stats = PrCycleStats {
        prs: vec![
            PrMetrics {
                pr_number: 1,
                title: "Merged PR".to_string(),
                url: "https://url/1".to_string(),
                state: "MERGED".to_string(),
                cycle_time_hours: Some(10.0),
                time_to_first_review_hours: Some(2.0),
                size_lines: Some(100),
                iteration_count: Some(1),
                created_at: Some(Utc.with_ymd_and_hms(2025, 1, 15, 10, 0, 0).unwrap()),
                merged_at: Some(Utc.with_ymd_and_hms(2025, 1, 15, 20, 0, 0).unwrap()),
            },
            PrMetrics {
                pr_number: 2,
                title: "Closed PR".to_string(),
                url: "https://url/2".to_string(),
                state: "CLOSED".to_string(),
                cycle_time_hours: None,
                time_to_first_review_hours: None,
                size_lines: Some(40),
                iteration_count: Some(0),
                created_at: Some(Utc.with_ymd_and_hms(2025, 1, 15, 10, 0, 0).unwrap()),
                merged_at: None,
            },
        ],
        median_cycle_time_hours: Some(10.0),
        median_time_to_first_review_hours: Some(2.0),
        median_pr_size_lines: Some(70.0),
        median_iteration_count: Some(0.5),
        total_prs: 2,
        merged_prs: 1,
    };
    let output = render_pr_cycle_stats(&stats);

    assert!(output.contains("2 PRs opened"));
    assert!(output.contains("1 merged"));
    assert!(output.contains("CLOSED"), "should show CLOSED state");
    assert!(!output.contains("abandoned"), "should not say abandoned");
    assert!(!output.contains("rejected"), "should not say rejected");
}
