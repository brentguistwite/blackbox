use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

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
    /// Canonical repo paths (symlinks resolved for matching with notify events).
    canonical_repos: Vec<PathBuf>,
    /// Original (non-canonical) repo paths, returned to caller.
    original_repos: Vec<PathBuf>,
}

impl RepoWatcher {
    /// Create a watcher monitoring .git/ and .git/refs/heads/ for each repo.
    /// Paths are canonicalized internally so notify events match on macOS
    /// (where /var -> /private/var).
    pub fn new(repos: &[PathBuf]) -> notify::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let mut watcher = notify::recommended_watcher(move |res| {
            let _ = tx.send(res);
        })?;

        let mut canonical_repos = Vec::new();
        let mut original_repos = Vec::new();
        for repo in repos {
            let canon = canonical(repo);
            let git_dir = canon.join(".git");
            let git_refs = git_dir.join("refs").join("heads");

            // Watch .git/ non-recursively (catches HEAD changes)
            // FSEvents on macOS watches directories, not individual files
            if git_dir.is_dir() {
                let _ = watcher.watch(&git_dir, RecursiveMode::NonRecursive);
            }
            // Watch refs/heads/ recursively (branch tip updates, new branches)
            if git_refs.is_dir() {
                let _ = watcher.watch(&git_refs, RecursiveMode::Recursive);
            }
            canonical_repos.push(canon);
            original_repos.push(repo.clone());
        }

        Ok(Self {
            _watcher: watcher,
            rx,
            canonical_repos,
            original_repos,
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
    fn repo_index_for_path(&self, event_path: &Path) -> Option<usize> {
        for ancestor in event_path.ancestors() {
            if ancestor.file_name().map(|n| n == ".git").unwrap_or(false)
                && let Some(repo_root) = ancestor.parent()
            {
                return self.canonical_repos.iter().position(|r| r == repo_root);
            }
        }
        None
    }

    /// Get the original repo paths being watched.
    pub fn repos(&self) -> &[PathBuf] {
        &self.original_repos
    }
}
