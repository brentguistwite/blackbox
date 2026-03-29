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

/// Write heartbeat data to daemon_state after each full_scan.
fn write_heartbeat(conn: &Connection, repo_count: usize) {
    let now = chrono::Utc::now().to_rfc3339();
    if let Err(e) = db::set_daemon_state(conn, "last_poll_at", &now) {
        log::warn!("Failed to write last_poll_at: {}", e);
    }
    if let Err(e) = db::set_daemon_state(conn, "repos_watched", &repo_count.to_string()) {
        log::warn!("Failed to write repos_watched: {}", e);
    }
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
    enrichment::collect_pr_snapshots(&repos, conn);
    claude_tracking::poll_claude_sessions(conn, &repos);
    repos
}

fn maybe_send_daily_notification(config: &Config, conn: &Connection) {
    if !config.notifications_enabled {
        return;
    }
    if !crate::notifications::is_available() {
        return;
    }

    let parts: Vec<&str> = config.notification_time.split(':').collect();
    let (notify_hour, notify_min) = match parts.as_slice() {
        [h, m] => {
            let h: u32 = match h.parse() {
                Ok(v) => v,
                Err(_) => {
                    log::warn!("Invalid notification_time '{}': bad hour", config.notification_time);
                    return;
                }
            };
            let m: u32 = match m.parse() {
                Ok(v) => v,
                Err(_) => {
                    log::warn!("Invalid notification_time '{}': bad minute", config.notification_time);
                    return;
                }
            };
            (h, m)
        }
        _ => {
            log::warn!("Invalid notification_time format '{}': expected HH:MM", config.notification_time);
            return;
        }
    };

    let now_local = chrono::Local::now();
    let now_time = now_local.time();
    let notify_time = match chrono::NaiveTime::from_hms_opt(notify_hour, notify_min, 0) {
        Some(t) => t,
        None => {
            log::warn!("Invalid notification_time '{}': out of range", config.notification_time);
            return;
        }
    };

    if now_time < notify_time {
        return;
    }

    let today_date = now_local.date_naive().to_string();

    match crate::db::notification_was_sent(conn, &today_date, "daily_summary") {
        Ok(true) => return,
        Ok(false) => {}
        Err(e) => {
            log::warn!("Failed to check notification_log: {}", e);
            return;
        }
    }

    let body = match crate::query::daily_summary_for_notification(
        conn,
        config.session_gap_minutes,
        config.first_commit_minutes,
    ) {
        Ok(Some(b)) => b,
        Ok(None) => return,
        Err(e) => {
            log::warn!("Failed to build daily summary for notification: {}", e);
            return;
        }
    };

    if let Err(e) = crate::notifications::send_notification("Blackbox Daily Summary", &body) {
        log::warn!("OS notification failed: {}", e);
    }

    if let Err(e) = crate::db::record_notification_sent(conn, &today_date, "daily_summary") {
        log::warn!("Failed to record notification_sent: {}", e);
    }
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
    write_heartbeat(&conn, repos.len());
    maybe_send_daily_notification(&config, &conn);

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
                    write_heartbeat(&conn, repos.len());
                    maybe_send_daily_notification(&config, &conn);
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
                write_heartbeat(&conn, repos.len());
                maybe_send_daily_notification(&config, &conn);

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
            write_heartbeat(&conn, repos.len());
            maybe_send_daily_notification(&config, &conn);

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
