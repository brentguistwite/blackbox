use std::path::{Path, PathBuf};

use chrono::TimeZone;
use git2::Repository;
use log::{info, warn};
use rusqlite::Connection;

use crate::db;

/// Resolve the local user's git identity (email, name) for commit filtering.
fn get_user_identity(repo: &Repository) -> Option<(Option<String>, Option<String>)> {
    let config = repo.config().ok()?;
    let email = config.get_string("user.email").ok();
    let name = config.get_string("user.name").ok();
    if email.is_none() && name.is_none() {
        return None;
    }
    Some((email, name))
}

/// Check if a commit was authored by the local git user.
#[allow(clippy::collapsible_if)]
fn is_own_commit(commit: &git2::Commit, identity: &(Option<String>, Option<String>)) -> bool {
    let author = commit.author();
    if let Some(ref email) = identity.0 {
        if let Some(author_email) = author.email() {
            if author_email.eq_ignore_ascii_case(email) {
                return true;
            }
        }
    }
    if let Some(ref name) = identity.1 {
        if let Some(author_name) = author.name() {
            if author_name == name {
                return true;
            }
        }
    }
    false
}

#[derive(Debug, Default)]
pub struct RepoState {
    pub last_commit_oid: Option<git2::Oid>,
    pub last_head_branch: Option<String>,
    /// For regular repos, equals the scanned path. For worktrees, equals the main repo root.
    pub main_repo_path: PathBuf,
}

/// Poll a repo for git activity.
/// `repo_path` = filesystem path for git2 to open (worktree or regular).
/// `db_repo_path` = path string for DB writes (always main repo path).
#[allow(clippy::collapsible_if, clippy::explicit_counter_loop)]
pub fn poll_repo(
    repo_path: &Path,
    db_repo_path: &str,
    state: &mut RepoState,
    conn: &Connection,
) -> anyhow::Result<()> {
    let repo = Repository::open(repo_path)?;

    let identity = get_user_identity(&repo);
    if identity.is_none() {
        warn!(
            "No user.email/user.name in git config for {} — recording all commits",
            repo_path.display()
        );
    }

    // Get current HEAD info
    let head = repo.head()?;
    let current_branch = if repo.head_detached()? {
        None
    } else {
        head.shorthand().map(|s| s.to_string())
    };
    let head_commit = head.peel_to_commit()?;
    let current_oid = head_commit.id();

    // Detect branch switch
    if state.last_head_branch.as_deref() != current_branch.as_deref() {
        if state.last_head_branch.is_some() {
            let ts = chrono::Utc::now().to_rfc3339();
            db::insert_activity(
                conn,
                db_repo_path,
                "branch_switch",
                current_branch.as_deref(),
                None,
                None,
                None,
                None,
                &ts,
            )?;
        }
        state.last_head_branch = current_branch.clone();
    }

    // First poll: seed state, backfill today's commits
    if state.last_commit_oid.is_none() {
        state.last_commit_oid = Some(current_oid);

        // Compute midnight local time as UTC epoch seconds
        let midnight_local = chrono::Local::now()
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        let midnight_epoch = chrono::Local
            .from_local_datetime(&midnight_local)
            .earliest()
            .unwrap()
            .timestamp();

        let mut revwalk = repo.revwalk()?;
        revwalk.push_head()?;
        revwalk.set_sorting(git2::Sort::TIME)?;

        let mut count = 0u32;
        for oid_result in revwalk {
            let oid = match oid_result {
                Ok(o) => o,
                Err(_) => break, // shallow clone boundary or corrupt history
            };
            let commit = match repo.find_commit(oid) {
                Ok(c) => c,
                Err(_) => break,
            };

            if commit.time().seconds() < midnight_epoch {
                break;
            }
            if count >= 50 {
                info!("Backfill capped at 50 commits for {}", db_repo_path);
                break;
            }
            count += 1;

            // Skip commits not authored by the local user
            if let Some(ref id) = identity {
                if !is_own_commit(&commit, id) {
                    continue;
                }
            }

            let author_name = commit.author().name().unwrap_or("unknown").to_string();
            let message = commit.message().unwrap_or("").to_string();
            let time = commit.time();
            let ts = chrono::DateTime::from_timestamp(time.seconds(), 0)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_default();
            let hash = oid.to_string();

            if commit.parent_count() > 1 {
                let source_branch = resolve_source_branch(&repo, &commit);
                db::insert_activity(
                    conn,
                    db_repo_path,
                    "merge",
                    current_branch.as_deref(),
                    Some(&source_branch),
                    Some(&hash),
                    Some(&author_name),
                    Some(&message),
                    &ts,
                )?;
            } else {
                db::insert_activity(
                    conn,
                    db_repo_path,
                    "commit",
                    current_branch.as_deref(),
                    None,
                    Some(&hash),
                    Some(&author_name),
                    Some(&message),
                    &ts,
                )?;
                backfill_line_stats(&repo, &commit, conn, db_repo_path, &hash, &ts);
            }
        }

        return Ok(());
    }

    let last_oid = state.last_commit_oid.unwrap();

    // Walk new commits since last known
    if last_oid != current_oid {
        let mut revwalk = repo.revwalk()?;
        revwalk.push_head()?;
        revwalk.hide(last_oid)?;
        revwalk.set_sorting(git2::Sort::TIME)?;

        for oid_result in revwalk {
            let oid = oid_result?;
            let commit = repo.find_commit(oid)?;

            // Skip commits not authored by the local user
            if let Some(ref id) = identity {
                if !is_own_commit(&commit, id) {
                    continue;
                }
            }

            let author_name = commit.author().name().unwrap_or("unknown").to_string();
            let message = commit.message().unwrap_or("").to_string();
            let time = commit.time();
            let ts = chrono::DateTime::from_timestamp(time.seconds(), 0)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_default();
            let hash = oid.to_string();

            if commit.parent_count() > 1 {
                // Merge commit -- resolve source branch from parent[1]
                let source_branch = resolve_source_branch(&repo, &commit);
                db::insert_activity(
                    conn,
                    db_repo_path,
                    "merge",
                    current_branch.as_deref(),
                    Some(&source_branch),
                    Some(&hash),
                    Some(&author_name),
                    Some(&message),
                    &ts,
                )?;
            } else {
                db::insert_activity(
                    conn,
                    db_repo_path,
                    "commit",
                    current_branch.as_deref(),
                    None,
                    Some(&hash),
                    Some(&author_name),
                    Some(&message),
                    &ts,
                )?;
                backfill_line_stats(&repo, &commit, conn, db_repo_path, &hash, &ts);
            }
        }
    }

    state.last_commit_oid = Some(current_oid);
    Ok(())
}

/// Compute per-file line stats for a commit and insert into DB. Logs warnings on failure.
fn backfill_line_stats(
    repo: &Repository,
    commit: &git2::Commit,
    conn: &Connection,
    db_repo_path: &str,
    hash: &str,
    ts: &str,
) {
    match crate::churn::diff_commit_stats(repo, commit) {
        Ok(file_stats) => {
            for stat in &file_stats {
                if let Err(e) = db::insert_commit_line_stats(
                    conn,
                    db_repo_path,
                    hash,
                    &stat.file_path,
                    stat.lines_added as i64,
                    stat.lines_deleted as i64,
                    ts,
                ) {
                    warn!("insert_commit_line_stats failed: {e}");
                }
            }
        }
        Err(e) => {
            warn!("diff_commit_stats failed for {hash}: {e}");
        }
    }
}

/// Try to resolve the source branch of a merge from parent[1].
/// Falls back to first 8 chars of parent OID if no branch found.
#[allow(clippy::collapsible_if, clippy::manual_flatten)]
fn resolve_source_branch(repo: &Repository, merge_commit: &git2::Commit) -> String {
    if merge_commit.parent_count() < 2 {
        return String::new();
    }

    let parent_oid = match merge_commit.parent_id(1) {
        Ok(oid) => oid,
        Err(_) => return String::new(),
    };

    // Try to find a branch pointing at this commit
    if let Ok(branches) = repo.branches(Some(git2::BranchType::Local)) {
        for branch_result in branches {
            if let Ok((branch, _)) = branch_result {
                if let Some(target) = branch.get().target() {
                    if target == parent_oid {
                        if let Some(name) = branch.name().ok().flatten() {
                            return name.to_string();
                        }
                    }
                }
            }
        }
    }

    // Fallback: first 8 chars of parent OID
    parent_oid.to_string()[..8].to_string()
}
