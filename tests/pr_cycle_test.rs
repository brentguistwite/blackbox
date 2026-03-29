use blackbox::db::{open_db, upsert_pr_snapshot};
use blackbox::enrichment::{GhCommit, GhPrDetail, GhReview, GhReviewAuthor};
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
