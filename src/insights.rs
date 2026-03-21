use crate::query::RepoSummary;

#[derive(Debug, Clone)]
pub struct ContextSwitchMetrics {
    pub branch_switches: usize,
    pub repo_switches: usize,
    pub focus_score: f64,
}

/// Count context switches across repos. Pure function — no DB access.
///
/// - branch_switches: count of "branch_switch" events across all repos
/// - repo_switches: chronological transitions between different repos
/// - focus_score: 1.0 / (1.0 + switches_per_hour)
pub fn context_switches(repos: &[RepoSummary]) -> ContextSwitchMetrics {
    if repos.is_empty() {
        return ContextSwitchMetrics {
            branch_switches: 0,
            repo_switches: 0,
            focus_score: 1.0,
        };
    }

    let branch_switches: usize = repos
        .iter()
        .flat_map(|r| &r.events)
        .filter(|e| e.event_type == "branch_switch")
        .count();

    // Flatten all events tagged with repo path, sort by timestamp
    let mut tagged: Vec<(&str, &crate::query::ActivityEvent)> = repos
        .iter()
        .flat_map(|r| r.events.iter().map(move |e| (r.repo_path.as_str(), e)))
        .collect();
    tagged.sort_by_key(|(_, e)| e.timestamp);

    let repo_switches = tagged
        .windows(2)
        .filter(|w| w[0].0 != w[1].0)
        .count();

    let total_minutes: f64 = repos.iter().map(|r| r.estimated_time.num_minutes() as f64).sum();
    let total_hours = total_minutes / 60.0;
    let total_switches = (branch_switches + repo_switches) as f64;
    let switches_per_hour = if total_hours > 0.0 { total_switches / total_hours } else { 0.0 };
    let focus_score = 1.0 / (1.0 + switches_per_hour);

    ContextSwitchMetrics {
        branch_switches,
        repo_switches,
        focus_score,
    }
}
