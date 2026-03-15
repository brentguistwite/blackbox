use crate::query::RepoSummary;
use serde::{Deserialize, Serialize};
use std::process::Command;
use std::sync::OnceLock;


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrInfo {
    pub number: u64,
    pub title: String,
    pub state: String,
    #[serde(rename = "headRefName")]
    pub head_ref_name: String,
}

/// Check once if `gh` CLI is available on PATH.
fn gh_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        Command::new("which")
            .arg("gh")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    })
}

/// Fetch PRs for a repo directory. Returns None on any failure.
fn fetch_prs(repo_path: &str) -> Option<Vec<PrInfo>> {
    let child = Command::new("gh")
        .args([
            "pr",
            "list",
            "--json",
            "number,title,state,headRefName",
            "--limit",
            "5",
        ])
        .current_dir(repo_path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;

    // Wait with timeout via thread
    let handle = std::thread::spawn(move || child.wait_with_output());

    match handle.join() {
        Ok(Ok(output)) if output.status.success() => {
            serde_json::from_slice(&output.stdout).ok()
        }
        _ => None,
    }
}

/// Enrich repo summaries with PR info from gh CLI.
/// Silently returns on any failure (no gh, not authenticated, not GitHub repo, timeout).
pub fn enrich_with_prs(repos: &mut [RepoSummary]) {
    if !gh_available() {
        log::debug!("gh CLI not available, skipping PR enrichment");
        return;
    }

    for repo in repos.iter_mut() {
        let prs = match fetch_prs(&repo.repo_path) {
            Some(prs) => prs,
            None => {
                log::debug!("Failed to fetch PRs for {}", repo.repo_path);
                continue;
            }
        };

        // Match PRs to repo branches
        let matched: Vec<PrInfo> = prs
            .into_iter()
            .filter(|pr| repo.branches.contains(&pr.head_ref_name))
            .collect();

        if !matched.is_empty() {
            repo.pr_info = Some(matched);
        }
    }
}
