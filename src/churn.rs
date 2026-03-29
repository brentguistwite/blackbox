use anyhow::Result;
use git2::{Commit, DiffOptions, Repository};
use std::collections::HashMap;

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
