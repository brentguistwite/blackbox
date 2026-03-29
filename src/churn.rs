use anyhow::Result;
use chrono::{Duration, Utc};
use git2::{Commit, DiffOptions, Repository};
use rusqlite::Connection;
use std::collections::HashMap;

use crate::db;

#[derive(Debug, Clone)]
pub struct FileLineStat {
    pub file_path: String,
    pub lines_added: u32,
    pub lines_deleted: u32,
}

/// Extract per-file line stats from a commit by diffing against its parent.
/// Returns empty Vec for merge commits (parent_count > 1).
/// For initial commits (no parent), diffs against empty tree.
pub fn diff_commit_stats(repo: &Repository, commit: &Commit) -> Result<Vec<FileLineStat>> {
    if commit.parent_count() > 1 {
        return Ok(Vec::new());
    }

    let tree = commit.tree()?;
    let parent_tree = if commit.parent_count() > 0 {
        Some(commit.parent(0)?.tree()?)
    } else {
        None
    };

    let mut opts = DiffOptions::new();
    let diff = repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), Some(&mut opts))?;

    let mut stats: HashMap<String, (u32, u32)> = HashMap::new();

    diff.foreach(
        &mut |_delta, _progress| true,
        None,
        None,
        Some(&mut |delta, _hunk, line| {
            let path = match delta.new_file().path() {
                Some(p) => p.to_string_lossy().to_string(),
                None => return true,
            };
            match line.origin() {
                '+' => {
                    stats.entry(path).or_insert((0, 0)).0 += 1;
                }
                '-' => {
                    // For deletions, use old_file path (handles renames)
                    let del_path = delta
                        .old_file()
                        .path()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or(path);
                    stats.entry(del_path).or_insert((0, 0)).1 += 1;
                }
                _ => {}
            }
            true
        }),
    )?;

    let result = stats
        .into_iter()
        .map(|(file_path, (added, deleted))| FileLineStat {
            file_path,
            lines_added: added,
            lines_deleted: deleted,
        })
        .collect();

    Ok(result)
}

#[derive(Debug, Clone)]
pub struct ChurnReport {
    pub repo_path: String,
    pub window_days: u32,
    pub total_lines_written: u64,
    pub churned_lines: u64,
    pub churn_rate_pct: f64,
    pub commit_count: usize,
    pub churn_event_count: usize,
}

/// Compute churn rate for a repo over a time window.
/// Fetches commit_line_stats from DB, groups by file, detects churn pairs,
/// inserts churn_events, returns aggregate report.
pub fn compute_churn(conn: &Connection, repo_path: &str, window_days: u32) -> Result<ChurnReport> {
    let since = Utc::now() - Duration::days(i64::from(window_days));
    let stats = db::query_commit_line_stats_for_repo(conn, repo_path, since)?;

    let commit_count = {
        let mut hashes = std::collections::HashSet::new();
        for s in &stats {
            hashes.insert(&s.commit_hash);
        }
        hashes.len()
    };

    let total_lines_written: u64 = stats.iter().map(|s| s.lines_added as u64).sum();

    // Group by file_path
    let mut by_file: HashMap<&str, Vec<&db::CommitLineStat>> = HashMap::new();
    for s in &stats {
        by_file.entry(&s.file_path).or_default().push(s);
    }

    let mut churned_lines: u64 = 0;
    let mut churn_event_count: usize = 0;
    let detected_at = Utc::now().to_rfc3339();

    for entries in by_file.values() {
        // Already sorted by committed_at ASC from query, but entries may interleave
        let mut sorted = entries.clone();
        sorted.sort_by_key(|e| e.committed_at);

        for (i, a) in sorted.iter().enumerate() {
            for b in sorted.iter().skip(i + 1) {
                let gap = b.committed_at.signed_duration_since(a.committed_at);
                if gap <= Duration::days(i64::from(window_days)) {
                    let churned = std::cmp::min(a.lines_added as u64, b.lines_deleted as u64);
                    if churned > 0 {
                        db::insert_churn_event(
                            conn,
                            repo_path,
                            &a.commit_hash,
                            &b.commit_hash,
                            &a.file_path,
                            churned as i64,
                            i64::from(window_days),
                            &detected_at,
                        )?;
                        churned_lines += churned;
                        churn_event_count += 1;
                    }
                }
            }
        }
    }

    let churn_rate_pct = if total_lines_written == 0 {
        0.0
    } else {
        churned_lines as f64 / total_lines_written as f64 * 100.0
    };

    Ok(ChurnReport {
        repo_path: repo_path.to_string(),
        window_days,
        total_lines_written,
        churned_lines,
        churn_rate_pct,
        commit_count,
        churn_event_count,
    })
}
