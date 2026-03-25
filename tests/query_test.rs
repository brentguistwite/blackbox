use blackbox::db::{
    insert_activity, insert_ai_session, insert_review, open_db, update_session_ended,
};
use blackbox::query::{
    ActivityEvent, TimeInterval, custom_range, estimate_time_v2, median_commit_gap,
    merge_intervals, month_range, query_activity, query_presence, today_range, week_range,
    yesterday_range,
};
use chrono::{Duration, Local, TimeZone, Utc};
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

    insert_activity(
        &conn,
        "/repo/alpha",
        "commit",
        Some("main"),
        None,
        Some("aaa"),
        Some("dev"),
        Some("first"),
        ts1,
    )
    .unwrap();
    insert_activity(
        &conn,
        "/repo/alpha",
        "commit",
        Some("main"),
        None,
        Some("bbb"),
        Some("dev"),
        Some("second"),
        ts2,
    )
    .unwrap();
    insert_activity(
        &conn,
        "/repo/beta",
        "commit",
        Some("feat"),
        None,
        Some("ccc"),
        Some("dev"),
        Some("init"),
        ts3,
    )
    .unwrap();

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
fn yesterday_range_is_24h_ending_at_today_midnight() {
    let (start, end) = yesterday_range();
    // Must be exactly 24h
    assert_eq!(end - start, Duration::hours(24));
    // end should be today's midnight local (converted to UTC)
    let local_today = Local::now().date_naive();
    let today_midnight = local_today.and_hms_opt(0, 0, 0).unwrap();
    let today_midnight_utc = Local
        .from_local_datetime(&today_midnight)
        .unwrap()
        .with_timezone(&Utc);
    assert_eq!(end, today_midnight_utc);
}

#[test]
fn custom_range_parses_valid_dates() {
    let (from, to) = custom_range("2025-06-01", "2025-06-15").unwrap();
    assert!(from < to);
    // Verify the local dates are correct (avoids DST edge cases in assertions)
    let from_local = from.with_timezone(&Local).date_naive();
    let to_local = to.with_timezone(&Local).date_naive();
    assert_eq!(from_local.to_string(), "2025-06-01");
    // to is next-day midnight (exclusive end)
    assert_eq!(to_local.to_string(), "2025-06-16");
}

#[test]
fn custom_range_rejects_reversed_dates() {
    let result = custom_range("2025-03-15", "2025-03-01");
    assert!(result.is_err());
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
    insert_activity(
        &conn,
        "/repo/alpha",
        "commit",
        Some("main"),
        None,
        Some("aaa"),
        Some("dev"),
        Some("first"),
        ts,
    )
    .unwrap();
    insert_review(
        &conn,
        "/repo/alpha",
        42,
        "Add auth",
        "https://github.com/repo/pull/42",
        "APPROVED",
        "2025-01-15T11:00:00+00:00",
    )
    .unwrap();
    insert_review(
        &conn,
        "/repo/alpha",
        43,
        "Fix typo",
        "https://github.com/repo/pull/43",
        "COMMENTED",
        "2025-01-15T12:00:00+00:00",
    )
    .unwrap();

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
    insert_review(
        &conn,
        "/repo/review-only",
        10,
        "Some PR",
        "https://github.com/repo/pull/10",
        "CHANGES_REQUESTED",
        "2025-01-15T10:00:00+00:00",
    )
    .unwrap();

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

#[test]
fn query_activity_includes_ai_sessions() {
    let (conn, _tmp) = setup_db();

    let ts = "2025-01-15T10:00:00+00:00";
    insert_activity(
        &conn,
        "/repo/alpha",
        "commit",
        Some("main"),
        None,
        Some("aaa"),
        Some("dev"),
        Some("first"),
        ts,
    )
    .unwrap();
    insert_ai_session(
        &conn,
        "/repo/alpha",
        "sess-001",
        "2025-01-15T09:00:00+00:00",
    )
    .unwrap();
    update_session_ended(&conn, "sess-001", "2025-01-15T10:30:00+00:00", Some(12)).unwrap();

    let from = Utc.with_ymd_and_hms(2025, 1, 15, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 1, 15, 23, 59, 59).unwrap();
    let repos = query_activity(&conn, from, to, 120, 30).unwrap();

    let alpha = repos.iter().find(|r| r.repo_path == "/repo/alpha").unwrap();
    assert_eq!(alpha.ai_sessions.len(), 1);
    assert_eq!(alpha.ai_sessions[0].session_id, "sess-001");
    assert_eq!(alpha.ai_sessions[0].turns, Some(12));
    assert!(alpha.ai_sessions[0].ended_at.is_some());
    assert_eq!(alpha.ai_sessions[0].duration.num_minutes(), 90);
}

#[test]
fn query_activity_ai_sessions_only_repo() {
    let (conn, _tmp) = setup_db();

    insert_ai_session(
        &conn,
        "/repo/ai-only",
        "sess-100",
        "2025-01-15T08:00:00+00:00",
    )
    .unwrap();
    update_session_ended(&conn, "sess-100", "2025-01-15T09:00:00+00:00", Some(5)).unwrap();

    let from = Utc.with_ymd_and_hms(2025, 1, 15, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 1, 15, 23, 59, 59).unwrap();
    let repos = query_activity(&conn, from, to, 120, 30).unwrap();

    assert_eq!(repos.len(), 1);
    let repo = &repos[0];
    assert_eq!(repo.repo_name, "ai-only");
    assert_eq!(repo.commits, 0);
    assert_eq!(repo.ai_sessions.len(), 1);
    assert_eq!(repo.ai_sessions[0].duration.num_minutes(), 60);
}

// --- Helper for time interval tests ---

fn ts(hour: u32, min: u32) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2025, 1, 15, hour, min, 0).unwrap()
}

fn iv(start_h: u32, start_m: u32, end_h: u32, end_m: u32) -> TimeInterval {
    TimeInterval {
        start: ts(start_h, start_m),
        end: ts(end_h, end_m),
    }
}

fn make_event_at(hour: u32, min: u32) -> ActivityEvent {
    ActivityEvent {
        event_type: "commit".to_string(),
        branch: Some("main".to_string()),
        commit_hash: Some(format!("hash_{:02}{:02}", hour, min)),
        message: Some("test".to_string()),
        timestamp: ts(hour, min),
    }
}

// ===== merge_intervals tests =====

#[test]
fn merge_intervals_empty() {
    let mut ivs = vec![];
    let (merged, dur) = merge_intervals(&mut ivs);
    assert!(merged.is_empty());
    assert_eq!(dur, Duration::zero());
}

#[test]
fn merge_intervals_single() {
    let mut ivs = vec![iv(10, 0, 11, 0)];
    let (merged, dur) = merge_intervals(&mut ivs);
    assert_eq!(merged.len(), 1);
    assert_eq!(dur, Duration::minutes(60));
}

#[test]
fn merge_intervals_non_overlapping() {
    let mut ivs = vec![iv(10, 0, 11, 0), iv(12, 0, 13, 0)];
    let (merged, dur) = merge_intervals(&mut ivs);
    assert_eq!(merged.len(), 2);
    assert_eq!(dur, Duration::minutes(120));
}

#[test]
fn merge_intervals_overlapping() {
    let mut ivs = vec![iv(10, 0, 11, 0), iv(10, 30, 11, 30)];
    let (merged, dur) = merge_intervals(&mut ivs);
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0], iv(10, 0, 11, 30));
    assert_eq!(dur, Duration::minutes(90));
}

#[test]
fn merge_intervals_adjacent() {
    // end == start of next => merged (start <= end is true)
    let mut ivs = vec![iv(10, 0, 11, 0), iv(11, 0, 12, 0)];
    let (merged, dur) = merge_intervals(&mut ivs);
    assert_eq!(merged.len(), 1);
    assert_eq!(dur, Duration::minutes(120));
}

#[test]
fn merge_intervals_fully_contained() {
    let mut ivs = vec![iv(10, 0, 13, 0), iv(11, 0, 12, 0)];
    let (merged, dur) = merge_intervals(&mut ivs);
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0], iv(10, 0, 13, 0));
    assert_eq!(dur, Duration::minutes(180));
}

// ===== median_commit_gap tests =====

#[test]
fn median_gap_empty() {
    assert!(median_commit_gap(&[]).is_none());
}

#[test]
fn median_gap_single_commit() {
    let events = vec![make_event_at(10, 0)];
    assert!(median_commit_gap(&events).is_none());
}

#[test]
fn median_gap_two_commits() {
    let events = vec![make_event_at(10, 0), make_event_at(10, 20)];
    assert_eq!(median_commit_gap(&events).unwrap(), Duration::minutes(20));
}

#[test]
fn median_gap_many_commits() {
    // Gaps: 10, 10, 10, 60, 10, 10, 10 => sorted: [10,10,10,10,10,10,60] => median=10
    let events = vec![
        make_event_at(10, 0),
        make_event_at(10, 10),
        make_event_at(10, 20),
        make_event_at(10, 30),
        make_event_at(11, 30),
        make_event_at(11, 40),
        make_event_at(11, 50),
        make_event_at(12, 0),
    ];
    assert_eq!(median_commit_gap(&events).unwrap(), Duration::minutes(10));
}

#[test]
fn median_gap_ignores_non_commit_events() {
    let events = vec![
        ActivityEvent {
            event_type: "branch_switch".into(),
            branch: Some("main".into()),
            commit_hash: None,
            message: None,
            timestamp: ts(10, 0),
        },
        make_event_at(10, 10),
        make_event_at(10, 30),
    ];
    // Only 2 commits with 20 min gap; branch_switch ignored
    assert_eq!(median_commit_gap(&events).unwrap(), Duration::minutes(20));
}

// ===== estimate_time_v2 tests =====

#[test]
fn v2_fallback_matches_legacy_with_few_commits() {
    // < 2 commits => falls back to config values (gap=120, credit=30)
    // Single commit: just the credit = 30 min (no gap to measure)
    // But estimate_time_v2 creates interval [first - credit, first] = 30 min
    let events = vec![make_event_at(10, 0)];
    let (result, _) = estimate_time_v2(&events, &[], 120, 30);
    assert_eq!(result, Duration::minutes(30));
}

#[test]
fn v2_two_commits_same_session() {
    // 2 commits 30 min apart. Median=30. effective_gap=clamp(90,30,120)=90.
    // effective_credit=clamp(30,5,30)=30. Session: [9:30, 10:30] = 60 min
    let events = vec![make_event_at(10, 0), make_event_at(10, 30)];
    let (result, _) = estimate_time_v2(&events, &[], 120, 30);
    assert_eq!(result, Duration::minutes(60));
}

#[test]
fn v2_rapid_committer_tight_gap() {
    // Commits every 8 min. Median=8. effective_gap=clamp(24,30,120)=30.
    // effective_credit=clamp(8,5,30)=8.
    // Events: 10:00, 10:08, 10:16, 10:24 (all within 30 min gap)
    // Session: [9:52, 10:24] = 32 min
    let events = vec![
        make_event_at(10, 0),
        make_event_at(10, 8),
        make_event_at(10, 16),
        make_event_at(10, 24),
    ];
    let (result, _) = estimate_time_v2(&events, &[], 120, 30);
    assert_eq!(result, Duration::minutes(32));
}

#[test]
fn v2_rapid_committer_session_split() {
    // Commits every 8 min, then 35 min gap (> 30 min effective_gap), then more 8 min commits
    // Median gaps: [8,8,35,8,8] => sorted [8,8,8,8,35] => median=8
    // effective_gap=30, effective_credit=8
    // Session 1: [9:52, 10:16] = 24 min. Session 2: [10:43, 11:07] = 24 min. Total = 48 min
    let events = vec![
        make_event_at(10, 0),
        make_event_at(10, 8),
        make_event_at(10, 16),
        make_event_at(10, 51),
        make_event_at(10, 59),
        make_event_at(11, 7),
    ];
    let (result, _) = estimate_time_v2(&events, &[], 120, 30);
    assert_eq!(result, Duration::minutes(48));
}

#[test]
fn v2_slow_committer_capped_gap() {
    // Commits every 50 min. Median=50. effective_gap=clamp(150,30,120)=120.
    // effective_credit=clamp(50,5,30)=30.
    // Events at 10:00 and 10:50 (gap=50 < 120, same session)
    // Session: [9:30, 10:50] = 80 min
    let events = vec![make_event_at(10, 0), make_event_at(10, 50)];
    let (result, _) = estimate_time_v2(&events, &[], 120, 30);
    assert_eq!(result, Duration::minutes(80));
}

#[test]
fn v2_ai_session_only() {
    // No git events, just AI session: 10:00-11:30 = 90 min
    let ai = vec![iv(10, 0, 11, 30)];
    let (result, _) = estimate_time_v2(&[], &ai, 120, 30);
    assert_eq!(result, Duration::minutes(90));
}

#[test]
fn v2_ai_session_merges_with_git() {
    // AI: 9:30-10:05, Git: commits at 10:00, 10:30 (median=30, credit=30, gap=90)
    // Git session tentative: [9:30, 10:30]
    // Credit suppression: AI [9:30, 10:05] overlaps credit window [9:30, 10:00] => suppress
    // Git session becomes: [10:00, 10:30]
    // Merge AI [9:30, 10:05] + git [10:00, 10:30] => [9:30, 10:30] = 60 min
    let events = vec![make_event_at(10, 0), make_event_at(10, 30)];
    let ai = vec![iv(9, 30, 10, 5)];
    let (result, _) = estimate_time_v2(&events, &ai, 120, 30);
    assert_eq!(result, Duration::minutes(60));
}

#[test]
fn v2_ai_no_overlap_no_suppression() {
    // AI: 8:00-9:00 (well before git events)
    // Git: commits at 10:00, 10:30 (median=30, credit=30, gap=90)
    // No overlap with credit window => credit kept
    // Git: [9:30, 10:30] = 60 min. AI: [8:00, 9:00] = 60 min. Total = 120 min
    let events = vec![make_event_at(10, 0), make_event_at(10, 30)];
    let ai = vec![iv(8, 0, 9, 0)];
    let (result, _) = estimate_time_v2(&events, &ai, 120, 30);
    assert_eq!(result, Duration::minutes(120));
}

// ===== query_presence tests =====

fn insert_presence(conn: &rusqlite::Connection, repo: &str, entered: &str, left: Option<&str>) {
    if let Some(l) = left {
        conn.execute(
            "INSERT INTO directory_presence (repo_path, entered_at, left_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![repo, entered, l],
        )
        .unwrap();
    } else {
        conn.execute(
            "INSERT INTO directory_presence (repo_path, entered_at) VALUES (?1, ?2)",
            rusqlite::params![repo, entered],
        )
        .unwrap();
    }
}

#[test]
fn query_presence_basic_intervals() {
    let (conn, _tmp) = setup_db();
    insert_presence(
        &conn,
        "/repo/alpha",
        "2025-01-15T10:00:00Z",
        Some("2025-01-15T11:00:00Z"),
    );
    insert_presence(
        &conn,
        "/repo/alpha",
        "2025-01-15T12:00:00Z",
        Some("2025-01-15T13:00:00Z"),
    );

    let from = ts(0, 0);
    let to = ts(23, 59);
    let map = query_presence(&conn, from, to, 120).unwrap();

    let intervals = map.get("/repo/alpha").unwrap();
    assert_eq!(intervals.len(), 2);
    assert_eq!(intervals[0], iv(10, 0, 11, 0));
    assert_eq!(intervals[1], iv(12, 0, 13, 0));
}

#[test]
fn query_presence_null_left_at_capped() {
    let (conn, _tmp) = setup_db();
    // NULL left_at => capped at entered_at + session_gap_minutes (120 min)
    insert_presence(&conn, "/repo/alpha", "2025-01-15T10:00:00Z", None);

    let from = ts(0, 0);
    let to = ts(23, 59);
    let map = query_presence(&conn, from, to, 120).unwrap();

    let intervals = map.get("/repo/alpha").unwrap();
    assert_eq!(intervals.len(), 1);
    assert_eq!(intervals[0], iv(10, 0, 12, 0)); // 10:00 + 120min = 12:00
}

#[test]
fn query_presence_spanning_boundary_included() {
    let (conn, _tmp) = setup_db();
    // Presence started before 'from' but extends into the window
    insert_presence(
        &conn,
        "/repo/alpha",
        "2025-01-15T08:00:00Z",
        Some("2025-01-15T11:00:00Z"),
    );

    let from = ts(10, 0); // query starts at 10:00
    let to = ts(23, 59);
    let map = query_presence(&conn, from, to, 120).unwrap();

    let intervals = map.get("/repo/alpha").unwrap();
    assert_eq!(intervals.len(), 1);
    // Clipped to query window: start clamped to from
    assert_eq!(intervals[0], iv(10, 0, 11, 0));
}

#[test]
fn query_presence_clipped_to_query_window() {
    let (conn, _tmp) = setup_db();
    // Presence spans wider than the query window on both sides
    insert_presence(
        &conn,
        "/repo/alpha",
        "2025-01-15T08:00:00Z",
        Some("2025-01-15T20:00:00Z"),
    );

    let from = ts(10, 0);
    let to = ts(15, 0);
    let map = query_presence(&conn, from, to, 120).unwrap();

    let intervals = map.get("/repo/alpha").unwrap();
    assert_eq!(intervals.len(), 1);
    assert_eq!(intervals[0], iv(10, 0, 15, 0)); // clipped both sides
}

// ===== query_activity: presence no longer affects time estimation =====

#[test]
fn query_activity_presence_does_not_create_repos() {
    let (conn, _tmp) = setup_db();
    // Presence-only entries should NOT create repo entries
    insert_presence(
        &conn,
        "/repo/presence-only",
        "2025-01-15T10:00:00Z",
        Some("2025-01-15T10:10:00Z"),
    );

    let from = ts(0, 0);
    let to = ts(23, 59);
    let repos = query_activity(&conn, from, to, 120, 30).unwrap();

    let repo = repos.iter().find(|r| r.repo_path == "/repo/presence-only");
    assert!(
        repo.is_none(),
        "presence should not create standalone repos"
    );
}
