use crate::query::ActivitySummary;
use chrono::Duration;
use colored::*;

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
        lines.push(format!(
            "{} [{}] ({})",
            repo.repo_name.bold().green(),
            branches_str.dimmed(),
            format_duration(repo.estimated_time).yellow(),
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
