use crate::enrichment::PrInfo;
use crate::query::ActivitySummary;
use chrono::Duration;
use colored::*;
use serde::Serialize;

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
    pub total_repos: usize,
    pub total_estimated_minutes: i64,
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
        total_repos: summary.total_repos,
        total_estimated_minutes: summary.total_estimated_time.num_minutes(),
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
    lines.push(format!(
        "{} commits across {} {} ({})",
        summary.total_commits,
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
        lines.push(String::new());
    }

    lines.join("\n")
}

/// Print summary to stdout with colors.
pub fn render_summary(summary: &ActivitySummary) {
    print!("{}", render_summary_to_string(summary));
}
