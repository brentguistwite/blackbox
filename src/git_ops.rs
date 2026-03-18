use std::path::{Path, PathBuf};

use git2::Repository;
use rusqlite::Connection;

use crate::db;

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
pub fn poll_repo(
    repo_path: &Path,
    db_repo_path: &str,
    state: &mut RepoState,
    conn: &Connection,
) -> anyhow::Result<()> {
    let repo = Repository::open(repo_path)?;

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

    // First poll: seed state, don't record history
    if state.last_commit_oid.is_none() {
        state.last_commit_oid = Some(current_oid);
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
            }
        }
    }

    state.last_commit_oid = Some(current_oid);
    Ok(())
}

/// Try to resolve the source branch of a merge from parent[1].
/// Falls back to first 8 chars of parent OID if no branch found.
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
