use blackbox::db::{insert_activity, insert_ai_session, insert_review, open_db, update_session_ended};
use blackbox::query::{
    daily_summary_for_notification, estimate_time_v2, filter_noise_switches,
    global_estimated_time, median_commit_gap, merge_intervals, query_activity,
    query_branch_switches, query_presence, today_range, week_range, month_range, quarter_range,
    ActivityEvent, BranchSwitchEvent, RepoSummary, TimeInterval,
};
use chrono::{Datelike, Duration, Timelike, TimeZone, Utc};
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

    let (start, end) = quarter_range();
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

#[test]
fn query_activity_includes_ai_sessions() {
    let (conn, _tmp) = setup_db();

    let ts = "2025-01-15T10:00:00+00:00";
    insert_activity(&conn, "/repo/alpha", "commit", Some("main"), None, Some("aaa"), Some("dev"), Some("first"), ts).unwrap();
    insert_ai_session(&conn, "claude-code", "/repo/alpha", "sess-001", "2025-01-15T09:00:00+00:00").unwrap();
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

    insert_ai_session(&conn, "claude-code", "/repo/ai-only", "sess-100", "2025-01-15T08:00:00+00:00").unwrap();
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
    TimeInterval { start: ts(start_h, start_m), end: ts(end_h, end_m) }
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
        make_event_at(10, 0), make_event_at(10, 10), make_event_at(10, 20),
        make_event_at(10, 30), make_event_at(11, 30), make_event_at(11, 40),
        make_event_at(11, 50), make_event_at(12, 0),
    ];
    assert_eq!(median_commit_gap(&events).unwrap(), Duration::minutes(10));
}

#[test]
fn median_gap_ignores_non_commit_events() {
    let events = vec![
        ActivityEvent { event_type: "branch_switch".into(), branch: Some("main".into()),
            commit_hash: None, message: None, timestamp: ts(10, 0) },
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
    let (result, _) = estimate_time_v2(&events, &[], &[], 120, 30);
    assert_eq!(result, Duration::minutes(30));
}

#[test]
fn v2_two_commits_same_session() {
    // 2 commits 30 min apart. Median=30. effective_gap=clamp(90,30,120)=90.
    // effective_credit=clamp(30,5,30)=30. Session: [9:30, 10:30] = 60 min
    let events = vec![make_event_at(10, 0), make_event_at(10, 30)];
    let (result, _) = estimate_time_v2(&events, &[], &[], 120, 30);
    assert_eq!(result, Duration::minutes(60));
}

#[test]
fn v2_rapid_committer_tight_gap() {
    // Commits every 8 min. Median=8. effective_gap=clamp(24,30,120)=30.
    // effective_credit=clamp(8,5,30)=8.
    // Events: 10:00, 10:08, 10:16, 10:24 (all within 30 min gap)
    // Session: [9:52, 10:24] = 32 min
    let events = vec![
        make_event_at(10, 0), make_event_at(10, 8),
        make_event_at(10, 16), make_event_at(10, 24),
    ];
    let (result, _) = estimate_time_v2(&events, &[], &[], 120, 30);
    assert_eq!(result, Duration::minutes(32));
}

#[test]
fn v2_rapid_committer_session_split() {
    // Commits every 8 min, then 35 min gap (> 30 min effective_gap), then more 8 min commits
    // Median gaps: [8,8,35,8,8] => sorted [8,8,8,8,35] => median=8
    // effective_gap=30, effective_credit=8
    // Session 1: [9:52, 10:16] = 24 min. Session 2: [10:43, 11:07] = 24 min. Total = 48 min
    let events = vec![
        make_event_at(10, 0), make_event_at(10, 8), make_event_at(10, 16),
        make_event_at(10, 51), make_event_at(10, 59), make_event_at(11, 7),
    ];
    let (result, _) = estimate_time_v2(&events, &[], &[], 120, 30);
    assert_eq!(result, Duration::minutes(48));
}

#[test]
fn v2_slow_committer_capped_gap() {
    // Commits every 50 min. Median=50. effective_gap=clamp(150,30,120)=120.
    // effective_credit=clamp(50,5,30)=30.
    // Events at 10:00 and 10:50 (gap=50 < 120, same session)
    // Session: [9:30, 10:50] = 80 min
    let events = vec![make_event_at(10, 0), make_event_at(10, 50)];
    let (result, _) = estimate_time_v2(&events, &[], &[], 120, 30);
    assert_eq!(result, Duration::minutes(80));
}

#[test]
fn v2_ai_session_only() {
    // No git events, just AI session: 10:00-11:30 = 90 min
    let ai = vec![iv(10, 0, 11, 30)];
    let (result, _) = estimate_time_v2(&[], &ai, &[], 120, 30);
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
    let (result, _) = estimate_time_v2(&events, &ai, &[], 120, 30);
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
    let (result, _) = estimate_time_v2(&events, &ai, &[], 120, 30);
    assert_eq!(result, Duration::minutes(120));
}

#[test]
fn v2_presence_only_no_git_events() {
    // No git events, presence intervals: 10:00-11:00 and 12:00-12:30 = 90 min total
    let presence = vec![iv(10, 0, 11, 0), iv(12, 0, 12, 30)];
    let (result, _) = estimate_time_v2(&[], &[], &presence, 120, 30);
    assert_eq!(result, Duration::minutes(90));
}

#[test]
fn v2_presence_merges_with_git_intervals() {
    // Git: commits at 10:00, 10:30 (median=30, credit=30, gap=90)
    // Git session tentative: [9:30, 10:30]
    // Presence: [9:00, 9:45] — overlaps with git interval after merge
    // After merge: [9:00, 10:30] = 90 min
    let events = vec![make_event_at(10, 0), make_event_at(10, 30)];
    let presence = vec![iv(9, 0, 9, 45)];
    let (result, _) = estimate_time_v2(&events, &[], &presence, 120, 30);
    assert_eq!(result, Duration::minutes(90));
}

// ===== query_presence tests =====

fn insert_presence(conn: &rusqlite::Connection, repo: &str, entered: &str, left: Option<&str>) {
    if let Some(l) = left {
        conn.execute(
            "INSERT INTO directory_presence (repo_path, entered_at, left_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![repo, entered, l],
        ).unwrap();
    } else {
        conn.execute(
            "INSERT INTO directory_presence (repo_path, entered_at) VALUES (?1, ?2)",
            rusqlite::params![repo, entered],
        ).unwrap();
    }
}

#[test]
fn query_presence_basic_intervals() {
    let (conn, _tmp) = setup_db();
    insert_presence(&conn, "/repo/alpha", "2025-01-15T10:00:00Z", Some("2025-01-15T11:00:00Z"));
    insert_presence(&conn, "/repo/alpha", "2025-01-15T12:00:00Z", Some("2025-01-15T13:00:00Z"));

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
    insert_presence(&conn, "/repo/alpha", "2025-01-15T08:00:00Z", Some("2025-01-15T11:00:00Z"));

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
    insert_presence(&conn, "/repo/alpha", "2025-01-15T08:00:00Z", Some("2025-01-15T20:00:00Z"));

    let from = ts(10, 0);
    let to = ts(15, 0);
    let map = query_presence(&conn, from, to, 120).unwrap();

    let intervals = map.get("/repo/alpha").unwrap();
    assert_eq!(intervals.len(), 1);
    assert_eq!(intervals[0], iv(10, 0, 15, 0)); // clipped both sides
}

// ===== US-002: presence anchoring tests =====

#[test]
fn v2_presence_anchors_session_start_before_credit_window() {
    // Presence started 45 min before first commit, credit=30
    // Commits at 10:00, 10:30 → median=30, credit=30, gap=90
    // Tentative git session: [9:30, 10:30]
    // Presence: [9:15, 10:00] starts before tentative (9:30), overlaps credit window
    // Anchored: session start = 9:15 → [9:15, 10:30] = 75 min
    let events = vec![make_event_at(10, 0), make_event_at(10, 30)];
    let presence = vec![iv(9, 15, 10, 0)];
    let (result, intervals) = estimate_time_v2(&events, &[], &presence, 120, 30);
    assert_eq!(result, Duration::minutes(75));
    assert_eq!(intervals[0].start, ts(9, 15), "session start should be anchored to presence.start");
}

#[test]
fn v2_presence_after_first_commit_no_anchoring() {
    // Presence started after first_event → no anchoring, credit logic applies
    // Commits at 10:00, 10:30 → credit=30, tentative [9:30, 10:30]
    // Presence: [10:10, 10:40] starts after first_event (10:00)
    // Git stays [9:30, 10:30], merge with presence → [9:30, 10:40] = 70 min
    let events = vec![make_event_at(10, 0), make_event_at(10, 30)];
    let presence = vec![iv(10, 10, 10, 40)];
    let (result, intervals) = estimate_time_v2(&events, &[], &presence, 120, 30);
    assert_eq!(result, Duration::minutes(70));
    assert_eq!(intervals[0].start, ts(9, 30), "credit should be preserved, not anchored");
}

#[test]
fn v2_presence_partial_credit_overlap_still_anchors() {
    // Presence covers only part of credit window → still anchors
    // Commits at 10:00, 10:30 → credit=30, tentative [9:30, 10:30]
    // Presence: [9:20, 9:40] starts before 9:30 and overlaps [9:30, 10:00]
    // Anchored: session start = 9:20 → merged [9:20, 10:30] = 70 min
    let events = vec![make_event_at(10, 0), make_event_at(10, 30)];
    let presence = vec![iv(9, 20, 9, 40)];
    let (result, intervals) = estimate_time_v2(&events, &[], &presence, 120, 30);
    assert_eq!(result, Duration::minutes(70));
    assert_eq!(intervals[0].start, ts(9, 20), "presence should anchor even with partial overlap");
}

#[test]
fn v2_presence_anchor_prevents_ai_credit_suppression() {
    // Presence anchoring should prevent AI credit suppression from creating a gap
    // Commits at 10:00, 10:30 (credit=30, gap=90)
    // AI: [9:32, 9:58] overlaps credit window [9:30, 10:00]
    // Presence: [9:10, 9:35] starts before tentative (9:30), overlaps credit window
    // Without anchoring: AI suppresses credit → git=[10:00,10:30], gap [9:58,10:00] → 78 min
    // With anchoring: git=[9:10,10:30], suppression skipped → 80 min
    let events = vec![make_event_at(10, 0), make_event_at(10, 30)];
    let ai = vec![iv(9, 32, 9, 58)];
    let presence = vec![iv(9, 10, 9, 35)];
    let (result, _) = estimate_time_v2(&events, &ai, &presence, 120, 30);
    assert_eq!(result, Duration::minutes(80));
}

// ===== US-003: query_activity wires presence into estimate_time_v2 =====

#[test]
fn query_activity_presence_increases_estimated_time() {
    // Repo with commits and presence that started well before the first commit.
    // Presence anchoring should pull session start earlier → increased estimated_time.
    let (conn, _tmp) = setup_db();

    // Commits at 10:00, 10:30 — without presence, credit=30 → session [9:30, 10:30] = 60 min
    insert_activity(&conn, "/repo/alpha", "commit", Some("main"), None, Some("aaa"), Some("dev"), Some("first"), "2025-01-15T10:00:00Z").unwrap();
    insert_activity(&conn, "/repo/alpha", "commit", Some("main"), None, Some("bbb"), Some("dev"), Some("second"), "2025-01-15T10:30:00Z").unwrap();

    let from = ts(0, 0);
    let to = ts(23, 59);

    // Baseline: no presence
    let repos_no_presence = query_activity(&conn, from, to, 120, 30).unwrap();
    let baseline = repos_no_presence.iter().find(|r| r.repo_path == "/repo/alpha").unwrap();
    let baseline_time = baseline.estimated_time;

    // Now add presence that started at 9:00 (45 min before first commit, well before credit window)
    insert_presence(&conn, "/repo/alpha", "2025-01-15T09:00:00Z", Some("2025-01-15T10:05:00Z"));

    let repos_with_presence = query_activity(&conn, from, to, 120, 30).unwrap();
    let with_pres = repos_with_presence.iter().find(|r| r.repo_path == "/repo/alpha").unwrap();

    assert!(
        with_pres.estimated_time > baseline_time,
        "presence should increase estimated_time: got {:?} vs baseline {:?}",
        with_pres.estimated_time, baseline_time
    );
}

#[test]
fn query_activity_no_presence_passes_empty() {
    // Repo with commits but NO presence data — behavior identical to before US-003
    let (conn, _tmp) = setup_db();
    insert_activity(&conn, "/repo/alpha", "commit", Some("main"), None, Some("aaa"), Some("dev"), Some("first"), "2025-01-15T10:00:00Z").unwrap();
    insert_activity(&conn, "/repo/alpha", "commit", Some("main"), None, Some("bbb"), Some("dev"), Some("second"), "2025-01-15T10:30:00Z").unwrap();

    let from = ts(0, 0);
    let to = ts(23, 59);
    let repos = query_activity(&conn, from, to, 120, 30).unwrap();
    let alpha = repos.iter().find(|r| r.repo_path == "/repo/alpha").unwrap();

    // Without presence: median=30, credit=30, gap=90 → session [9:30, 10:30] = 60 min
    assert_eq!(alpha.estimated_time, Duration::minutes(60));
}

// ===== query_activity: presence must not create standalone repos =====

#[test]
fn query_activity_presence_does_not_create_repos() {
    let (conn, _tmp) = setup_db();
    // Presence-only entries should NOT create repo entries
    insert_presence(&conn, "/repo/presence-only", "2025-01-15T10:00:00Z", Some("2025-01-15T10:10:00Z"));

    let from = ts(0, 0);
    let to = ts(23, 59);
    let repos = query_activity(&conn, from, to, 120, 30).unwrap();

    let repo = repos.iter().find(|r| r.repo_path == "/repo/presence-only");
    assert!(repo.is_none(), "presence should not create standalone repos");
}

// ===== US-004: global_estimated_time passes presence to estimate_time_v2 =====

#[test]
fn global_estimated_time_no_double_count_overlapping_presence() {
    // Two repos with overlapping presence intervals 10:00-11:00.
    // global_estimated_time merges across repos → should count 10:00-11:00 only once = 60 min.
    let repo_a = RepoSummary {
        repo_path: "/repo/alpha".into(),
        repo_name: "alpha".into(),
        commits: 0,
        branches: vec![],
        estimated_time: Duration::minutes(60),
        events: vec![],
        pr_info: None,
        reviews: vec![],
        ai_sessions: vec![],
        presence_intervals: vec![iv(10, 0, 11, 0)],
        branch_switches: 0,
    };
    let repo_b = RepoSummary {
        repo_path: "/repo/beta".into(),
        repo_name: "beta".into(),
        commits: 0,
        branches: vec![],
        estimated_time: Duration::minutes(60),
        events: vec![],
        pr_info: None,
        reviews: vec![],
        ai_sessions: vec![],
        presence_intervals: vec![iv(10, 0, 11, 0)],
        branch_switches: 0,
    };
    let total = global_estimated_time(&[repo_a, repo_b], 120, 30);
    assert_eq!(total, Duration::minutes(60), "overlapping presence across repos should not double-count");
}

#[test]
fn query_activity_populates_presence_intervals() {
    let (conn, _tmp) = setup_db();
    insert_activity(&conn, "/repo/alpha", "commit", Some("main"), None, Some("aaa"), Some("dev"), Some("first"), "2025-01-15T10:00:00Z").unwrap();
    insert_presence(&conn, "/repo/alpha", "2025-01-15T09:00:00Z", Some("2025-01-15T10:05:00Z"));

    let from = ts(0, 0);
    let to = ts(23, 59);
    let repos = query_activity(&conn, from, to, 120, 30).unwrap();
    let alpha = repos.iter().find(|r| r.repo_path == "/repo/alpha").unwrap();

    assert!(!alpha.presence_intervals.is_empty(), "presence_intervals should be populated");
    assert_eq!(alpha.presence_intervals[0].start, ts(9, 0));
    assert_eq!(alpha.presence_intervals[0].end, ts(10, 5));
}

// ===== US-CS-02: query_branch_switches =====

#[test]
fn query_branch_switches_returns_events_ordered_by_timestamp() {
    let (conn, _tmp) = setup_db();

    // Insert two branch_switch events with source_branch populated (US-CS-01)
    insert_activity(&conn, "/repo/alpha", "branch_switch", Some("feature-a"), Some("main"), None, None, None, "2025-01-15T10:00:00Z").unwrap();
    insert_activity(&conn, "/repo/alpha", "branch_switch", Some("main"), Some("feature-a"), None, None, None, "2025-01-15T11:00:00Z").unwrap();

    let from = Utc.with_ymd_and_hms(2025, 1, 15, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 1, 15, 23, 59, 59).unwrap();

    let switches = query_branch_switches(&conn, from, to).unwrap();

    assert_eq!(switches.len(), 2);

    // First event: main -> feature-a
    assert_eq!(switches[0].repo_path, "/repo/alpha");
    assert_eq!(switches[0].from_branch, Some("main".to_string()));
    assert_eq!(switches[0].to_branch, Some("feature-a".to_string()));
    assert_eq!(switches[0].timestamp, Utc.with_ymd_and_hms(2025, 1, 15, 10, 0, 0).unwrap());

    // Second event: feature-a -> main
    assert_eq!(switches[1].from_branch, Some("feature-a".to_string()));
    assert_eq!(switches[1].to_branch, Some("main".to_string()));
    assert_eq!(switches[1].timestamp, Utc.with_ymd_and_hms(2025, 1, 15, 11, 0, 0).unwrap());

    // Verify ordering: first timestamp < second
    assert!(switches[0].timestamp < switches[1].timestamp);
}

#[test]
fn query_branch_switches_excludes_commits_and_merges() {
    let (conn, _tmp) = setup_db();

    insert_activity(&conn, "/repo/alpha", "commit", Some("main"), None, Some("aaa"), Some("dev"), Some("msg"), "2025-01-15T10:00:00Z").unwrap();
    insert_activity(&conn, "/repo/alpha", "branch_switch", Some("feat"), Some("main"), None, None, None, "2025-01-15T10:30:00Z").unwrap();
    insert_activity(&conn, "/repo/alpha", "merge", Some("main"), None, Some("bbb"), Some("dev"), Some("merge"), "2025-01-15T11:00:00Z").unwrap();

    let from = Utc.with_ymd_and_hms(2025, 1, 15, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 1, 15, 23, 59, 59).unwrap();

    let switches = query_branch_switches(&conn, from, to).unwrap();
    assert_eq!(switches.len(), 1);
    assert_eq!(switches[0].to_branch, Some("feat".to_string()));
}

#[test]
fn query_branch_switches_empty_range() {
    let (conn, _tmp) = setup_db();

    let from = Utc.with_ymd_and_hms(2025, 6, 1, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 6, 30, 23, 59, 59).unwrap();

    let switches = query_branch_switches(&conn, from, to).unwrap();
    assert!(switches.is_empty());
}

// ===== US-CS-03: filter_noise_switches =====

fn make_switch(from: Option<&str>, to: Option<&str>, hour: u32, min: u32) -> BranchSwitchEvent {
    BranchSwitchEvent {
        repo_path: "/repo/alpha".to_string(),
        from_branch: from.map(|s| s.to_string()),
        to_branch: to.map(|s| s.to_string()),
        timestamp: ts(hour, min),
    }
}

#[test]
fn filter_noise_excludes_detached_head() {
    let events = vec![
        make_switch(Some("main"), None, 10, 0),        // detached HEAD
        make_switch(Some("main"), Some("feat"), 10, 5), // real switch
    ];
    let filtered = filter_noise_switches(&events, 120);
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].to_branch, Some("feat".to_string()));
}

#[test]
fn filter_noise_excludes_same_branch_recheckout() {
    let events = vec![
        make_switch(Some("main"), Some("main"), 10, 0),   // same-branch
        make_switch(Some("main"), Some("feat"), 10, 5),    // real
    ];
    let filtered = filter_noise_switches(&events, 120);
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].to_branch, Some("feat".to_string()));
}

#[test]
fn filter_noise_collapses_round_trip_under_threshold() {
    // A->B at 10:00, B->A at 10:01 — round-trip in 60s < 120s threshold
    let events = vec![
        make_switch(Some("main"), Some("feat"), 10, 0),
        make_switch(Some("feat"), Some("main"), 10, 1),
    ];
    let filtered = filter_noise_switches(&events, 120);
    assert!(filtered.is_empty(), "round-trip under threshold should be collapsed");
}

#[test]
fn filter_noise_keeps_round_trip_over_threshold() {
    // A->B at 10:00, B->A at 10:05 — round-trip in 300s > 120s threshold
    let events = vec![
        make_switch(Some("main"), Some("feat"), 10, 0),
        make_switch(Some("feat"), Some("main"), 10, 5),
    ];
    let filtered = filter_noise_switches(&events, 120);
    assert_eq!(filtered.len(), 2, "round-trip over threshold should be kept");
}

#[test]
fn filter_noise_empty_input() {
    let filtered = filter_noise_switches(&[], 120);
    assert!(filtered.is_empty());
}

#[test]
fn filter_noise_mixed_scenario() {
    // Real switch, detached HEAD, same-branch, fast round-trip, slow round-trip
    let events = vec![
        make_switch(Some("main"), Some("feat-a"), 10, 0),       // real — kept
        make_switch(Some("feat-a"), None, 10, 2),                // detached HEAD — excluded
        make_switch(None, Some("feat-a"), 10, 3),                // back from detached — from is None, but kept after detached filter
        make_switch(Some("feat-a"), Some("feat-a"), 10, 10),     // same-branch — excluded
        make_switch(Some("feat-a"), Some("main"), 10, 15),       // fast trip start
        make_switch(Some("main"), Some("feat-a"), 10, 16),       // fast trip end (<120s) — both collapsed
        make_switch(Some("feat-a"), Some("release"), 11, 0),     // real — kept
        make_switch(Some("release"), Some("feat-a"), 11, 30),    // slow round-trip (>120s) — kept
    ];
    let filtered = filter_noise_switches(&events, 120);
    // Expected: feat-a(10:00), feat-a(10:03), release(11:00), feat-a(11:30)
    assert_eq!(filtered.len(), 4);
    assert_eq!(filtered[0].to_branch, Some("feat-a".to_string()));
    assert_eq!(filtered[1].to_branch, Some("feat-a".to_string()));
    assert_eq!(filtered[2].to_branch, Some("release".to_string()));
    assert_eq!(filtered[3].to_branch, Some("feat-a".to_string()));
}

// ===== US-CS-04: per-repo and cross-repo branch_switches aggregation =====

#[test]
fn query_activity_populates_branch_switches_per_repo() {
    let (conn, _tmp) = setup_db();

    // Repo alpha: 2 real switches (main->feat, feat->main with 5min dwell > 120s threshold)
    insert_activity(&conn, "/repo/alpha", "commit", Some("main"), None, Some("aaa"), Some("dev"), Some("first"), "2025-01-15T10:00:00Z").unwrap();
    insert_activity(&conn, "/repo/alpha", "branch_switch", Some("feat"), Some("main"), None, None, None, "2025-01-15T10:30:00Z").unwrap();
    insert_activity(&conn, "/repo/alpha", "branch_switch", Some("main"), Some("feat"), None, None, None, "2025-01-15T10:35:00Z").unwrap();

    // Repo beta: 1 real switch
    insert_activity(&conn, "/repo/beta", "commit", Some("main"), None, Some("bbb"), Some("dev"), Some("init"), "2025-01-15T11:00:00Z").unwrap();
    insert_activity(&conn, "/repo/beta", "branch_switch", Some("dev"), Some("main"), None, None, None, "2025-01-15T11:30:00Z").unwrap();

    let from = Utc.with_ymd_and_hms(2025, 1, 15, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 1, 15, 23, 59, 59).unwrap();

    let repos = query_activity(&conn, from, to, 120, 30).unwrap();

    let alpha = repos.iter().find(|r| r.repo_path == "/repo/alpha").unwrap();
    assert_eq!(alpha.branch_switches, 2, "alpha should have 2 filtered switches");

    let beta = repos.iter().find(|r| r.repo_path == "/repo/beta").unwrap();
    assert_eq!(beta.branch_switches, 1, "beta should have 1 filtered switch");
}

#[test]
fn query_activity_total_branch_switches_sums_repos() {
    let (conn, _tmp) = setup_db();

    // Repo alpha: 2 switches
    insert_activity(&conn, "/repo/alpha", "commit", Some("main"), None, Some("aaa"), Some("dev"), Some("first"), "2025-01-15T10:00:00Z").unwrap();
    insert_activity(&conn, "/repo/alpha", "branch_switch", Some("feat"), Some("main"), None, None, None, "2025-01-15T10:30:00Z").unwrap();
    insert_activity(&conn, "/repo/alpha", "branch_switch", Some("main"), Some("feat"), None, None, None, "2025-01-15T10:35:00Z").unwrap();

    // Repo beta: 1 switch
    insert_activity(&conn, "/repo/beta", "commit", Some("main"), None, Some("bbb"), Some("dev"), Some("init"), "2025-01-15T11:00:00Z").unwrap();
    insert_activity(&conn, "/repo/beta", "branch_switch", Some("dev"), Some("main"), None, None, None, "2025-01-15T11:30:00Z").unwrap();

    let from = Utc.with_ymd_and_hms(2025, 1, 15, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 1, 15, 23, 59, 59).unwrap();

    let repos = query_activity(&conn, from, to, 120, 30).unwrap();
    let total: usize = repos.iter().map(|r| r.branch_switches).sum();
    assert_eq!(total, 3, "total_branch_switches should be sum of per-repo counts");
}

#[test]
fn query_activity_branch_switches_filters_noise() {
    let (conn, _tmp) = setup_db();

    // Fast round-trip: main->feat at 10:30, feat->main at 10:31 (60s < 120s threshold)
    insert_activity(&conn, "/repo/alpha", "commit", Some("main"), None, Some("aaa"), Some("dev"), Some("first"), "2025-01-15T10:00:00Z").unwrap();
    insert_activity(&conn, "/repo/alpha", "branch_switch", Some("feat"), Some("main"), None, None, None, "2025-01-15T10:30:00Z").unwrap();
    insert_activity(&conn, "/repo/alpha", "branch_switch", Some("main"), Some("feat"), None, None, None, "2025-01-15T10:31:00Z").unwrap();

    let from = Utc.with_ymd_and_hms(2025, 1, 15, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2025, 1, 15, 23, 59, 59).unwrap();

    let repos = query_activity(&conn, from, to, 120, 30).unwrap();
    let alpha = repos.iter().find(|r| r.repo_path == "/repo/alpha").unwrap();
    assert_eq!(alpha.branch_switches, 0, "fast round-trip should be filtered as noise");
}

// ===== US-008: daily_summary_for_notification tests =====

#[test]
fn daily_summary_no_activity_returns_none() {
    let (conn, _tmp) = setup_db();
    let result = daily_summary_for_notification(&conn, 120, 30).unwrap();
    assert!(result.is_none());
}

#[test]
fn daily_summary_single_repo_three_commits() {
    let (conn, _tmp) = setup_db();
    let now = Utc::now();
    let t1 = (now - Duration::minutes(60)).to_rfc3339();
    let t2 = (now - Duration::minutes(30)).to_rfc3339();
    let t3 = now.to_rfc3339();

    insert_activity(&conn, "/repo/alpha", "commit", Some("main"), None, Some("aaa"), Some("dev"), Some("first"), &t1).unwrap();
    insert_activity(&conn, "/repo/alpha", "commit", Some("main"), None, Some("bbb"), Some("dev"), Some("second"), &t2).unwrap();
    insert_activity(&conn, "/repo/alpha", "commit", Some("main"), None, Some("ccc"), Some("dev"), Some("third"), &t3).unwrap();

    let body = daily_summary_for_notification(&conn, 120, 30).unwrap().unwrap();
    assert!(body.contains("3 commits"), "expected '3 commits' in: {body}");
    assert!(body.contains("1 repo"), "expected '1 repo' in: {body}");
}

#[test]
fn daily_summary_two_repos() {
    let (conn, _tmp) = setup_db();
    let now = Utc::now();
    let t1 = (now - Duration::minutes(30)).to_rfc3339();
    let t2 = now.to_rfc3339();

    insert_activity(&conn, "/repo/alpha", "commit", Some("main"), None, Some("aaa"), Some("dev"), Some("msg"), &t1).unwrap();
    insert_activity(&conn, "/repo/beta", "commit", Some("main"), None, Some("bbb"), Some("dev"), Some("msg"), &t2).unwrap();

    let body = daily_summary_for_notification(&conn, 120, 30).unwrap().unwrap();
    assert!(body.contains("2 repos"), "expected '2 repos' in: {body}");
}

#[test]
fn daily_summary_singular_commit_and_repo() {
    let (conn, _tmp) = setup_db();
    let t = Utc::now().to_rfc3339();

    insert_activity(&conn, "/repo/alpha", "commit", Some("main"), None, Some("aaa"), Some("dev"), Some("only"), &t).unwrap();

    let body = daily_summary_for_notification(&conn, 120, 30).unwrap().unwrap();
    assert!(body.contains("1 commit"), "expected '1 commit' in: {body}");
    assert!(!body.contains("commits"), "should be singular 'commit' not 'commits' in: {body}");
    assert!(body.contains("1 repo"), "expected '1 repo' in: {body}");
    assert!(!body.contains("repos"), "should be singular 'repo' not 'repos' in: {body}");
}

#[test]
fn daily_summary_includes_time_estimate() {
    let (conn, _tmp) = setup_db();
    let now = Utc::now();
    // Two commits 30 min apart → estimate_time_v2 produces >0 time
    let t1 = (now - Duration::minutes(30)).to_rfc3339();
    let t2 = now.to_rfc3339();

    insert_activity(&conn, "/repo/alpha", "commit", Some("main"), None, Some("aaa"), Some("dev"), Some("first"), &t1).unwrap();
    insert_activity(&conn, "/repo/alpha", "commit", Some("main"), None, Some("bbb"), Some("dev"), Some("second"), &t2).unwrap();

    let body = daily_summary_for_notification(&conn, 120, 30).unwrap().unwrap();
    assert!(body.contains('~'), "expected time estimate with '~' in: {body}");
    // Should contain either 'h' or 'm' for the time format
    assert!(body.contains('m') || body.contains('h'), "expected time format in: {body}");
}

#[test]
fn quarter_range_starts_at_quarter_boundary() {
    let (start, end) = quarter_range();
    let start_local = start.with_timezone(&chrono::Local);
    let month = start_local.month();
    // Must be Jan, Apr, Jul, or Oct
    assert!(
        [1, 4, 7, 10].contains(&month),
        "quarter start month was {month}, expected 1/4/7/10"
    );
    // Must be day 1 at midnight
    assert_eq!(start_local.day(), 1);
    assert_eq!(start_local.hour(), 0);
    assert_eq!(start_local.minute(), 0);
    assert_eq!(start_local.second(), 0);
    // End is close to now
    assert!(end <= Utc::now() + chrono::Duration::seconds(1));
}

#[test]
fn quarter_range_covers_current_month() {
    let (start, end) = quarter_range();
    let now = Utc::now();
    // Current time must fall within the range
    assert!(start <= now);
    assert!(now <= end + chrono::Duration::seconds(1));
}

#[test]
fn quarter_boundaries_are_correct_for_each_quarter() {
    // Verify the quarter-start-month mapping logic directly:
    // Q1: months 1,2,3 → start month 1
    // Q2: months 4,5,6 → start month 4
    // Q3: months 7,8,9 → start month 7
    // Q4: months 10,11,12 → start month 10
    let current_month = chrono::Local::now().month();
    let (start, _) = quarter_range();
    let start_month = start.with_timezone(&chrono::Local).month();
    let expected = match current_month {
        1..=3 => 1,
        4..=6 => 4,
        7..=9 => 7,
        _ => 10,
    };
    assert_eq!(start_month, expected, "month {current_month} should map to quarter starting {expected}");
}
