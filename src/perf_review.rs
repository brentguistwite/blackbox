use anyhow::bail;
use chrono::{DateTime, Datelike, Duration, Local, Utc};
use rusqlite::Connection;
use serde::Serialize;
use std::collections::{BTreeSet, HashMap};

use crate::config::Config;
use crate::enrichment;
use crate::llm::{LlmConfig, call_llm_streaming};
use crate::query::{self, ActivitySummary};

// --- US-04: Theme extraction ---

const STOP_WORDS: &[&str] = &[
    "fix", "the", "a", "an", "add", "added", "update", "updated", "chore", "feat", "feature",
    "refactor", "test", "tests", "wip", "merge", "merged", "bump", "release", "minor", "major",
    "revert", "pr", "co", "authored", "by", "for", "in", "of", "on", "to", "use", "used",
    "using", "remove", "removed", "change", "changes", "clean", "cleanup", "typo", "misc",
];

/// Extract recurring themes from commit messages. Returns top-20 frequent words
/// formatted as "word (N occurrences)", sorted descending by count.
/// Returns empty vec if total commits < 5.
pub fn extract_commit_themes(repos: &[query::RepoSummary]) -> Vec<String> {
    let total_commits: usize = repos.iter().map(|r| r.commits).sum();
    if total_commits < 5 {
        return vec![];
    }

    let stop_set: std::collections::HashSet<&str> = STOP_WORDS.iter().copied().collect();
    let mut freq: HashMap<String, usize> = HashMap::new();

    for repo in repos {
        for event in &repo.events {
            if event.event_type != "commit" {
                continue;
            }
            let Some(ref msg) = event.message else {
                continue;
            };
            let lower = msg.to_lowercase();
            for word in lower.split(|c: char| !c.is_alphanumeric()) {
                if word.len() < 3 || stop_set.contains(word) {
                    continue;
                }
                *freq.entry(word.to_string()).or_default() += 1;
            }
        }
    }

    let mut sorted: Vec<(String, usize)> = freq.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    sorted.truncate(20);

    sorted
        .into_iter()
        .map(|(word, count)| format!("{word} ({count} occurrences)"))
        .collect()
}

// --- US-05: PR summary compilation ---

fn review_action_priority(action: &str) -> u8 {
    match action {
        "APPROVED" => 3,
        "CHANGES_REQUESTED" => 2,
        "COMMENTED" => 1,
        _ => 0,
    }
}

/// Compile deduplicated PR summary from repo data.
/// Authored PRs from `pr_info`, reviewed PRs from `reviews`.
/// Deduplicates reviewed PRs by (repo, pr_number), keeping highest-priority action.
pub fn compile_pr_summary(repos: &[query::RepoSummary]) -> PrSummary {
    let mut authored_prs = Vec::new();
    let mut reviewed_prs = Vec::new();

    for repo in repos {
        // Authored PRs from pr_info
        if let Some(ref prs) = repo.pr_info {
            for pr in prs {
                authored_prs.push((repo.repo_name.clone(), pr.number, pr.title.clone()));
            }
        }

        // Reviewed PRs — dedup by pr_number within this repo
        let mut best: HashMap<i64, (String, String)> = HashMap::new();
        for rv in &repo.reviews {
            let entry = best.entry(rv.pr_number).or_insert_with(|| {
                (rv.pr_title.clone(), rv.action.clone())
            });
            if review_action_priority(&rv.action) > review_action_priority(&entry.1) {
                entry.1 = rv.action.clone();
            }
        }
        let mut repo_reviews: Vec<(i64, String, String)> = best
            .into_iter()
            .map(|(num, (title, action))| (num, title, action))
            .collect();
        repo_reviews.sort_by_key(|(num, _, _)| *num);
        for (num, title, action) in repo_reviews {
            reviewed_prs.push((repo.repo_name.clone(), num as u64, title, action));
        }
    }

    PrSummary {
        authored_prs,
        reviewed_prs,
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

// --- US-06: LLM prompt + generate fn ---

pub const PERF_REVIEW_SYSTEM_PROMPT: &str = "\
You are helping a software developer write a performance review self-assessment \
based on their recorded git activity data. Write in first person, professional tone. \
Produce approximately 400-600 words in markdown format with these sections:\n\n\
## Summary\n\
A brief overview of the review period, total output, and key areas of focus.\n\n\
## Key Contributions\n\
The most impactful work items, referencing specific repositories, features, and PRs \
where available. Do NOT invent accomplishments not evidenced in the data.\n\n\
## Technical Themes\n\
Recurring technical areas and domains based on commit message analysis and repository patterns.\n\n\
## Collaboration & Code Review\n\
Cross-team collaboration evidenced by PR reviews, authored PRs, and multi-repo work. \
If PR data is unavailable, note this and skip detailed PR references.\n\n\
## Time Investment\n\
Total estimated hours, distribution across repositories, and AI-assisted development time \
where applicable.\n\n\
Ground every claim in the provided data. Do not fabricate accomplishments or metrics \
not present in the input. If the data is sparse, acknowledge the limited window rather \
than speculating.";

/// Stream an LLM-generated performance review self-assessment to stdout.
///
/// Serializes `PerfReviewContext` to JSON, applies token budget guards
/// (truncating commit messages if serialized context > 40,000 chars),
/// and streams the result via the configured LLM provider.
pub fn generate_perf_review(config: &LlmConfig, context: &PerfReviewContext) -> anyhow::Result<()> {
    // Zero activity guard — bail before any LLM call
    if context.total_commits == 0 && context.total_reviews == 0 {
        bail!("No activity found for this period. Is the daemon running?");
    }

    let mut truncated = false;
    let context_json = {
        let json = serde_json::to_string_pretty(context)?;
        if json.len() > 40_000 {
            let mut ctx = context.clone();
            let total_msgs: usize = ctx
                .repos
                .iter()
                .map(|r| r.recent_commit_messages.len())
                .sum();
            if total_msgs > 200 {
                for repo in &mut ctx.repos {
                    let keep = (200 * repo.recent_commit_messages.len()) / total_msgs.max(1);
                    let keep = keep.max(1);
                    let start = repo.recent_commit_messages.len().saturating_sub(keep);
                    repo.recent_commit_messages = repo.recent_commit_messages[start..].to_vec();
                }
            }
            truncated = true;
            serde_json::to_string_pretty(&ctx)?
        } else {
            json
        }
    };

    let mut user_message = String::new();

    // Sparse data warning (US-09)
    if context.total_commits < 10 {
        user_message.push_str(&format!(
            "Note: limited data — only {} commits recorded. \
             User may have started tracking recently.\n\n",
            context.total_commits
        ));
    }

    if truncated {
        user_message.push_str(
            "Note: commit message list was truncated to fit token budget. \
             Not all commits are shown.\n\n",
        );
    }

    user_message
        .push_str("Generate a performance review self-assessment for this developer's activity:\n\n");
    user_message.push_str(&context_json);

    call_llm_streaming(
        config,
        PERF_REVIEW_SYSTEM_PROMPT,
        &user_message,
        2048,
        120,
    )
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
            last_active_at: None,
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

    // --- US-04: Theme extraction tests ---

    #[test]
    fn themes_empty_input() {
        let themes = extract_commit_themes(&[]);
        assert!(themes.is_empty());
    }

    #[test]
    fn themes_below_threshold_returns_empty() {
        // < 5 commits → empty vec
        let repo = RepoSummary {
            repo_path: "/r".to_string(),
            repo_name: "r".to_string(),
            commits: 3,
            branches: vec![],
            estimated_time: Duration::zero(),
            events: vec![
                ActivityEvent {
                    event_type: "commit".to_string(),
                    branch: None,
                    commit_hash: None,
                    message: Some("implement auth module".to_string()),
                    timestamp: Utc::now(),
                },
                ActivityEvent {
                    event_type: "commit".to_string(),
                    branch: None,
                    commit_hash: None,
                    message: Some("implement auth tests".to_string()),
                    timestamp: Utc::now(),
                },
                ActivityEvent {
                    event_type: "commit".to_string(),
                    branch: None,
                    commit_hash: None,
                    message: Some("auth cleanup".to_string()),
                    timestamp: Utc::now(),
                },
            ],
            pr_info: None,
            reviews: vec![],
            ai_sessions: vec![],
            presence_intervals: vec![],
            branch_switches: 0,
        };
        let themes = extract_commit_themes(&[repo]);
        assert!(themes.is_empty());
    }

    #[test]
    fn themes_all_stopwords_returns_empty() {
        let events: Vec<ActivityEvent> = (0..10)
            .map(|_| ActivityEvent {
                event_type: "commit".to_string(),
                branch: None,
                commit_hash: None,
                message: Some("fix: add update merge bump".to_string()),
                timestamp: Utc::now(),
            })
            .collect();
        let repo = RepoSummary {
            repo_path: "/r".to_string(),
            repo_name: "r".to_string(),
            commits: 10,
            branches: vec![],
            estimated_time: Duration::zero(),
            events,
            pr_info: None,
            reviews: vec![],
            ai_sessions: vec![],
            presence_intervals: vec![],
            branch_switches: 0,
        };
        let themes = extract_commit_themes(&[repo]);
        assert!(themes.is_empty());
    }

    #[test]
    fn themes_normal_corpus() {
        let messages = vec![
            "feat: implement auth module",
            "fix: auth token refresh",
            "feat: add database migrations",
            "chore: update dependencies",
            "feat: implement caching layer",
            "fix: caching invalidation bug",
            "feat: auth rate limiting",
            "refactor: database connection pool",
            "feat: implement logging middleware",
            "fix: logging format errors",
            "feat: database schema update",
            "test: auth integration tests",
        ];
        let events: Vec<ActivityEvent> = messages
            .into_iter()
            .map(|m| ActivityEvent {
                event_type: "commit".to_string(),
                branch: None,
                commit_hash: None,
                message: Some(m.to_string()),
                timestamp: Utc::now(),
            })
            .collect();
        let repo = RepoSummary {
            repo_path: "/r".to_string(),
            repo_name: "r".to_string(),
            commits: 12,
            branches: vec![],
            estimated_time: Duration::zero(),
            events,
            pr_info: None,
            reviews: vec![],
            ai_sessions: vec![],
            presence_intervals: vec![],
            branch_switches: 0,
        };
        let themes = extract_commit_themes(&[repo]);
        assert!(!themes.is_empty());
        // "auth" appears 4 times — should be first
        assert!(themes[0].contains("auth"), "expected 'auth' first, got: {:?}", themes);
        assert!(themes[0].contains("occurrences"), "expected count format: {}", themes[0]);
        // "database" appears 3 times
        assert!(
            themes.iter().any(|t| t.contains("database")),
            "expected 'database' in themes: {:?}",
            themes
        );
    }

    #[test]
    fn themes_skips_non_commit_events() {
        let events = vec![
            ActivityEvent {
                event_type: "branch_switch".to_string(),
                branch: None,
                commit_hash: None,
                message: Some("auth auth auth auth auth".to_string()),
                timestamp: Utc::now(),
            },
        ];
        // Need 5+ commits total, so add commits with bland messages
        let mut commit_events: Vec<ActivityEvent> = (0..6)
            .map(|_| ActivityEvent {
                event_type: "commit".to_string(),
                branch: None,
                commit_hash: None,
                message: Some("implement pipeline handler".to_string()),
                timestamp: Utc::now(),
            })
            .collect();
        commit_events.extend(events);
        let repo = RepoSummary {
            repo_path: "/r".to_string(),
            repo_name: "r".to_string(),
            commits: 6,
            branches: vec![],
            estimated_time: Duration::zero(),
            events: commit_events,
            pr_info: None,
            reviews: vec![],
            ai_sessions: vec![],
            presence_intervals: vec![],
            branch_switches: 0,
        };
        let themes = extract_commit_themes(&[repo]);
        // "auth" from non-commit event should NOT appear (or if it does, not dominate)
        // "pipeline" and "handler" and "implement" should be top
        // Actually "implement" is not a stop word, so it should appear
        assert!(
            themes.iter().any(|t| t.contains("pipeline")),
            "expected 'pipeline': {:?}",
            themes
        );
    }

    #[test]
    fn themes_caps_at_20() {
        // Generate many distinct words
        let events: Vec<ActivityEvent> = (0..30)
            .map(|i| ActivityEvent {
                event_type: "commit".to_string(),
                branch: None,
                commit_hash: None,
                message: Some(format!(
                    "word{i}alpha word{i}beta word{i}gamma word{i}delta"
                )),
                timestamp: Utc::now(),
            })
            .collect();
        let repo = RepoSummary {
            repo_path: "/r".to_string(),
            repo_name: "r".to_string(),
            commits: 30,
            branches: vec![],
            estimated_time: Duration::zero(),
            events,
            pr_info: None,
            reviews: vec![],
            ai_sessions: vec![],
            presence_intervals: vec![],
            branch_switches: 0,
        };
        let themes = extract_commit_themes(&[repo]);
        assert!(themes.len() <= 20, "expected <=20 themes, got {}", themes.len());
    }

    // --- US-05: PR summary compilation tests ---

    #[test]
    fn pr_summary_empty_repos() {
        let summary = compile_pr_summary(&[]);
        assert!(summary.authored_prs.is_empty());
        assert!(summary.reviewed_prs.is_empty());
    }

    #[test]
    fn pr_summary_no_pr_info_no_reviews() {
        let repo = RepoSummary {
            repo_path: "/r".to_string(),
            repo_name: "r".to_string(),
            commits: 5,
            branches: vec![],
            estimated_time: Duration::zero(),
            events: vec![],
            pr_info: None,
            reviews: vec![],
            ai_sessions: vec![],
            presence_intervals: vec![],
            branch_switches: 0,
        };
        let summary = compile_pr_summary(&[repo]);
        assert!(summary.authored_prs.is_empty());
        assert!(summary.reviewed_prs.is_empty());
    }

    #[test]
    fn pr_summary_collects_authored_prs() {
        let repo = RepoSummary {
            repo_path: "/r".to_string(),
            repo_name: "myrepo".to_string(),
            commits: 5,
            branches: vec![],
            estimated_time: Duration::zero(),
            events: vec![],
            pr_info: Some(vec![
                crate::enrichment::PrInfo {
                    number: 10,
                    title: "First PR".to_string(),
                    state: "MERGED".to_string(),
                    head_ref_name: "feat/a".to_string(),
                    created_at: None,
                    merged_at: None,
                },
                crate::enrichment::PrInfo {
                    number: 20,
                    title: "Second PR".to_string(),
                    state: "OPEN".to_string(),
                    head_ref_name: "feat/b".to_string(),
                    created_at: None,
                    merged_at: None,
                },
            ]),
            reviews: vec![],
            ai_sessions: vec![],
            presence_intervals: vec![],
            branch_switches: 0,
        };
        let summary = compile_pr_summary(&[repo]);
        assert_eq!(summary.authored_prs.len(), 2);
        assert_eq!(summary.authored_prs[0], ("myrepo".to_string(), 10, "First PR".to_string()));
        assert_eq!(summary.authored_prs[1], ("myrepo".to_string(), 20, "Second PR".to_string()));
    }

    #[test]
    fn pr_summary_collects_reviews() {
        let now = Utc::now();
        let repo = RepoSummary {
            repo_path: "/r".to_string(),
            repo_name: "myrepo".to_string(),
            commits: 0,
            branches: vec![],
            estimated_time: Duration::zero(),
            events: vec![],
            pr_info: None,
            reviews: vec![ReviewInfo {
                pr_number: 42,
                pr_title: "Some PR".to_string(),
                action: "APPROVED".to_string(),
                reviewed_at: now,
            }],
            ai_sessions: vec![],
            presence_intervals: vec![],
            branch_switches: 0,
        };
        let summary = compile_pr_summary(&[repo]);
        assert_eq!(summary.reviewed_prs.len(), 1);
        assert_eq!(
            summary.reviewed_prs[0],
            ("myrepo".to_string(), 42, "Some PR".to_string(), "APPROVED".to_string())
        );
    }

    #[test]
    fn pr_summary_dedup_reviews_keeps_highest_priority() {
        let now = Utc::now();
        let repo = RepoSummary {
            repo_path: "/r".to_string(),
            repo_name: "myrepo".to_string(),
            commits: 0,
            branches: vec![],
            estimated_time: Duration::zero(),
            events: vec![],
            pr_info: None,
            reviews: vec![
                ReviewInfo {
                    pr_number: 42,
                    pr_title: "Some PR".to_string(),
                    action: "COMMENTED".to_string(),
                    reviewed_at: now,
                },
                ReviewInfo {
                    pr_number: 42,
                    pr_title: "Some PR".to_string(),
                    action: "APPROVED".to_string(),
                    reviewed_at: now,
                },
                ReviewInfo {
                    pr_number: 42,
                    pr_title: "Some PR".to_string(),
                    action: "CHANGES_REQUESTED".to_string(),
                    reviewed_at: now,
                },
            ],
            ai_sessions: vec![],
            presence_intervals: vec![],
            branch_switches: 0,
        };
        let summary = compile_pr_summary(&[repo]);
        // Should dedup to 1 entry with APPROVED (highest priority)
        assert_eq!(summary.reviewed_prs.len(), 1);
        assert_eq!(summary.reviewed_prs[0].3, "APPROVED");
    }

    #[test]
    fn pr_summary_dedup_reviews_per_repo() {
        let now = Utc::now();
        // Same pr_number in different repos should NOT be deduped
        let repo1 = RepoSummary {
            repo_path: "/r1".to_string(),
            repo_name: "repo1".to_string(),
            commits: 0,
            branches: vec![],
            estimated_time: Duration::zero(),
            events: vec![],
            pr_info: None,
            reviews: vec![ReviewInfo {
                pr_number: 42,
                pr_title: "PR in repo1".to_string(),
                action: "COMMENTED".to_string(),
                reviewed_at: now,
            }],
            ai_sessions: vec![],
            presence_intervals: vec![],
            branch_switches: 0,
        };
        let repo2 = RepoSummary {
            repo_path: "/r2".to_string(),
            repo_name: "repo2".to_string(),
            commits: 0,
            branches: vec![],
            estimated_time: Duration::zero(),
            events: vec![],
            pr_info: None,
            reviews: vec![ReviewInfo {
                pr_number: 42,
                pr_title: "PR in repo2".to_string(),
                action: "APPROVED".to_string(),
                reviewed_at: now,
            }],
            ai_sessions: vec![],
            presence_intervals: vec![],
            branch_switches: 0,
        };
        let summary = compile_pr_summary(&[repo1, repo2]);
        assert_eq!(summary.reviewed_prs.len(), 2);
    }

    #[test]
    fn pr_summary_dedup_changes_requested_over_commented() {
        let now = Utc::now();
        let repo = RepoSummary {
            repo_path: "/r".to_string(),
            repo_name: "myrepo".to_string(),
            commits: 0,
            branches: vec![],
            estimated_time: Duration::zero(),
            events: vec![],
            pr_info: None,
            reviews: vec![
                ReviewInfo {
                    pr_number: 99,
                    pr_title: "Bug fix".to_string(),
                    action: "COMMENTED".to_string(),
                    reviewed_at: now,
                },
                ReviewInfo {
                    pr_number: 99,
                    pr_title: "Bug fix".to_string(),
                    action: "CHANGES_REQUESTED".to_string(),
                    reviewed_at: now,
                },
            ],
            ai_sessions: vec![],
            presence_intervals: vec![],
            branch_switches: 0,
        };
        let summary = compile_pr_summary(&[repo]);
        assert_eq!(summary.reviewed_prs.len(), 1);
        assert_eq!(summary.reviewed_prs[0].3, "CHANGES_REQUESTED");
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
