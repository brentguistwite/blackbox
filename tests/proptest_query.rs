use blackbox::query::{TimeInterval, merge_intervals};
use chrono::{Duration, TimeZone, Utc};
use proptest::prelude::*;

/// Generate a random TimeInterval within a reasonable range.
/// Uses fixed epoch for deterministic shrinking.
fn arb_interval() -> impl Strategy<Value = TimeInterval> {
    (0i64..86400, 1i64..3600).prop_map(|(offset_secs, duration_secs)| {
        let base = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
        let start = base + Duration::seconds(offset_secs);
        let end = start + Duration::seconds(duration_secs);
        TimeInterval { start, end }
    })
}

proptest! {
    #[test]
    fn merged_intervals_never_overlap(mut intervals in prop::collection::vec(arb_interval(), 0..50)) {
        let (merged, _) = merge_intervals(&mut intervals);
        for window in merged.windows(2) {
            prop_assert!(window[0].end <= window[1].start,
                "Overlap: {:?} and {:?}", window[0], window[1]);
        }
    }

    #[test]
    fn merging_never_inflates_total(mut intervals in prop::collection::vec(arb_interval(), 1..50)) {
        let input_sum: Duration = intervals.iter().map(|iv| iv.end - iv.start).fold(Duration::zero(), |a, b| a + b);
        let (_, total) = merge_intervals(&mut intervals);
        prop_assert!(total <= input_sum,
            "merged total ({}) > sum of input lengths ({})", total, input_sum);
    }

    #[test]
    fn merged_total_lte_span(mut intervals in prop::collection::vec(arb_interval(), 1..50)) {
        let (merged, total) = merge_intervals(&mut intervals);
        if let (Some(first), Some(last)) = (merged.first(), merged.last()) {
            let span = last.end - first.start;
            prop_assert!(total <= span,
                "Total {} > span {}", total, span);
        }
    }
}

#[test]
fn empty_intervals_yield_zero() {
    let (merged, total) = merge_intervals(&mut []);
    assert!(merged.is_empty());
    assert_eq!(total, Duration::zero());
}
