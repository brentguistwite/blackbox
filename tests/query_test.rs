use blackbox::db::{insert_activity, insert_review, open_db};
use blackbox::query::{query_activity, today_range, week_range, month_range};
use chrono::{TimeZone, Utc};
use tempfile::NamedTempFile;

fn setup_db() -> (rusqlite::Connection, tempfile::NamedTempFile) {
    let tmp = NamedTempFile::new().unwrap();
    let conn = open_db(tmp.path()).unwrap();
    (conn, tmp)
}

#[test]
fn query_activity_groups_by_repo() {
    let (conn, _tmp) = setup_db();

    // Insert events for two repos
    let ts1 = "2025-01-15T10:00:00Z";
    let ts2 = "2025-01-15T10:30:00Z";
    let ts3 = "2025-01-15T11:00:00Z";

    insert_activity(&conn, "/repo/alpha", "commit", Some("main"), None, Some("aaa"), Some("dev"), Some("first"), ts1).unwrap();
    insert_activity(&conn, "/repo/alpha", "commit", Some("main"), None, Some("bbb"), Some("dev"), Some("second"), ts2).unwrap();
    insert_activity(&conn, "/repo/beta", "commit", Some("feat"), None, Some("ccc"), Some("dev"), Some("init"), ts3).unwrap();

    let from = Utc.with_ymd_and_hms(2025, 1, 15, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 1, 15, 23, 59, 59).unwrap();

    let repos = query_activity(&conn, from, to, 120, 30).unwrap();

    assert_eq!(repos.len(), 2);

    let alpha = repos.iter().find(|r| r.repo_path == "/repo/alpha").unwrap();
    assert_eq!(alpha.commits, 2);
    assert_eq!(alpha.repo_name, "alpha");
    assert_eq!(alpha.events.len(), 2);

    let beta = repos.iter().find(|r| r.repo_path == "/repo/beta").unwrap();
    assert_eq!(beta.commits, 1);
    assert_eq!(beta.repo_name, "beta");
}

#[test]
fn query_activity_empty_range_returns_empty() {
    let (conn, _tmp) = setup_db();

    let from = Utc.with_ymd_and_hms(2025, 6, 1, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 6, 30, 23, 59, 59).unwrap();

    let repos = query_activity(&conn, from, to, 120, 30).unwrap();
    assert!(repos.is_empty());
}

#[test]
fn date_ranges_are_valid() {
    let (start, end) = today_range();
    assert!(start <= end);
    assert!(end <= Utc::now() + chrono::Duration::seconds(1));

    let (start, end) = week_range();
    assert!(start <= end);

    let (start, end) = month_range();
    assert!(start <= end);
}

#[test]
fn config_session_gap_defaults() {
    let config: blackbox::config::Config = toml::from_str("").unwrap();
    assert_eq!(config.session_gap_minutes, 120);
    assert_eq!(config.first_commit_minutes, 30);
}

#[test]
fn query_activity_includes_reviews() {
    let (conn, _tmp) = setup_db();

    let ts = "2025-01-15T10:00:00+00:00";
    insert_activity(&conn, "/repo/alpha", "commit", Some("main"), None, Some("aaa"), Some("dev"), Some("first"), ts).unwrap();
    insert_review(&conn, "/repo/alpha", 42, "Add auth", "https://github.com/repo/pull/42", "APPROVED", "2025-01-15T11:00:00+00:00").unwrap();
    insert_review(&conn, "/repo/alpha", 43, "Fix typo", "https://github.com/repo/pull/43", "COMMENTED", "2025-01-15T12:00:00+00:00").unwrap();

    let from = Utc.with_ymd_and_hms(2025, 1, 15, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 1, 15, 23, 59, 59).unwrap();
    let repos = query_activity(&conn, from, to, 120, 30).unwrap();

    let alpha = repos.iter().find(|r| r.repo_path == "/repo/alpha").unwrap();
    assert_eq!(alpha.reviews.len(), 2);
    assert_eq!(alpha.reviews[0].pr_number, 42);
    assert_eq!(alpha.reviews[0].action, "APPROVED");
    assert_eq!(alpha.reviews[1].pr_number, 43);
}

#[test]
fn query_activity_reviews_only_repo() {
    let (conn, _tmp) = setup_db();

    // Repo with only reviews, no git activity
    insert_review(&conn, "/repo/review-only", 10, "Some PR", "https://github.com/repo/pull/10", "CHANGES_REQUESTED", "2025-01-15T10:00:00+00:00").unwrap();

    let from = Utc.with_ymd_and_hms(2025, 1, 15, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 1, 15, 23, 59, 59).unwrap();
    let repos = query_activity(&conn, from, to, 120, 30).unwrap();

    assert_eq!(repos.len(), 1);
    let repo = &repos[0];
    assert_eq!(repo.repo_name, "review-only");
    assert_eq!(repo.commits, 0);
    assert_eq!(repo.reviews.len(), 1);
    assert_eq!(repo.reviews[0].action, "CHANGES_REQUESTED");
}
