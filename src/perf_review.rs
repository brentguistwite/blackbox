use chrono::{DateTime, Datelike, Duration, Local, Utc};
use rusqlite::Connection;
use serde::Serialize;
use std::collections::BTreeSet;

use crate::config::Config;
use crate::enrichment;
use crate::query::{self, ActivitySummary};

// --- US-04: Theme extraction (stub — full impl in US-04) ---

/// Extract recurring themes from commit messages. Returns top-N frequent words.
/// Full implementation in US-04; this stub returns empty vec.
pub fn extract_commit_themes(_repos: &[query::RepoSummary]) -> Vec<String> {
    vec![]
}

// --- US-05: PR summary compilation (stub — full impl in US-05) ---

/// Compile deduplicated PR summary from repo data.
/// Full implementation in US-05; this stub returns empty PrSummary.
pub fn compile_pr_summary(_repos: &[query::RepoSummary]) -> PrSummary {
    PrSummary {
        authored_prs: vec![],
        reviewed_prs: vec![],
    }
}

// --- US-07: PerfReviewContext struct + builder ---

#[derive(Debug, Clone, Serialize)]
pub struct PerfReviewContext {
    pub period_label: String,
    pub total_commits: usize,
    pub total_reviews: usize,
    pub total_repos: usize,
    pub total_estimated_hours: f64,
    pub total_ai_session_hours: f64,
    pub repos: Vec<PerfReviewRepo>,
    pub themes: Vec<String>,
    pub pr_summary: PrSummary,
}

#[derive(Debug, Clone, Serialize)]
pub struct PerfReviewRepo {
    pub repo_name: String,
    pub commits: usize,
    pub branches: Vec<String>,
    pub estimated_hours: f64,
    pub recent_commit_messages: Vec<String>,
    pub authored_prs: Vec<(String, u64, String)>,
    pub reviewed_prs: Vec<(String, u64, String, String)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PrSummary {
    pub authored_prs: Vec<(String, u64, String)>,
    pub reviewed_prs: Vec<(String, u64, String, String)>,
}

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

/// Build the typed context struct that bundles all data for the LLM.
pub fn build_perf_review_context(summary: &ActivitySummary) -> PerfReviewContext {
    let themes = extract_commit_themes(&summary.repos);
    let pr_summary = compile_pr_summary(&summary.repos);

    let repos: Vec<PerfReviewRepo> = summary
        .repos
        .iter()
        .map(|r| {
            let recent_commit_messages: Vec<String> = r
                .events
                .iter()
                .filter(|e| e.event_type == "commit")
                .filter_map(|e| e.message.clone())
                .take(50)
                .collect();

            let authored_prs: Vec<(String, u64, String)> = r
                .pr_info
                .as_ref()
                .map(|prs| {
                    prs.iter()
                        .map(|p| (r.repo_name.clone(), p.number, p.title.clone()))
                        .collect()
                })
                .unwrap_or_default();

            let reviewed_prs: Vec<(String, u64, String, String)> = r
                .reviews
                .iter()
                .map(|rv| {
                    (
                        r.repo_name.clone(),
                        rv.pr_number as u64,
                        rv.pr_title.clone(),
                        rv.action.clone(),
                    )
                })
                .collect();

            PerfReviewRepo {
                repo_name: r.repo_name.clone(),
                commits: r.commits,
                branches: r.branches.clone(),
                estimated_hours: r.estimated_time.num_minutes() as f64 / 60.0,
                recent_commit_messages,
                authored_prs,
                reviewed_prs,
            }
        })
        .collect();

    PerfReviewContext {
        period_label: summary.period_label.clone(),
        total_commits: summary.total_commits,
        total_reviews: summary.total_reviews,
        total_repos: summary.total_repos,
        total_estimated_hours: summary.total_estimated_time.num_minutes() as f64 / 60.0,
        total_ai_session_hours: summary.total_ai_session_time.num_minutes() as f64 / 60.0,
        repos,
        themes,
        pr_summary,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::{ActivityEvent, AiSessionInfo, RepoSummary, ReviewInfo};
    use chrono::Duration;

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

    fn make_test_summary() -> ActivitySummary {
        let now = Utc::now();
        let events = vec![
            ActivityEvent {
                event_type: "commit".to_string(),
                branch: Some("main".to_string()),
                commit_hash: Some("abc123".to_string()),
                message: Some("feat: add auth module".to_string()),
                timestamp: now,
            },
            ActivityEvent {
                event_type: "commit".to_string(),
                branch: Some("feature/api".to_string()),
                commit_hash: Some("def456".to_string()),
                message: Some("fix: resolve timeout".to_string()),
                timestamp: now,
            },
        ];
        let reviews = vec![ReviewInfo {
            pr_number: 42,
            pr_title: "Add caching layer".to_string(),
            action: "APPROVED".to_string(),
            reviewed_at: now,
        }];
        let ai_sessions = vec![AiSessionInfo {
            tool: "claude".to_string(),
            session_id: "sess-1".to_string(),
            started_at: now - Duration::hours(1),
            ended_at: Some(now),
            duration: Duration::hours(1),
            turns: Some(10),
        }];
        let repo = RepoSummary {
            repo_path: "/home/dev/myproject".to_string(),
            repo_name: "myproject".to_string(),
            commits: 2,
            branches: vec!["main".to_string(), "feature/api".to_string()],
            estimated_time: Duration::hours(3),
            events,
            pr_info: Some(vec![crate::enrichment::PrInfo {
                number: 101,
                title: "Add auth module".to_string(),
                state: "MERGED".to_string(),
                head_ref_name: "feature/auth".to_string(),
                created_at: None,
                merged_at: None,
            }]),
            reviews,
            ai_sessions,
            presence_intervals: vec![],
            branch_switches: 1,
        };

        ActivitySummary {
            period_label: "Q1 2025".to_string(),
            total_commits: 2,
            total_reviews: 1,
            total_repos: 1,
            total_estimated_time: Duration::hours(3),
            total_ai_session_time: Duration::hours(1),
            streak_days: 5,
            total_branch_switches: 1,
            repos: vec![repo],
        }
    }

    #[test]
    fn build_context_roundtrips_through_json() {
        let summary = make_test_summary();
        let ctx = build_perf_review_context(&summary);

        // Verify fields
        assert_eq!(ctx.period_label, "Q1 2025");
        assert_eq!(ctx.total_commits, 2);
        assert_eq!(ctx.total_reviews, 1);
        assert_eq!(ctx.total_repos, 1);
        assert!((ctx.total_estimated_hours - 3.0).abs() < 0.01);
        assert!((ctx.total_ai_session_hours - 1.0).abs() < 0.01);
        assert_eq!(ctx.repos.len(), 1);
        assert_eq!(ctx.repos[0].repo_name, "myproject");
        assert_eq!(ctx.repos[0].commits, 2);
        assert_eq!(ctx.repos[0].recent_commit_messages.len(), 2);

        // Serde round-trip
        let json = serde_json::to_string(&ctx).expect("serialize");
        let deser: serde_json::Value = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deser["period_label"], "Q1 2025");
        assert_eq!(deser["total_commits"], 2);
        assert_eq!(deser["repos"][0]["repo_name"], "myproject");
    }

    #[test]
    fn build_context_caps_commit_messages_at_50() {
        let now = Utc::now();
        let events: Vec<ActivityEvent> = (0..60)
            .map(|i| ActivityEvent {
                event_type: "commit".to_string(),
                branch: Some("main".to_string()),
                commit_hash: Some(format!("hash{i}")),
                message: Some(format!("commit {i}")),
                timestamp: now,
            })
            .collect();
        let repo = RepoSummary {
            repo_path: "/repo".to_string(),
            repo_name: "repo".to_string(),
            commits: 60,
            branches: vec!["main".to_string()],
            estimated_time: Duration::hours(10),
            events,
            pr_info: None,
            reviews: vec![],
            ai_sessions: vec![],
            presence_intervals: vec![],
            branch_switches: 0,
        };
        let summary = ActivitySummary {
            period_label: "Q1 2025".to_string(),
            total_commits: 60,
            total_reviews: 0,
            total_repos: 1,
            total_estimated_time: Duration::hours(10),
            total_ai_session_time: Duration::zero(),
            streak_days: 0,
            total_branch_switches: 0,
            repos: vec![repo],
        };

        let ctx = build_perf_review_context(&summary);
        assert_eq!(ctx.repos[0].recent_commit_messages.len(), 50);
    }

    #[test]
    fn build_context_empty_repos() {
        let summary = ActivitySummary {
            period_label: "Q1 2025".to_string(),
            total_commits: 0,
            total_reviews: 0,
            total_repos: 0,
            total_estimated_time: Duration::zero(),
            total_ai_session_time: Duration::zero(),
            streak_days: 0,
            total_branch_switches: 0,
            repos: vec![],
        };

        let ctx = build_perf_review_context(&summary);
        assert_eq!(ctx.total_commits, 0);
        assert!(ctx.repos.is_empty());
        assert!(ctx.themes.is_empty());
        assert!(ctx.pr_summary.authored_prs.is_empty());
    }
}
