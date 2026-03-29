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

// --- PR detail fetching (cycle time metrics) ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GhCommit {
    pub oid: String,
    #[serde(rename = "committedDate")]
    pub committed_date: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GhPrDetail {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: String,
    #[serde(rename = "headRefName")]
    pub head_ref_name: String,
    #[serde(rename = "baseRefName")]
    pub base_ref_name: String,
    #[serde(default)]
    pub author: Option<GhReviewAuthor>,
    #[serde(rename = "createdAt")]
    pub created_at: Option<String>,
    #[serde(rename = "mergedAt")]
    pub merged_at: Option<String>,
    #[serde(rename = "closedAt")]
    pub closed_at: Option<String>,
    #[serde(default)]
    pub reviews: Vec<GhReview>,
    #[serde(default)]
    pub additions: Option<i64>,
    #[serde(default)]
    pub deletions: Option<i64>,
    #[serde(default)]
    pub commits: Vec<GhCommit>,
}

/// Fetch detailed PR data for cycle time metrics. Returns None on any failure.
/// Limit 50 caps API requests; PRs older than the 50 most-recent open/closed may not be captured.
pub fn fetch_pr_details(repo_path: &str) -> Option<Vec<GhPrDetail>> {
    if !gh_available() {
        return None;
    }

    let child = Command::new("gh")
        .args([
            "pr",
            "list",
            "--state",
            "all",
            "--limit",
            "50",
            "--json",
            "number,title,url,state,headRefName,baseRefName,author,createdAt,mergedAt,closedAt,reviews,additions,deletions,commits",
        ])
        .current_dir(repo_path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;

    let handle = std::thread::spawn(move || child.wait_with_output());
    match handle.join() {
        Ok(Ok(output)) if output.status.success() => serde_json::from_slice(&output.stdout).ok(),
        _ => None,
    }
}

/// Collect PR snapshots for all given repo paths, dedup worktrees, upsert into DB.
/// Silently returns if gh not available.
pub fn collect_pr_snapshots(repo_paths: &[std::path::PathBuf], conn: &Connection) {
    use crate::repo_scanner;
    use std::collections::HashSet;

    if !gh_available() {
        log::debug!("gh CLI not available, skipping PR snapshot collection");
        return;
    }

    let mut seen_main_repos: HashSet<std::path::PathBuf> = HashSet::new();
    for repo_path in repo_paths {
        let main_repo = if repo_scanner::is_worktree(repo_path).is_some() {
            repo_scanner::resolve_main_repo(repo_path).unwrap_or_else(|_| repo_path.clone())
        } else {
            repo_path.clone()
        };

        if !seen_main_repos.insert(main_repo.clone()) {
            continue;
        }

        let main_str = main_repo.to_string_lossy();
        let prs = match fetch_pr_details(&repo_path.to_string_lossy()) {
            Some(prs) => prs,
            None => {
                log::debug!("Failed to fetch PR details for {}", main_str);
                continue;
            }
        };

        for pr in &prs {
            match db::upsert_pr_snapshot(conn, &main_str, pr) {
                Ok(()) => log::debug!("Upserted PR #{} for {}", pr.number, main_str),
                Err(e) => log::warn!("Failed to upsert PR #{} for {}: {}", pr.number, main_str, e),
            }
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
            "--search",
            "reviewed-by:@me",
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
    use crate::repo_scanner;
    use std::collections::HashSet;

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

    // Dedup: resolve worktrees to main repo, only fetch reviews once per main repo.
    let mut seen_main_repos: HashSet<std::path::PathBuf> = HashSet::new();
    for repo_path in repo_paths {
        let main_repo = if repo_scanner::is_worktree(repo_path).is_some() {
            repo_scanner::resolve_main_repo(repo_path).unwrap_or_else(|_| repo_path.clone())
        } else {
            repo_path.clone()
        };

        if !seen_main_repos.insert(main_repo.clone()) {
            continue; // already fetched reviews for this main repo
        }

        let main_str = main_repo.to_string_lossy();
        // Run gh from repo_path (works from both main repo and worktree)
        let prs = match fetch_reviewed_prs(&repo_path.to_string_lossy()) {
            Some(prs) => prs,
            None => {
                log::debug!("Failed to fetch reviews for {}", main_str);
                continue;
            }
        };

        for pr in &prs {
            for review in &pr.reviews {
                if review.author.login != username {
                    continue;
                }
                let action = match review.state.as_str() {
                    "APPROVED" => "APPROVED",
                    "CHANGES_REQUESTED" => "CHANGES_REQUESTED",
                    "COMMENTED" => "COMMENTED",
                    _ => continue,
                };

                // Store under main repo path, not worktree path
                match db::insert_review(
                    conn,
                    &main_str,
                    pr.number as i64,
                    &pr.title,
                    &pr.url,
                    action,
                    &review.submitted_at,
                ) {
                    Ok(true) => log::debug!(
                        "Recorded review: {} PR#{} ({})",
                        main_str,
                        pr.number,
                        action
                    ),
                    Ok(false) => {}
                    Err(e) => log::warn!(
                        "Failed to insert review for {} PR#{}: {}",
                        main_str,
                        pr.number,
                        e
                    ),
                }
            }
        }
    }
}
