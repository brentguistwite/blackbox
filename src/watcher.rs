use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use crate::repo_scanner;

/// Canonicalize a path, falling back to the original if resolution fails.
fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Maps a filesystem event path back to its repo root by finding .git ancestor.
pub fn path_to_repo(event_path: &Path, watched_repos: &[PathBuf]) -> Option<PathBuf> {
    for ancestor in event_path.ancestors() {
        if ancestor.file_name().map(|n| n == ".git").unwrap_or(false)
            && let Some(repo_root) = ancestor.parent()
        {
            let repo_root = repo_root.to_path_buf();
            if watched_repos.contains(&repo_root) {
                return Some(repo_root);
            }
        }
    }
    None
}

/// Events returned by `recv_events`.
pub struct WatcherEvents {
    /// Repos with git state changes (existing behavior).
    pub changed_repos: Vec<PathBuf>,
    /// New worktree directories detected in watched worktree parent dirs.
    pub new_worktrees: Vec<PathBuf>,
}

pub struct RepoWatcher {
    watcher: RecommendedWatcher,
    rx: mpsc::Receiver<notify::Result<notify::Event>>,
    /// Original (non-canonical) repo paths, returned to caller.
    original_repos: Vec<PathBuf>,
    /// Maps watched directory -> repo index. Sorted longest-first for specific matching.
    watched_to_idx: Vec<(PathBuf, usize)>,
    /// Canonical paths of watched worktree parent dirs (e.g. <repo>/.worktrees/).
    /// Events from these dirs are treated as new-worktree signals, not git state changes.
    worktree_parent_dirs: HashSet<PathBuf>,
    /// The worktree dir name (e.g. ".worktrees"), stored for watch_repo.
    worktree_dir_name: Option<String>,
}

impl RepoWatcher {
    /// Create a watcher monitoring git state for each repo.
    /// Worktrees: watch resolved_gitdir/ only (HEAD-specific, refs shared w/ main repo).
    /// Regular repos: watch .git/ and .git/refs/heads/.
    /// If `worktree_dir_name` is Some, also watches `<repo>/<name>/` for new worktree creation.
    /// Paths canonicalized internally for macOS /var -> /private/var matching.
    pub fn new(repos: &[PathBuf], worktree_dir_name: Option<&str>) -> notify::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let mut watcher = notify::recommended_watcher(move |res| {
            let _ = tx.send(res);
        })?;

        let mut original_repos = Vec::new();
        let mut watched_to_idx = Vec::new();
        let mut worktree_parent_dirs = HashSet::new();

        for (idx, repo) in repos.iter().enumerate() {
            let canon = canonical(repo);

            if let Some(resolved_gitdir) = repo_scanner::is_worktree(&canon) {
                // Worktree: watch only the worktree-specific HEAD directory.
                // refs/ are shared with main repo and watched via main repo's entry.
                if resolved_gitdir.is_dir() {
                    let _ = watcher.watch(&resolved_gitdir, RecursiveMode::NonRecursive);
                    watched_to_idx.push((resolved_gitdir, idx));
                }
            } else {
                // Regular repo: watch .git/ and .git/refs/heads/
                let git_dir = canon.join(".git");
                let git_refs = git_dir.join("refs").join("heads");
                if git_dir.is_dir() {
                    let _ = watcher.watch(&git_dir, RecursiveMode::NonRecursive);
                    watched_to_idx.push((git_dir, idx));
                }
                if git_refs.is_dir() {
                    let _ = watcher.watch(&git_refs, RecursiveMode::Recursive);
                    watched_to_idx.push((git_refs, idx));
                }

                // Watch worktree parent dir if configured and exists
                if let Some(name) = worktree_dir_name {
                    let wt_parent = canon.join(name);
                    if wt_parent.is_dir() {
                        let _ = watcher.watch(&wt_parent, RecursiveMode::NonRecursive);
                        worktree_parent_dirs.insert(wt_parent);
                    }
                }
            }
            original_repos.push(repo.clone());
        }

        // Sort longest-first so most specific watched dir matches first
        watched_to_idx.sort_by(|a, b| b.0.as_os_str().len().cmp(&a.0.as_os_str().len()));

        Ok(Self {
            watcher,
            rx,
            original_repos,
            watched_to_idx,
            worktree_parent_dirs,
            worktree_dir_name: worktree_dir_name.map(|s| s.to_string()),
        })
    }

    /// Block until events arrive or timeout. Returns debounced changed repos and new worktrees.
    /// Returns the original (non-canonical) paths the caller passed to `new()`.
    pub fn recv_events(
        &self,
        debounce_map: &mut HashMap<PathBuf, Instant>,
        timeout: Duration,
    ) -> WatcherEvents {
        let mut changed = Vec::new();
        let mut new_worktrees = Vec::new();

        // Block on first event or timeout
        let first = self.rx.recv_timeout(timeout);
        if let Ok(event_result) = first {
            self.process_event(event_result, debounce_map, &mut changed, &mut new_worktrees);
        }

        // Drain any additional pending events
        while let Ok(event_result) = self.rx.try_recv() {
            self.process_event(event_result, debounce_map, &mut changed, &mut new_worktrees);
        }

        WatcherEvents {
            changed_repos: changed,
            new_worktrees,
        }
    }

    #[allow(clippy::collapsible_if)]
    fn process_event(
        &self,
        event_result: notify::Result<notify::Event>,
        debounce_map: &mut HashMap<PathBuf, Instant>,
        changed: &mut Vec<PathBuf>,
        new_worktrees: &mut Vec<PathBuf>,
    ) {
        let Ok(event) = event_result else { return };
        let now = Instant::now();
        let debounce = Duration::from_secs(1);

        for path in &event.paths {
            // Check if this event is from a worktree parent dir
            if self.is_worktree_parent_event(path) {
                // New entry in worktree parent dir — check if it's a valid worktree
                if path.is_dir() && repo_scanner::is_worktree(path).is_some() {
                    if !new_worktrees.contains(path) {
                        new_worktrees.push(path.clone());
                    }
                }
                continue;
            }

            if let Some(idx) = self.repo_index_for_path(path) {
                let original = &self.original_repos[idx];
                let should_poll = debounce_map
                    .get(original)
                    .is_none_or(|t| now.duration_since(*t) >= debounce);
                if should_poll {
                    debounce_map.insert(original.clone(), now);
                    if !changed.contains(original) {
                        changed.push(original.clone());
                    }
                }
            }
        }
    }

    /// Check if an event path is inside one of the watched worktree parent dirs.
    fn is_worktree_parent_event(&self, event_path: &Path) -> bool {
        for wt_dir in &self.worktree_parent_dirs {
            if event_path.starts_with(wt_dir) && event_path != wt_dir.as_path() {
                return true;
            }
        }
        false
    }

    /// Find which watched repo an event path belongs to.
    /// Uses watched_to_idx (sorted longest-first) for both regular repos and worktrees.
    fn repo_index_for_path(&self, event_path: &Path) -> Option<usize> {
        for (watched_dir, idx) in &self.watched_to_idx {
            if event_path.starts_with(watched_dir) {
                return Some(*idx);
            }
        }
        None
    }

    /// Dynamically add a repo to the watcher.
    /// Watches its git dirs (same logic as `new()` for regular repos vs worktrees).
    pub fn watch_repo(&mut self, repo: &Path) {
        let idx = self.original_repos.len();
        let canon = canonical(repo);

        if let Some(resolved_gitdir) = repo_scanner::is_worktree(&canon) {
            if resolved_gitdir.is_dir() {
                let _ = self.watcher.watch(&resolved_gitdir, RecursiveMode::NonRecursive);
                self.watched_to_idx.push((resolved_gitdir, idx));
            }
        } else {
            let git_dir = canon.join(".git");
            let git_refs = git_dir.join("refs").join("heads");
            if git_dir.is_dir() {
                let _ = self.watcher.watch(&git_dir, RecursiveMode::NonRecursive);
                self.watched_to_idx.push((git_dir, idx));
            }
            if git_refs.is_dir() {
                let _ = self.watcher.watch(&git_refs, RecursiveMode::Recursive);
                self.watched_to_idx.push((git_refs, idx));
            }

            if let Some(ref name) = self.worktree_dir_name {
                let wt_parent = canon.join(name);
                if wt_parent.is_dir() {
                    let _ = self.watcher.watch(&wt_parent, RecursiveMode::NonRecursive);
                    self.worktree_parent_dirs.insert(wt_parent);
                }
            }
        }

        self.original_repos.push(repo.to_path_buf());

        // Re-sort longest-first
        self.watched_to_idx
            .sort_by(|a, b| b.0.as_os_str().len().cmp(&a.0.as_os_str().len()));
    }

    /// Get the original repo paths being watched.
    pub fn repos(&self) -> &[PathBuf] {
        &self.original_repos
    }

    /// Get the directories being watched (for testing/debugging).
    pub fn watched_dirs(&self) -> Vec<&Path> {
        let mut dirs: Vec<&Path> = self.watched_to_idx.iter().map(|(p, _)| p.as_path()).collect();
        for wt_dir in &self.worktree_parent_dirs {
            dirs.push(wt_dir.as_path());
        }
        dirs
    }
}
