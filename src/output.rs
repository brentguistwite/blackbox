use crate::enrichment::PrInfo;
use crate::query::ActivitySummary;
use chrono::{Datelike, Duration, Local};
use colored::*;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, clap::ValueEnum)]
pub enum OutputFormat {
    #[default]
    Pretty,
    Json,
    Csv,
}

// --- JSON serialization structs ---

#[derive(Serialize)]
pub struct JsonSummary {
    pub period_label: String,
    pub total_commits: usize,
    pub total_reviews: usize,
    pub total_repos: usize,
    pub total_estimated_minutes: i64,
    pub total_ai_session_minutes: i64,
    pub repos: Vec<JsonRepo>,
}

#[derive(Serialize)]
pub struct JsonRepo {
    pub repo_name: String,
    pub repo_path: String,
    pub commits: usize,
    pub branches: Vec<String>,
    pub estimated_minutes: i64,
    pub events: Vec<JsonEvent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr_info: Option<Vec<PrInfo>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reviews: Vec<JsonReview>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ai_sessions: Vec<JsonAiSession>,
}

#[derive(Serialize)]
pub struct JsonReview {
    pub pr_number: i64,
    pub pr_title: String,
    pub action: String,
    pub reviewed_at: String,
}

#[derive(Serialize)]
pub struct JsonAiSession {
    pub session_id: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub duration_minutes: i64,
    pub turns: Option<i64>,
}

#[derive(Serialize)]
pub struct JsonEvent {
    pub event_type: String,
    pub branch: Option<String>,
    pub commit_hash: Option<String>,
    pub message: Option<String>,
    pub timestamp: String,
}

// --- CSV serialization struct (flat) ---

#[derive(Serialize)]
pub struct CsvRow {
    pub period: String,
    pub repo_name: String,
    pub event_type: String,
    pub branch: String,
    pub commit_hash: String,
    pub message: String,
    pub timestamp: String,
    pub repo_estimated_minutes: i64,
    pub pr_number: String,
    pub pr_title: String,
}

/// Render ActivitySummary as pretty-printed JSON string.
pub fn render_json(summary: &ActivitySummary) -> String {
    let json_summary = JsonSummary {
        period_label: summary.period_label.clone(),
        total_commits: summary.total_commits,
        total_reviews: summary.total_reviews,
        total_repos: summary.total_repos,
        total_estimated_minutes: summary.total_estimated_time.num_minutes(),
        total_ai_session_minutes: summary.total_ai_session_time.num_minutes(),
        repos: summary
            .repos
            .iter()
            .map(|r| JsonRepo {
                repo_name: r.repo_name.clone(),
                repo_path: r.repo_path.clone(),
                commits: r.commits,
                branches: r.branches.clone(),
                estimated_minutes: r.estimated_time.num_minutes(),
                events: r
                    .events
                    .iter()
                    .map(|e| JsonEvent {
                        event_type: e.event_type.clone(),
                        branch: e.branch.clone(),
                        commit_hash: e.commit_hash.clone(),
                        message: e.message.clone(),
                        timestamp: e.timestamp.to_rfc3339(),
                    })
                    .collect(),
                pr_info: r.pr_info.clone(),
                reviews: r
                    .reviews
                    .iter()
                    .map(|rv| JsonReview {
                        pr_number: rv.pr_number,
                        pr_title: rv.pr_title.clone(),
                        action: rv.action.clone(),
                        reviewed_at: rv.reviewed_at.to_rfc3339(),
                    })
                    .collect(),
                ai_sessions: r
                    .ai_sessions
                    .iter()
                    .map(|s| JsonAiSession {
                        session_id: s.session_id.clone(),
                        started_at: s.started_at.to_rfc3339(),
                        ended_at: s.ended_at.map(|dt| dt.to_rfc3339()),
                        duration_minutes: s.duration.num_minutes(),
                        turns: s.turns,
                    })
                    .collect(),
            })
            .collect(),
    };
    serde_json::to_string_pretty(&json_summary).expect("JSON serialization should not fail")
}

/// Render ActivitySummary as CSV string with header row.
pub fn render_csv(summary: &ActivitySummary) -> String {
    let mut wtr = csv::Writer::from_writer(vec![]);
    for repo in &summary.repos {
        // Collect first PR info if available
        let (pr_num, pr_title) = repo
            .pr_info
            .as_ref()
            .and_then(|prs| prs.first())
            .map(|pr| (pr.number.to_string(), pr.title.clone()))
            .unwrap_or_default();
        for event in &repo.events {
            let row = CsvRow {
                period: summary.period_label.clone(),
                repo_name: repo.repo_name.clone(),
                event_type: event.event_type.clone(),
                branch: event.branch.clone().unwrap_or_default(),
                commit_hash: event.commit_hash.clone().unwrap_or_default(),
                message: event.message.clone().unwrap_or_default(),
                timestamp: event.timestamp.to_rfc3339(),
                repo_estimated_minutes: repo.estimated_time.num_minutes(),
                pr_number: pr_num.clone(),
                pr_title: pr_title.clone(),
            };
            wtr.serialize(row).expect("CSV serialization should not fail");
        }
        // Add review rows
        for review in &repo.reviews {
            let action_label = match review.action.as_str() {
                "APPROVED" => "review_approved",
                "CHANGES_REQUESTED" => "review_changes_requested",
                _ => "review_commented",
            };
            let row = CsvRow {
                period: summary.period_label.clone(),
                repo_name: repo.repo_name.clone(),
                event_type: action_label.to_string(),
                branch: String::new(),
                commit_hash: String::new(),
                message: format!("PR #{}: {}", review.pr_number, review.pr_title),
                timestamp: review.reviewed_at.to_rfc3339(),
                repo_estimated_minutes: repo.estimated_time.num_minutes(),
                pr_number: review.pr_number.to_string(),
                pr_title: review.pr_title.clone(),
            };
            wtr.serialize(row).expect("CSV serialization should not fail");
        }
        // Add AI session rows
        for session in &repo.ai_sessions {
            let status = if session.ended_at.is_some() { "ended" } else { "active" };
            let row = CsvRow {
                period: summary.period_label.clone(),
                repo_name: repo.repo_name.clone(),
                event_type: "ai_session".to_string(),
                branch: String::new(),
                commit_hash: String::new(),
                message: format!("Claude Code session ({}, {}m)", status, session.duration.num_minutes()),
                timestamp: session.started_at.to_rfc3339(),
                repo_estimated_minutes: repo.estimated_time.num_minutes(),
                pr_number: String::new(),
                pr_title: String::new(),
            };
            wtr.serialize(row).expect("CSV serialization should not fail");
        }
    }
    // If no rows written, still need header
    if summary.repos.is_empty() || summary.repos.iter().all(|r| r.events.is_empty()) {
        // Write a dummy to force header, then remove the data line
        wtr.write_record([
            "period",
            "repo_name",
            "event_type",
            "branch",
            "commit_hash",
            "message",
            "timestamp",
            "repo_estimated_minutes",
            "pr_number",
            "pr_title",
        ])
        .expect("CSV header write should not fail");
        let data = String::from_utf8(wtr.into_inner().expect("flush")).expect("utf8");
        // The write_record wrote the header already via first serialize, but we used write_record
        // Actually since no serialize was called, write_record is the only row
        return data.trim_end().to_string();
    }
    let data = String::from_utf8(wtr.into_inner().expect("flush")).expect("utf8");
    data.trim_end().to_string()
}

/// Format duration with ~ prefix. e.g. "~1h 30m", "~45m", "~0m"
pub fn format_duration(d: Duration) -> String {
    let total_minutes = d.num_minutes();
    let hours = total_minutes / 60;
    let minutes = total_minutes % 60;
    if hours > 0 {
        format!("~{}h {}m", hours, minutes)
    } else {
        format!("~{}m", minutes)
    }
}

fn review_action_icon(action: &str) -> ColoredString {
    match action {
        "APPROVED" => "v".green(),
        "CHANGES_REQUESTED" => "!".yellow(),
        _ => "c".cyan(),
    }
}

/// Render summary to a String (for testing). No colors when colored override is false.
pub fn render_summary_to_string(summary: &ActivitySummary) -> String {
    let mut lines: Vec<String> = Vec::new();

    if summary.repos.is_empty() {
        lines.push(
            format!("No activity recorded for {}.", summary.period_label)
                .dimmed()
                .to_string(),
        );
        return lines.join("\n");
    }

    // Header
    lines.push(format!("=== {} ===", summary.period_label).bold().cyan().to_string());
    lines.push(String::new());

    // Summary line
    let repo_word = if summary.total_repos == 1 { "repo" } else { "repos" };
    let review_suffix = if summary.total_reviews > 0 {
        format!(", {} reviews", summary.total_reviews)
    } else {
        String::new()
    };
    let ai_suffix = if summary.total_ai_session_time > Duration::zero() {
        format!(", AI sessions: {}", format_duration(summary.total_ai_session_time))
    } else {
        String::new()
    };
    lines.push(format!(
        "{} commits{}{} across {} {} ({})",
        summary.total_commits,
        review_suffix,
        ai_suffix,
        summary.total_repos,
        repo_word,
        format_duration(summary.total_estimated_time),
    ));
    lines.push(String::new());

    // Per-repo breakdown
    for repo in &summary.repos {
        let branches_str = repo.branches.join(", ");
        let pr_str = repo
            .pr_info
            .as_ref()
            .map(|prs| {
                prs.iter()
                    .map(|pr| format!("[PR #{}: {}]", pr.number, pr.title))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default();
        let pr_suffix = if pr_str.is_empty() {
            String::new()
        } else {
            format!(" {}", pr_str.cyan())
        };
        lines.push(format!(
            "{} [{}] ({}){}",
            repo.repo_name.bold().green(),
            branches_str.dimmed(),
            format_duration(repo.estimated_time).yellow(),
            pr_suffix,
        ));

        for event in &repo.events {
            match event.event_type.as_str() {
                "commit" => {
                    let hash = event
                        .commit_hash
                        .as_deref()
                        .map(|h| if h.len() > 7 { &h[..7] } else { h })
                        .unwrap_or("-------");
                    let msg = event.message.as_deref().unwrap_or("");
                    lines.push(format!("  {} {}", hash.dimmed(), msg));
                }
                other => {
                    // branch_switch, merge, etc
                    let detail = event
                        .branch
                        .as_deref()
                        .map(|b| format!(" -> {}", b))
                        .unwrap_or_default();
                    lines.push(format!("  {} {}{}", "~".dimmed(), other.italic(), detail));
                }
            }
        }

        // Reviews
        if !repo.reviews.is_empty() {
            let review_word = if repo.reviews.len() == 1 { "PR" } else { "PRs" };
            lines.push(format!(
                "  {} Reviewed {} {}",
                "~".dimmed(),
                repo.reviews.len(),
                review_word,
            ));
            for review in &repo.reviews {
                let icon = review_action_icon(&review.action);
                lines.push(format!(
                    "    {} PR #{}: {}",
                    icon,
                    review.pr_number,
                    review.pr_title,
                ));
            }
        }

        // AI Sessions
        if !repo.ai_sessions.is_empty() {
            let total_session_time: Duration = repo.ai_sessions.iter().map(|s| s.duration).fold(Duration::zero(), |a, b| a + b);
            lines.push(format!(
                "  {} {} Claude Code sessions ({})",
                "~".dimmed(),
                repo.ai_sessions.len(),
                format_duration(total_session_time).magenta(),
            ));
            for session in &repo.ai_sessions {
                let status = if session.ended_at.is_none() {
                    "active".magenta().to_string()
                } else {
                    format_duration(session.duration)
                };
                let turns_str = session.turns.map(|t| format!(", {} turns", t)).unwrap_or_default();
                lines.push(format!(
                    "    {} {}{}",
                    "o".magenta(),
                    status,
                    turns_str,
                ));
            }
        }

        lines.push(String::new());
    }

    lines.join("\n")
}

/// Print summary to stdout with colors.
pub fn render_summary(summary: &ActivitySummary) {
    print!("{}", render_summary_to_string(summary));
}

/// Format duration without ~ prefix for standup output.
fn format_duration_plain(d: Duration) -> String {
    let total_minutes = d.num_minutes();
    let hours = total_minutes / 60;
    let minutes = total_minutes % 60;
    if hours > 0 {
        format!("{}h {}m", hours, minutes)
    } else {
        format!("{}m", minutes)
    }
}

/// Render activity in Slack/Teams-friendly plain text (no ANSI codes).
pub fn render_standup(summary: &ActivitySummary) -> String {
    let mut lines: Vec<String> = Vec::new();

    if summary.repos.is_empty() {
        lines.push(format!("No activity recorded for {}.", summary.period_label));
        return lines.join("\n");
    }

    // Header with date
    let now = Local::now();
    let header = if summary.period_label == "This Week" {
        let weekday = now.weekday().num_days_from_monday();
        let monday = now - Duration::days(weekday as i64);
        format!("**{} ({} - {})**", summary.period_label, monday.format("%b %-d"), now.format("%b %-d"))
    } else {
        format!("**{} ({})**", summary.period_label, now.format("%b %-d"))
    };
    lines.push(header);
    lines.push(String::new());

    for repo in &summary.repos {
        lines.push(format!("\u{2022} {} (~{})", repo.repo_name, format_duration_plain(repo.estimated_time)));

        // Group commits by branch
        let mut branch_commits: BTreeMap<&str, usize> = BTreeMap::new();
        for event in &repo.events {
            if event.event_type == "commit" {
                let branch = event.branch.as_deref().unwrap_or("unknown");
                *branch_commits.entry(branch).or_default() += 1;
            }
        }
        for (branch, count) in &branch_commits {
            let word = if *count == 1 { "commit" } else { "commits" };
            lines.push(format!("  - {}: {} {}", branch, count, word));
        }

        // PR info
        if let Some(prs) = &repo.pr_info {
            for pr in prs {
                let action = if pr.state == "MERGED" { "Merged" } else { "Opened" };
                lines.push(format!("  - {} PR #{}", action, pr.number));
            }
        }

        // Reviews — deduplicate by PR number
        if !repo.reviews.is_empty() {
            let mut seen = std::collections::BTreeSet::new();
            let unique: Vec<String> = repo.reviews.iter()
                .filter(|r| seen.insert(r.pr_number))
                .map(|r| format!("PR #{}", r.pr_number))
                .collect();
            lines.push(format!("  - Reviewed {}", unique.join(", ")));
        }

        // AI Sessions
        if !repo.ai_sessions.is_empty() {
            let total: Duration = repo.ai_sessions.iter().map(|s| s.duration).fold(Duration::zero(), |a, b| a + b);
            let word = if repo.ai_sessions.len() == 1 { "session" } else { "sessions" };
            lines.push(format!("  - {} Claude Code {} ({})", repo.ai_sessions.len(), word, format_duration_plain(total)));
        }
    }

    // Total summary
    lines.push(String::new());
    let repo_word = if summary.total_repos == 1 { "repo" } else { "repos" };
    lines.push(format!("Total: ~{} across {} {}", format_duration_plain(summary.total_estimated_time), summary.total_repos, repo_word));

    lines.join("\n")
}
