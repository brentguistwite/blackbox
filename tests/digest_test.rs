use blackbox::query::digest_week_range;
use chrono::{Datelike, Timelike, Utc, Weekday};

#[test]
fn test_digest_week_range_last_week_monday_start() {
    // For any "now", offset=-1 should give previous full Mon-Sun week
    let (start, end) = digest_week_range(-1, Weekday::Mon);

    // start should be a Monday at midnight local (converted to UTC)
    let start_local = start.with_timezone(&chrono::Local);
    assert_eq!(start_local.weekday(), Weekday::Mon);
    assert_eq!(start_local.hour(), 0);
    assert_eq!(start_local.minute(), 0);

    // end should be exactly 7 days after start (full week boundary)
    let diff = end - start;
    assert_eq!(diff.num_days(), 7);
}

#[test]
fn test_digest_week_range_current_week_capped_at_now() {
    let before = Utc::now();
    let (start, end) = digest_week_range(0, Weekday::Mon);
    let after = Utc::now();

    // start should be this week's Monday (or earlier if today is Mon)
    let start_local = start.with_timezone(&chrono::Local);
    assert_eq!(start_local.weekday(), Weekday::Mon);

    // end should be <= now (capped), not a full week boundary
    assert!(end >= before);
    assert!(end <= after + chrono::Duration::seconds(1));

    // end should be at most 7 days from start
    assert!(end - start <= chrono::Duration::weeks(1));
}

#[test]
fn test_digest_week_range_current_week_start_is_monday_midnight() {
    let (start, _end) = digest_week_range(0, Weekday::Mon);
    let start_local = start.with_timezone(&chrono::Local);
    assert_eq!(start_local.weekday(), Weekday::Mon);
    assert_eq!(start_local.hour(), 0);
    assert_eq!(start_local.minute(), 0);
    assert_eq!(start_local.second(), 0);
}

#[test]
fn test_digest_week_range_sunday_start() {
    let (start, _end) = digest_week_range(0, Weekday::Sun);
    let start_local = start.with_timezone(&chrono::Local);
    assert_eq!(start_local.weekday(), Weekday::Sun);
    assert_eq!(start_local.hour(), 0);
}

#[test]
fn test_digest_week_range_last_week_sunday_start() {
    let (start, end) = digest_week_range(-1, Weekday::Sun);
    let start_local = start.with_timezone(&chrono::Local);
    assert_eq!(start_local.weekday(), Weekday::Sun);
    // Past week: full 7-day range
    assert_eq!((end - start).num_days(), 7);
}

#[test]
fn test_digest_week_range_large_negative_offset() {
    // offset=-52 (a year ago) should not panic
    let (start, end) = digest_week_range(-52, Weekday::Mon);
    let start_local = start.with_timezone(&chrono::Local);
    assert_eq!(start_local.weekday(), Weekday::Mon);
    assert_eq!((end - start).num_days(), 7);
}
