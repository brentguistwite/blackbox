use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rusqlite::Connection;

use crate::claude_tracking;
use crate::config::{self, Config};
use crate::db;
use crate::enrichment;
use crate::git_ops::{self, RepoState};
use crate::repo_scanner;
use crate::watcher::RepoWatcher;

/// Full-scan interval when watcher is active (30 min).
const FULL_SCAN_SECS: u64 = 30 * 60;

/// Ensure a RepoState entry exists for a repo path, resolving worktrees.
/// For worktrees, main_repo_path = resolved main repo root.
/// For regular repos, main_repo_path = repo_path.
#[allow(clippy::ptr_arg)]
fn ensure_state(repo_path: &PathBuf, repo_states: &mut HashMap<PathBuf, RepoState>) {
    repo_states.entry(repo_path.clone()).or_insert_with(|| {
        let main_repo_path = if repo_scanner::is_worktree(repo_path).is_some() {
            repo_scanner::resolve_main_repo(repo_path).unwrap_or_else(|_| repo_path.clone())
        } else {
            repo_path.clone()
        };
        RepoState {
            main_repo_path,
            ..Default::default()
        }
    });
}

/// Poll all repos for git activity.
fn poll_all_repos(
    repos: &[PathBuf],
    repo_states: &mut HashMap<PathBuf, RepoState>,
    conn: &Connection,
) {
    for repo_path in repos {
        ensure_state(repo_path, repo_states);
        let state = repo_states.get_mut(repo_path).unwrap();
        let db_repo_path = state.main_repo_path.to_string_lossy().to_string();
        if let Err(e) = git_ops::poll_repo(repo_path, &db_repo_path, state, conn) {
            log::warn!("Error polling {}: {}", repo_path.display(), e);
        }
    }
}

/// Remove stale worktree entries from repo_states.
/// A worktree is stale if is_worktree() returns None (deleted .git file) or
/// the resolved gitdir HEAD no longer exists.
pub fn remove_stale_worktrees(repo_states: &mut HashMap<PathBuf, RepoState>) -> Vec<PathBuf> {
    let stale: Vec<PathBuf> = repo_states
        .iter()
        .filter(|(path, state)| {
            // Only check worktrees (main_repo_path != scanned path)
            state.main_repo_path != **path && repo_scanner::is_worktree(path).is_none()
        })
        .map(|(path, _)| path.clone())
        .collect();
    for path in &stale {
        log::warn!("Stale worktree removed: {}", path.display());
        repo_states.remove(path);
    }
    stale
}

/// Full scan: re-discover repos, poll all, collect reviews, track sessions.
fn full_scan(
    config: &Config,
    repo_states: &mut HashMap<PathBuf, RepoState>,
    conn: &Connection,
) -> Vec<PathBuf> {
    let repos = repo_scanner::discover_repos(&config.watch_dirs, config.worktree_dir_name.as_deref());
    poll_all_repos(&repos, repo_states, conn);
    enrichment::collect_reviews(&repos, conn);
    claude_tracking::poll_claude_sessions(conn, &repos);
    repos
}

pub fn run_poll_loop(mut config: Config) -> anyhow::Result<()> {
    // Register SIGHUP handler — sets atomic flag, checked each loop iteration
    let reload_requested = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGHUP, Arc::clone(&reload_requested))?;

    let db_path = config::data_dir()?.join("blackbox.db");
    let conn = db::open_db(&db_path)?;
    let mut repo_states: HashMap<PathBuf, RepoState> = HashMap::new();
    let mut debounce_map: HashMap<PathBuf, Instant> = HashMap::new();
    // Initial full scan
    let mut repos = full_scan(&config, &mut repo_states, &conn);

    // Try to set up filesystem watcher
    let mut watcher_opt = RepoWatcher::new(&repos, config.worktree_dir_name.as_deref()).ok();
    if watcher_opt.is_some() {
        log::info!("Watching {} repos for changes", repos.len());
    } else {
        log::warn!("File watcher unavailable, falling back to polling");
    }

    let mut last_full_scan = Instant::now();

    loop {
        // Check for SIGHUP reload request between poll cycles
        if reload_requested.swap(false, Ordering::Relaxed) {
            log::info!("SIGHUP received, reloading config");
            match config::reload_config() {
                Ok(new_cfg) => {
                    if new_cfg.watch_dirs != config.watch_dirs {
                        log::info!("watch_dirs: {:?} -> {:?}", config.watch_dirs, new_cfg.watch_dirs);
                    }
                    if new_cfg.poll_interval_secs != config.poll_interval_secs {
                        log::info!("poll_interval_secs: {} -> {}", config.poll_interval_secs, new_cfg.poll_interval_secs);
                    }
                    config = new_cfg;
                    log::info!("Config reloaded successfully");
                    // Re-discover repos and recreate watcher with new config
                    repos = full_scan(&config, &mut repo_states, &conn);
                    watcher_opt = RepoWatcher::new(&repos, config.worktree_dir_name.as_deref()).ok();
                    last_full_scan = Instant::now();
                    debounce_map.clear();
                }
                Err(e) => log::warn!("Config reload failed: {e}, keeping previous config"),
            }
        }

        if let Some(ref mut watcher) = watcher_opt {
            // Hybrid mode: block until event or 1s timeout
            let events = watcher.recv_events(&mut debounce_map, Duration::from_secs(1));

            for repo_path in &events.changed_repos {
                log::info!("Detected change in {}", repo_path.display());
                ensure_state(repo_path, &mut repo_states);
                let state = repo_states.get_mut(repo_path).unwrap();
                let db_repo_path = state.main_repo_path.to_string_lossy().to_string();
                if let Err(e) = git_ops::poll_repo(repo_path, &db_repo_path, state, &conn) {
                    log::warn!("Error polling {}: {}", repo_path.display(), e);
                }
            }

            // Handle newly-discovered worktrees
            for wt_path in &events.new_worktrees {
                log::info!("New worktree detected: {}", wt_path.display());
                ensure_state(wt_path, &mut repo_states);
                let state = repo_states.get_mut(wt_path).unwrap();
                let db_repo_path = state.main_repo_path.to_string_lossy().to_string();
                if let Err(e) = git_ops::poll_repo(wt_path, &db_repo_path, state, &conn) {
                    log::warn!("Error polling new worktree {}: {}", wt_path.display(), e);
                }
                watcher.watch_repo(wt_path);
            }

            // Clean up stale worktrees
            remove_stale_worktrees(&mut repo_states);

            // Periodic full scan for missed events + new repos
            if last_full_scan.elapsed() >= Duration::from_secs(FULL_SCAN_SECS) {
                repos = full_scan(&config, &mut repo_states, &conn);

                // Recreate watcher with updated repo list
                watcher_opt = RepoWatcher::new(&repos, config.worktree_dir_name.as_deref()).ok();
                if let Some(ref _w) = watcher_opt {
                    log::info!("Watching {} repos for changes", repos.len());
                }
                last_full_scan = Instant::now();
                debounce_map.clear();
            }
        } else {
            // Pure polling fallback (original behavior)
            std::thread::sleep(Duration::from_secs(config.poll_interval_secs));
            repos = full_scan(&config, &mut repo_states, &conn);

            // Retry watcher setup on each full scan
            watcher_opt = RepoWatcher::new(&repos, config.worktree_dir_name.as_deref()).ok();
            if watcher_opt.is_some() {
                log::info!(
                    "File watcher now available, watching {} repos",
                    repos.len()
                );
                last_full_scan = Instant::now();
            }
        }
    }
}
