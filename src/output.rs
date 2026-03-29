use crate::enrichment::PrInfo;
use crate::query::ActivitySummary;
use chrono::{Datelike, Duration, Local};
use colored::*;
use serde::Serialize;
use std::collections::BTreeMap;
use std::io::IsTerminal;

/// Gloria Mark (UC Irvine): avg 23 min to regain deep focus after a context switch.
pub const FOCUS_COST_PER_SWITCH_MINS: i64 = 23;

#[derive(Clone, Debug, Default, clap::ValueEnum)]
pub enum OutputFormat {
    #[default]
    Pretty,
    Json,
    Csv,
}

/// Returns true when stdout is an interactive terminal.
/// Returns false when stdout is a pipe, file redirect, or non-terminal.
pub fn is_tty() -> bool {
    std::io::stdout().is_terminal()
}

/// Resolve output format from --format, --json, --csv flags, and TTY state.
/// Priority: --json > --csv > explicit --format > TTY auto-detect.
/// When stdout is not a TTY and no flags are set, defaults to JSON for pipe-friendliness.
/// Note: Clap's default_value="pretty" is indistinguishable from explicit --format pretty,
/// so piped output with default format will auto-detect to JSON.
pub fn resolve_format(format: OutputFormat, json: bool, csv: bool, tty: bool) -> OutputFormat {
    if json { return OutputFormat::Json; }
    if csv { return OutputFormat::Csv; }
    if !tty && matches!(format, OutputFormat::Pretty)
        && std::env::var("BLACKBOX_FORMAT").as_deref() != Ok("pretty")
    {
        return OutputFormat::Json;
    }
    format
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
    pub streak_days: u32,
    pub total_branch_switches: usize,
    pub repos: Vec<JsonRepo>,
}

#[derive(Serialize)]
pub struct JsonRepo {
    pub repo_name: String,
    pub repo_path: String,
    pub commits: usize,
    pub branch_switches: usize,
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
        streak_days: summary.streak_days,
        total_branch_switches: summary.total_branch_switches,
        repos: summary
            .repos
            .iter()
            .map(|r| JsonRepo {
                repo_name: r.repo_name.clone(),
                repo_path: r.repo_path.clone(),
                commits: r.commits,
                branch_switches: r.branch_switches,
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
        "APPROVED" => "✓".green(),
        "CHANGES_REQUESTED" => "✗".yellow(),
        _ => "💬".cyan(),
    }
}

fn review_priority(action: &str) -> u8 {
    match action {
        "APPROVED" => 3,
        "CHANGES_REQUESTED" => 2,
        _ => 1,
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
    let streak_suffix = if summary.period_label == "Today" && summary.streak_days > 0 {
        format!("  {}", format!("{}-day streak", summary.streak_days).dimmed())
    } else {
        String::new()
    };
    let switch_suffix = if summary.total_branch_switches > 0 {
        let cost = summary.total_branch_switches as i64 * FOCUS_COST_PER_SWITCH_MINS;
        format!(", {} branch switches (~{}m focus cost)", summary.total_branch_switches, cost)
    } else {
        String::new()
    };
    lines.push(format!(
        "{} commits{}{}{} across {} {} ({}){}",
        summary.total_commits,
        review_suffix,
        ai_suffix,
        switch_suffix,
        summary.total_repos,
        repo_word,
        format_duration(summary.total_estimated_time),
        streak_suffix,
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

        // Reviews — deduplicate by PR number, keep most significant action
        if !repo.reviews.is_empty() {
            let mut pr_map: BTreeMap<i64, &crate::query::ReviewInfo> = BTreeMap::new();
            for review in &repo.reviews {
                let entry = pr_map.entry(review.pr_number).or_insert(review);
                if review_priority(&review.action) > review_priority(&entry.action) {
                    *entry = review;
                }
            }
            let unique: Vec<_> = pr_map.values().collect();
            let review_word = if unique.len() == 1 { "PR" } else { "PRs" };
            lines.push(format!(
                "  {} Reviewed {} {}",
                "~".dimmed(),
                unique.len(),
                review_word,
            ));
            for review in unique {
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

        // Branch switches with breadcrumb trail
        if repo.branch_switches > 0 {
            let switch_branches: Vec<&str> = repo
                .events
                .iter()
                .filter(|e| e.event_type == "branch_switch")
                .filter_map(|e| e.branch.as_deref())
                .collect();
            let breadcrumb = if switch_branches.len() > 3 {
                let tail = &switch_branches[switch_branches.len() - 3..];
                format!("...->{}",  tail.join("->"))
            } else {
                switch_branches.join("->")
            };
            let crumb_suffix = if breadcrumb.is_empty() {
                String::new()
            } else {
                format!(" ({})", breadcrumb)
            };
            lines.push(format!(
                "  {} {} branch switches{}",
                "~".dimmed(),
                repo.branch_switches,
                crumb_suffix,
            ));
        }

        lines.push(String::new());
    }

    lines.join("\n")
}

/// Render hour-of-day histogram as ASCII bar chart.
/// Returns empty message if all slots are zero.
pub fn render_hour_histogram(histogram: &[u32; 24]) -> String {
    let max = *histogram.iter().max().unwrap_or(&0);
    if max == 0 {
        return "No commit activity in this period.".dimmed().to_string();
    }

    let max_bar_width: u32 = 20;
    let mut lines: Vec<String> = Vec::new();

    // Find peak hour
    let peak_hour = histogram
        .iter()
        .enumerate()
        .max_by_key(|&(_, v)| v)
        .map(|(i, _)| i)
        .unwrap_or(0);

    for (hour, &count) in histogram.iter().enumerate() {
        let bar_len = if max > 0 {
            (count as u64 * max_bar_width as u64 / max as u64) as usize
        } else {
            0
        };
        let bar = "█".repeat(bar_len);
        let padding = " ".repeat(max_bar_width as usize - bar_len);
        if hour == peak_hour && count > 0 {
            lines.push(format!(
                "{:>2} | {}{} {:>4}  <- peak",
                hour,
                bar.green(),
                padding,
                count
            ));
        } else {
            lines.push(format!(
                "{:>2} | {}{} {:>4}",
                hour,
                bar.yellow(),
                padding,
                count
            ));
        }
    }

    let commit_word = if histogram[peak_hour] == 1 { "commit" } else { "commits" };
    lines.push(format!(
        "Peak: {:02}:00–{:02}:00 ({} {})",
        peak_hour,
        (peak_hour + 1) % 24,
        histogram[peak_hour],
        commit_word,
    ));

    lines.join("\n")
}

/// Render day-of-week histogram as ASCII bar chart.
/// Index 0=Mon..6=Sun. Returns empty message if all slots are zero.
pub fn render_dow_histogram(histogram: &[u32; 7]) -> String {
    const DAY_LABELS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

    let max = *histogram.iter().max().unwrap_or(&0);
    if max == 0 {
        return "No commit activity in this period.".dimmed().to_string();
    }

    let max_bar_width: u32 = 20;
    let mut lines: Vec<String> = Vec::new();

    let peak_idx = histogram
        .iter()
        .enumerate()
        .max_by_key(|&(_, v)| v)
        .map(|(i, _)| i)
        .unwrap_or(0);

    for (i, &count) in histogram.iter().enumerate() {
        let bar_len = if max > 0 {
            (count as u64 * max_bar_width as u64 / max as u64) as usize
        } else {
            0
        };
        let bar = "█".repeat(bar_len);
        let padding = " ".repeat(max_bar_width as usize - bar_len);
        let weekend_tag = if i >= 5 { " [wknd]" } else { "" };

        if i == peak_idx && count > 0 {
            lines.push(format!(
                "{} | {}{} {:>4}  <- peak{}",
                DAY_LABELS[i],
                bar.green(),
                padding,
                count,
                weekend_tag,
            ));
        } else {
            lines.push(format!(
                "{} | {}{} {:>4}{}",
                DAY_LABELS[i],
                bar.yellow(),
                padding,
                count,
                weekend_tag,
            ));
        }
    }

    let commit_word = if histogram[peak_idx] == 1 { "commit" } else { "commits" };
    lines.push(format!(
        "Peak: {} ({} {})",
        DAY_LABELS[peak_idx],
        histogram[peak_idx],
        commit_word,
    ));

    lines.join("\n")
}

/// Render after-hours/weekend stats as a compact display.
/// No evaluative language — mirror, not a score.
pub fn render_after_hours_stats(stats: &crate::query::AfterHoursStats) -> String {
    let mut lines: Vec<String> = Vec::new();

    let ah_pct = (stats.after_hours_ratio * 100.0).round() as u32;
    let wk_pct = (stats.weekend_ratio * 100.0).round() as u32;

    lines.push(format!(
        "After-hours: {}/{} commits ({}%)",
        stats.after_hours_commits, stats.total_commits, ah_pct,
    ));
    lines.push(format!(
        "Weekend:     {}/{} commits ({}%)",
        stats.weekend_commits, stats.total_commits, wk_pct,
    ));

    if stats.after_hours_ratio > 0.5 {
        lines.push("(more than half outside core hours)".to_string());
    }

    lines.join("\n")
}

/// Render session length distribution as compact stats line.
/// Shows median, p90, mean. No flow/quality scoring language.
pub fn render_session_distribution(dist: &crate::query::SessionDistribution) -> String {
    if dist.sessions.is_empty() {
        return "No sessions detected in this period".to_string();
    }

    let session_word = if dist.sessions.len() == 1 { "session" } else { "sessions" };
    format!(
        "Session lengths ({} {}):  median {}  p90 {}  mean {}",
        dist.sessions.len(),
        session_word,
        format_duration(Duration::minutes(dist.median_minutes)),
        format_duration(Duration::minutes(dist.p90_minutes)),
        format_duration(Duration::minutes(dist.mean_minutes)),
    )
}

/// Render burst pattern stats as descriptive label.
/// Neutral pattern labels only — no evaluative language.
pub fn render_burst_stats(stats: &crate::query::BurstStats) -> String {
    match stats.pattern {
        crate::query::CommitPattern::Burst => {
            format!("Commit pattern: bursty (CV={:.2})", stats.cv_of_gaps)
        }
        crate::query::CommitPattern::Steady => {
            format!("Commit pattern: steady (CV={:.2})", stats.cv_of_gaps)
        }
        crate::query::CommitPattern::Insufficient => {
            "Commit pattern: insufficient data (< 3 commits)".to_string()
        }
    }
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

/// Render RepoDeepDive as pretty terminal output.
pub fn render_repo_pretty(dive: &crate::repo_deep_dive::RepoDeepDive) {
    println!("{}", format!("=== {} ===", dive.repo_name).bold().cyan());
    if !dive.tracked {
        println!("{}", "(untracked \u{2014} no activity in DB)".dimmed());
    }
    println!();

    let fmt_opt_date = |d: Option<chrono::DateTime<chrono::Utc>>| {
        d.map(|t| t.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "-".to_string())
    };
    println!("{:<20} {}", "Tracked commits:", dive.total_commits);
    println!(
        "{:<20} {}",
        "Total time:",
        format_duration(dive.total_estimated_time)
    );
    println!(
        "{:<20} {}",
        "First commit:",
        fmt_opt_date(dive.first_commit_at)
    );
    println!(
        "{:<20} {}",
        "Last commit:",
        fmt_opt_date(dive.last_commit_at)
    );
    println!();

    if !dive.languages.is_empty() {
        println!("{}", "Languages:".bold());
        let colors = [
            "cyan", "green", "yellow", "magenta", "blue", "red", "white", "bright_cyan",
        ];
        for (i, lang) in dive.languages.iter().take(8).enumerate() {
            let filled = (lang.percent / 100.0 * 20.0).round() as usize;
            let bar = "\u{2588}".repeat(filled) + &"\u{2591}".repeat(20 - filled);
            let bar_colored = match colors[i % colors.len()] {
                "cyan" => bar.cyan().to_string(),
                "green" => bar.green().to_string(),
                "yellow" => bar.yellow().to_string(),
                "magenta" => bar.magenta().to_string(),
                "blue" => bar.blue().to_string(),
                "red" => bar.red().to_string(),
                "bright_cyan" => bar.bright_cyan().to_string(),
                _ => bar.white().to_string(),
            };
            println!(
                "  {:<16} {:>5.1}%  {}",
                lang.language, lang.percent, bar_colored
            );
        }
        println!();
    }

    if !dive.top_files.is_empty() {
        println!("{}", "Top files changed:".bold());
        for f in dive.top_files.iter().take(10) {
            println!("  {:>4}x  {}", f.change_count, f.path);
        }
        println!();
    }

    if !dive.branches.is_empty() {
        println!("{}", "Branches:".bold());
        for b in &dive.branches {
            println!(
                "  {} \u{2014} {} commits, last active {}",
                b.name,
                b.commit_count,
                b.last_active.format("%Y-%m-%d")
            );
        }
        println!();
    }

    if !dive.prs.is_empty() {
        println!("{}", "Pull requests:".bold());
        for pr in dive.prs.iter().take(10) {
            let state_str = match pr.state.as_str() {
                "MERGED" => format!("[{}]", pr.state).magenta().to_string(),
                "OPEN" => format!("[{}]", pr.state).green().to_string(),
                "CLOSED" => format!("[{}]", pr.state).dimmed().to_string(),
                _ => format!("[{}]", pr.state).cyan().to_string(),
            };
            println!("  #{} {} {}", pr.number, pr.title, state_str);
        }
        println!();
    }
}

// --- JSON serialization for repo deep dive ---

#[derive(Serialize)]
pub struct JsonRepoDeepDive {
    pub repo_path: String,
    pub repo_name: String,
    pub tracked: bool,
    pub total_commits: usize,
    pub total_estimated_minutes: i64,
    pub first_commit_at: Option<String>,
    pub last_commit_at: Option<String>,
    pub languages: Vec<crate::repo_deep_dive::LanguageBreakdown>,
    pub top_files: Vec<crate::repo_deep_dive::FileChurnEntry>,
    pub branches: Vec<JsonBranchActivity>,
    pub prs: Vec<crate::repo_deep_dive::RepoPrEntry>,
}

#[derive(Serialize)]
pub struct JsonBranchActivity {
    pub name: String,
    pub commit_count: usize,
    pub last_active: String,
}

/// Serialize RepoDeepDive to pretty-printed JSON.
pub fn render_repo_json(dive: &crate::repo_deep_dive::RepoDeepDive) -> String {
    let json = JsonRepoDeepDive {
        repo_path: dive.repo_path.clone(),
        repo_name: dive.repo_name.clone(),
        tracked: dive.tracked,
        total_commits: dive.total_commits,
        total_estimated_minutes: dive.total_estimated_time.num_minutes(),
        first_commit_at: dive.first_commit_at.map(|dt| dt.to_rfc3339()),
        last_commit_at: dive.last_commit_at.map(|dt| dt.to_rfc3339()),
        languages: dive.languages.clone(),
        top_files: dive.top_files.clone(),
        branches: dive
            .branches
            .iter()
            .map(|b| JsonBranchActivity {
                name: b.name.clone(),
                commit_count: b.commit_count,
                last_active: b.last_active.to_rfc3339(),
            })
            .collect(),
        prs: dive.prs.clone(),
    };
    serde_json::to_string_pretty(&json).expect("JSON serialization should not fail")
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

// --- Rhythm report ---

/// Aggregated rhythm analysis report for a time window.
#[derive(Debug, Clone, Serialize)]
pub struct RhythmReport {
    pub days: u64,
    pub hour_histogram: [u32; 24],
    pub dow_histogram: [u32; 7],
    pub after_hours: crate::query::AfterHoursStats,
    pub session_distribution: crate::query::SessionDistribution,
    pub burst_stats: crate::query::BurstStats,
}

/// Render full rhythm report as pretty terminal output.
/// Composes all rhythm sections with headers and blank-line separators.
pub fn render_rhythm(report: &RhythmReport) -> String {
    let mut lines: Vec<String> = Vec::new();

    lines.push(
        format!("=== Work Rhythm (last {} days) ===", report.days)
            .bold()
            .cyan()
            .to_string(),
    );
    lines.push(String::new());

    // Hour of day
    lines.push("Hour of day".bold().to_string());
    lines.push(render_hour_histogram(&report.hour_histogram));
    lines.push(String::new());

    // Day of week
    lines.push("Day of week".bold().to_string());
    lines.push(render_dow_histogram(&report.dow_histogram));
    lines.push(String::new());

    // Sustainability (after-hours)
    lines.push("Sustainability".bold().to_string());
    lines.push(render_after_hours_stats(&report.after_hours));
    lines.push(String::new());

    // Session lengths
    lines.push("Session lengths".bold().to_string());
    lines.push(render_session_distribution(&report.session_distribution));
    lines.push(String::new());

    // Commit pattern
    lines.push("Commit pattern".bold().to_string());
    lines.push(render_burst_stats(&report.burst_stats));

    lines.join("\n")
}

/// JSON struct for rhythm report output.
#[derive(Serialize)]
pub struct RhythmReportJson {
    pub days: u64,
    pub hour_histogram: [u32; 24],
    pub dow_histogram: [u32; 7],
    pub after_hours: crate::query::AfterHoursStats,
    pub session_distribution: SessionDistributionJson,
    pub burst_stats: crate::query::BurstStats,
}

#[derive(Serialize)]
pub struct SessionDistributionJson {
    pub session_count: usize,
    pub median_minutes: i64,
    pub p90_minutes: i64,
    pub mean_minutes: i64,
}

/// Render rhythm report as pretty-printed JSON string.
pub fn render_rhythm_json(report: &RhythmReport) -> String {
    let json = RhythmReportJson {
        days: report.days,
        hour_histogram: report.hour_histogram,
        dow_histogram: report.dow_histogram,
        after_hours: report.after_hours.clone(),
        session_distribution: SessionDistributionJson {
            session_count: report.session_distribution.sessions.len(),
            median_minutes: report.session_distribution.median_minutes,
            p90_minutes: report.session_distribution.p90_minutes,
            mean_minutes: report.session_distribution.mean_minutes,
        },
        burst_stats: report.burst_stats.clone(),
    };
    serde_json::to_string_pretty(&json).expect("JSON serialization should not fail")
}
