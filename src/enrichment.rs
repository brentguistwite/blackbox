use crate::db;
use crate::query::RepoSummary;
use rusqlite::Connection;
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

// --- Review activity tracking ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GhReviewAuthor {
    pub login: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GhReview {
    pub author: GhReviewAuthor,
    pub state: String,
    #[serde(rename = "submittedAt")]
    pub submitted_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GhPrWithReviews {
    pub number: u64,
    pub title: String,
    pub url: String,
    #[serde(default)]
    pub reviews: Vec<GhReview>,
}

/// Get current GitHub username, cached via OnceLock. Returns None if unavailable.
fn gh_username() -> Option<&'static str> {
    static USERNAME: OnceLock<Option<String>> = OnceLock::new();
    USERNAME
        .get_or_init(|| {
            let child = Command::new("gh")
                .args(["api", "user", "--jq", ".login"])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .spawn()
                .ok()?;

            let handle = std::thread::spawn(move || child.wait_with_output());
            match handle.join() {
                Ok(Ok(output)) if output.status.success() => {
                    let login = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if login.is_empty() { None } else { Some(login) }
                }
                _ => None,
            }
        })
        .as_deref()
}

/// Fetch PRs reviewed by current user for a repo. Returns None on any failure.
fn fetch_reviewed_prs(repo_path: &str) -> Option<Vec<GhPrWithReviews>> {
    let child = Command::new("gh")
        .args([
            "pr",
            "list",
            "--reviewed-by",
            "@me",
            "--state",
            "all",
            "--json",
            "number,title,url,reviews",
            "--limit",
            "20",
        ])
        .current_dir(repo_path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;

    let handle = std::thread::spawn(move || child.wait_with_output());
    match handle.join() {
        Ok(Ok(output)) if output.status.success() => {
            serde_json::from_slice(&output.stdout).ok()
        }
        _ => None,
    }
}

/// Collect reviews for all given repo paths, dedup and insert into DB.
/// Silently skips if gh not available or username can't be determined.
pub fn collect_reviews(repo_paths: &[std::path::PathBuf], conn: &Connection) {
    if !gh_available() {
        log::debug!("gh CLI not available, skipping review collection");
        return;
    }

    let username = match gh_username() {
        Some(u) => u,
        None => {
            log::debug!("Could not determine gh username, skipping review collection");
            return;
        }
    };

    for repo_path in repo_paths {
        let repo_str = repo_path.to_string_lossy();
        let prs = match fetch_reviewed_prs(&repo_str) {
            Some(prs) => prs,
            None => {
                log::debug!("Failed to fetch reviews for {}", repo_str);
                continue;
            }
        };

        for pr in &prs {
            // Filter to current user's reviews only
            for review in &pr.reviews {
                if review.author.login != username {
                    continue;
                }
                // Map gh review states to our action names
                let action = match review.state.as_str() {
                    "APPROVED" => "APPROVED",
                    "CHANGES_REQUESTED" => "CHANGES_REQUESTED",
                    "COMMENTED" => "COMMENTED",
                    _ => continue, // skip PENDING, DISMISSED, etc.
                };

                match db::insert_review(
                    conn,
                    &repo_str,
                    pr.number as i64,
                    &pr.title,
                    &pr.url,
                    action,
                    &review.submitted_at,
                ) {
                    Ok(true) => log::debug!(
                        "Recorded review: {} PR#{} ({})",
                        repo_str,
                        pr.number,
                        action
                    ),
                    Ok(false) => {} // duplicate, skip silently
                    Err(e) => log::warn!(
                        "Failed to insert review for {} PR#{}: {}",
                        repo_str,
                        pr.number,
                        e
                    ),
                }
            }
        }
    }
}
