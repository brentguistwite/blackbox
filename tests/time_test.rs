use blackbox::query::{estimate_time, ActivityEvent};
use chrono::{Duration, TimeZone, Utc};

fn make_event(minutes_offset: i64) -> ActivityEvent {
    ActivityEvent {
        event_type: "commit".to_string(),
        branch: Some("main".to_string()),
        commit_hash: Some("abc123".to_string()),
        message: Some("test commit".to_string()),
        timestamp: Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap()
            + Duration::minutes(minutes_offset),
    }
}

#[test]
fn empty_events_returns_zero() {
    let result = estimate_time(&[], 120, 30);
    assert_eq!(result, Duration::zero());
}

#[test]
fn single_event_returns_first_commit_credit() {
    let events = vec![make_event(0)];
    let result = estimate_time(&events, 120, 30);
    assert_eq!(result, Duration::minutes(30));
}

#[test]
fn two_events_within_session_gap() {
    // t=0, t=60 with gap=120 => same session => 30 credit + 60 gap = 90
    let events = vec![make_event(0), make_event(60)];
    let result = estimate_time(&events, 120, 30);
    assert_eq!(result, Duration::minutes(90));
}

#[test]
fn three_events_cross_session() {
    // t=0, t=60, t=180 with gap=120
    // Session 1: t=0,t=60 => 30+60=90
    // Session 2: t=180 (gap from t=60 is 120, equals threshold => new session) => 30
    // Total: 120
    let events = vec![make_event(0), make_event(60), make_event(180)];
    let result = estimate_time(&events, 120, 30);
    assert_eq!(result, Duration::minutes(120));
}

#[test]
fn four_events_single_session() {
    // t=0, t=30, t=60, t=90 with gap=120
    // All within one session: 30 credit + 90 total gaps = 120
    let events = vec![make_event(0), make_event(30), make_event(60), make_event(90)];
    let result = estimate_time(&events, 120, 30);
    assert_eq!(result, Duration::minutes(120));
}
