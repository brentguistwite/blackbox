use chrono::{DateTime, Datelike, Duration, Local, Utc};
use rusqlite::Connection;
use std::collections::BTreeSet;

use crate::config::Config;
use crate::enrichment;
use crate::query::{self, ActivitySummary};

/// Build a human-readable period label for the perf review.
/// Quarter default → "Q1 2025". Custom range → "Jan 1 – Mar 31 2025".
pub fn perf_review_period_label(from_opt: Option<&str>, to_opt: Option<&str>) -> String {
    match (from_opt, to_opt) {
        (None, None) => {
            let today = Local::now().date_naive();
            let q = match today.month() {
                1..=3 => 1,
                4..=6 => 2,
                7..=9 => 3,
                _ => 4,
            };
            format!("Q{} {}", q, today.year())
        }
        (Some(f), Some(t)) => {
            // Parse YYYY-MM-DD to format as "Jan 1 – Mar 31 2025"
            let from_date = chrono::NaiveDate::parse_from_str(f, "%Y-%m-%d");
            let to_date = chrono::NaiveDate::parse_from_str(t, "%Y-%m-%d");
            match (from_date, to_date) {
                (Ok(fd), Ok(td)) => {
                    format!(
                        "{} – {}",
                        fd.format("%b %-d %Y"),
                        td.format("%b %-d %Y"),
                    )
                }
                _ => format!("{} – {}", f, t),
            }
        }
        _ => "Custom period".to_string(),
    }
}

/// Aggregate activity for perf-review: query DB, enrich with PRs, compute totals.
/// Returns ActivitySummary ready for context building or display.
pub fn aggregate_perf_review(
    conn: &Connection,
    config: &Config,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    period_label: String,
) -> anyhow::Result<ActivitySummary> {
    let mut repos = query::query_activity(
        conn,
        from,
        to,
        config.session_gap_minutes,
        config.first_commit_minutes,
    )?;

    // Enrich with PR data — gracefully degrades if gh unavailable
    enrichment::enrich_with_prs(&mut repos);

    let total_commits: usize = repos.iter().map(|r| r.commits).sum();
    let total_reviews: usize = repos
        .iter()
        .map(|r| {
            r.reviews
                .iter()
                .map(|rv| rv.pr_number)
                .collect::<BTreeSet<_>>()
                .len()
        })
        .sum();
    let total_time =
        query::global_estimated_time(&repos, config.session_gap_minutes, config.first_commit_minutes);
    let total_ai_session_time = repos.iter().fold(Duration::zero(), |acc, r| {
        acc + r
            .ai_sessions
            .iter()
            .fold(Duration::zero(), |a, s| a + s.duration)
    });

    Ok(ActivitySummary {
        period_label,
        total_commits,
        total_reviews,
        total_repos: repos.len(),
        total_estimated_time: total_time,
        total_ai_session_time,
        streak_days: 0,
        total_branch_switches: repos.iter().map(|r| r.branch_switches).sum(),
        repos,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn period_label_quarter_default() {
        let label = perf_review_period_label(None, None);
        // Should be "Q<n> <year>"
        assert!(label.starts_with('Q'), "expected Q-prefix, got: {}", label);
        assert!(label.contains(' '), "expected space in label: {}", label);
    }

    #[test]
    fn period_label_custom_range() {
        let label = perf_review_period_label(Some("2025-01-01"), Some("2025-03-31"));
        assert!(label.contains('–'), "expected en-dash in label: {}", label);
        assert!(label.contains("Jan"), "expected Jan in label: {}", label);
        assert!(label.contains("Mar"), "expected Mar in label: {}", label);
    }

    #[test]
    fn period_label_invalid_dates_fallback() {
        let label = perf_review_period_label(Some("bad"), Some("dates"));
        assert_eq!(label, "bad – dates");
    }
}
