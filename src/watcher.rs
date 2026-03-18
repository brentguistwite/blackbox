use std::collections::HashMap;
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

pub struct RepoWatcher {
    _watcher: RecommendedWatcher,
    rx: mpsc::Receiver<notify::Result<notify::Event>>,
    /// Original (non-canonical) repo paths, returned to caller.
    original_repos: Vec<PathBuf>,
    /// Maps watched directory → repo index. Sorted longest-first for specific matching.
    watched_to_idx: Vec<(PathBuf, usize)>,
}

impl RepoWatcher {
    /// Create a watcher monitoring git state for each repo.
    /// Worktrees: watch resolved_gitdir/ only (HEAD-specific, refs shared w/ main repo).
    /// Regular repos: watch .git/ and .git/refs/heads/.
    /// Paths canonicalized internally for macOS /var → /private/var matching.
    pub fn new(repos: &[PathBuf]) -> notify::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let mut watcher = notify::recommended_watcher(move |res| {
            let _ = tx.send(res);
        })?;

        let mut original_repos = Vec::new();
        let mut watched_to_idx = Vec::new();

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
            }
            original_repos.push(repo.clone());
        }

        // Sort longest-first so most specific watched dir matches first
        watched_to_idx.sort_by(|a, b| b.0.as_os_str().len().cmp(&a.0.as_os_str().len()));

        Ok(Self {
            _watcher: watcher,
            rx,
            original_repos,
            watched_to_idx,
        })
    }

    /// Block until events arrive or timeout. Returns debounced list of changed repo paths.
    /// Returns the original (non-canonical) paths the caller passed to `new()`.
    pub fn recv_events(
        &self,
        debounce_map: &mut HashMap<PathBuf, Instant>,
        timeout: Duration,
    ) -> Vec<PathBuf> {
        let mut changed = Vec::new();

        // Block on first event or timeout
        let first = self.rx.recv_timeout(timeout);
        if let Ok(event_result) = first {
            self.process_event(event_result, debounce_map, &mut changed);
        }

        // Drain any additional pending events
        while let Ok(event_result) = self.rx.try_recv() {
            self.process_event(event_result, debounce_map, &mut changed);
        }

        changed
    }

    fn process_event(
        &self,
        event_result: notify::Result<notify::Event>,
        debounce_map: &mut HashMap<PathBuf, Instant>,
        changed: &mut Vec<PathBuf>,
    ) {
        let Ok(event) = event_result else { return };
        let now = Instant::now();
        let debounce = Duration::from_secs(1);

        for path in &event.paths {
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

    /// Get the original repo paths being watched.
    pub fn repos(&self) -> &[PathBuf] {
        &self.original_repos
    }

    /// Get the directories being watched (for testing/debugging).
    pub fn watched_dirs(&self) -> Vec<&Path> {
        self.watched_to_idx.iter().map(|(p, _)| p.as_path()).collect()
    }
}
