use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rusqlite::Connection;

use crate::ai_tracking;
use crate::config::{self, Config};
use crate::db;
use crate::enrichment;
use crate::git_ops::{self, RepoState};
use crate::repo_scanner;
use crate::watcher::RepoWatcher;

/// Full-scan interval when watcher is active (30 min).
pub const FULL_SCAN_SECS: u64 = 30 * 60;

/// Ensure a RepoState entry exists for a repo path, resolving worktrees.
/// For worktrees, main_repo_path = resolved main repo root.
/// For regular repos, main_repo_path = repo_path.
fn ensure_state(repo_path: &Path, repo_states: &mut HashMap<PathBuf, RepoState>) {
    repo_states.entry(repo_path.to_path_buf()).or_insert_with(|| {
        let main_repo_path = if repo_scanner::is_worktree(repo_path).is_some() {
            repo_scanner::resolve_main_repo(repo_path).unwrap_or_else(|_| repo_path.to_path_buf())
        } else {
            repo_path.to_path_buf()
        };
        RepoState {
            main_repo_path,
            ..Default::default()
        }
    });
}

/// Outcome of a single poll cycle. Used to write health metrics so `doctor`
/// can detect silent data loss (every repo failing while daemon stays alive).
#[derive(Debug, Default, Clone)]
pub struct PollMetrics {
    pub failed_paths: Vec<PathBuf>,
}

/// Update the running set of repos in a failure state.
/// Pure function — separated from I/O so it's trivially testable and
/// reusable across the full-scan and watcher poll paths.
pub fn record_repo_outcome(
    failed: &mut std::collections::HashSet<PathBuf>,
    repo_path: &std::path::Path,
    was_error: bool,
) {
    if was_error {
        failed.insert(repo_path.to_path_buf());
    } else {
        failed.remove(repo_path);
    }
}

/// Convenience: build PollMetrics from the current failure set.
pub fn metrics_from_set(failed: &std::collections::HashSet<PathBuf>) -> PollMetrics {
    let mut paths: Vec<PathBuf> = failed.iter().cloned().collect();
    paths.sort();
    PollMetrics { failed_paths: paths }
}

/// Outcome of probing the configured watch_dirs for read access. A watch_dir
/// that's unreadable means repos under it never reach `poll_one`, so
/// per-repo failure metrics would silently report "0 failed" — exactly the
/// blind spot Codex flagged. Tracked separately and surfaced as a Required
/// failure in `doctor`.
#[derive(Debug, Default, Clone)]
pub struct DiscoveryMetrics {
    pub failures: Vec<(PathBuf, String)>,
}

/// Verify each configured watch_dir can be opened for directory read.
/// Returns the subset of paths that errored, with the OS error message.
/// Does not mutate the filesystem; safe to call from any context.
pub fn probe_watch_dirs(watch_dirs: &[PathBuf]) -> Vec<(PathBuf, String)> {
    watch_dirs
        .iter()
        .filter_map(|d| match std::fs::read_dir(d) {
            Ok(_) => None,
            Err(e) => Some((d.clone(), e.to_string())),
        })
        .collect()
}

/// Persist discovery health to daemon_state. Always writes both keys so a
/// recovery (errors → no errors) clears stale state — same contract as
/// `write_poll_metrics`.
pub fn write_discovery_metrics(
    conn: &Connection,
    metrics: &DiscoveryMetrics,
) -> anyhow::Result<()> {
    let count = metrics.failures.len();
    db::set_daemon_state(conn, "last_poll_discovery_failed", &count.to_string())?;
    let sample = metrics
        .failures
        .iter()
        .take(FAILED_SAMPLE_LIMIT)
        .map(|(p, _)| p.display().to_string())
        .collect::<Vec<_>>()
        .join("\n");
    db::set_daemon_state(conn, "last_poll_discovery_failed_sample", &sample)?;
    Ok(())
}

/// Maximum number of failed paths to persist as a sample. Bounds the
/// daemon_state row size; the failed *count* is always exact.
const FAILED_SAMPLE_LIMIT: usize = 5;

/// Persist poll-cycle health metrics to daemon_state.
/// Always writes both keys (count + sample) so a recovery clears stale failures.
pub fn write_poll_metrics(conn: &Connection, metrics: &PollMetrics) -> anyhow::Result<()> {
    let count = metrics.failed_paths.len();
    db::set_daemon_state(conn, "last_poll_repos_failed", &count.to_string())?;
    let sample = metrics
        .failed_paths
        .iter()
        .take(FAILED_SAMPLE_LIMIT)
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join("\n");
    db::set_daemon_state(conn, "last_poll_failed_sample", &sample)?;
    Ok(())
}

/// Poll all repos for git activity. Replaces the failure set with this cycle's
/// outcomes — full_scan covers every watched repo so it is authoritative.
fn poll_all_repos(
    repos: &[PathBuf],
    repo_states: &mut HashMap<PathBuf, RepoState>,
    conn: &Connection,
    failed_set: &mut std::collections::HashSet<PathBuf>,
) {
    failed_set.clear();
    for repo_path in repos {
        let was_error = poll_one(repo_path, repo_states, conn);
        record_repo_outcome(failed_set, repo_path, was_error);
    }
}

/// Poll a single repo. Returns true on error (caller decides what to log/track).
/// Used by both full_scan and the watcher event path so failure tracking stays
/// consistent across entry points.
pub fn poll_one(
    repo_path: &Path,
    repo_states: &mut HashMap<PathBuf, RepoState>,
    conn: &Connection,
) -> bool {
    ensure_state(repo_path, repo_states);
    let state = repo_states.get_mut(repo_path).unwrap();
    let db_repo_path = state.main_repo_path.to_string_lossy().to_string();
    match git_ops::poll_repo(repo_path, &db_repo_path, state, conn) {
        Ok(()) => false,
        Err(e) => {
            log::warn!("Error polling {}: {}", repo_path.display(), e);
            true
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
    failed_set: &mut std::collections::HashSet<PathBuf>,
) -> Vec<PathBuf> {
    // Probe top-level watch_dirs for read access BEFORE discovery so a TCC
    // denial there can't silently shrink the discovered repo set to 0.
    let discovery_failures = probe_watch_dirs(&config.watch_dirs);
    for (path, err) in &discovery_failures {
        log::warn!("Cannot read watch_dir {}: {}", path.display(), err);
    }
    let discovery_metrics = DiscoveryMetrics { failures: discovery_failures };
    if let Err(e) = write_discovery_metrics(conn, &discovery_metrics) {
        log::warn!("Failed to write discovery metrics: {}", e);
    }

    let repos = repo_scanner::discover_repos(&config.watch_dirs, config.worktree_dir_name.as_deref());
    poll_all_repos(&repos, repo_states, conn, failed_set);
    if let Err(e) = write_poll_metrics(conn, &metrics_from_set(failed_set)) {
        log::warn!("Failed to write poll metrics: {}", e);
    }
    enrichment::collect_reviews(&repos, conn);
    enrichment::collect_pr_snapshots(&repos, conn);
    ai_tracking::poll_all_ai_sessions(conn, &repos);
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
    // Track per-repo failure state across full_scan and watcher events so
    // a watcher-driven recovery clears stale failures and a watcher-driven
    // failure shows up in `doctor` immediately, not 30 minutes later.
    let mut failed_set: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    // Initial full scan
    let mut repos = full_scan(&config, &mut repo_states, &conn, &mut failed_set);
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
                    repos = full_scan(&config, &mut repo_states, &conn, &mut failed_set);
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
            let mut metrics_dirty = false;

            for repo_path in &events.changed_repos {
                log::info!("Detected change in {}", repo_path.display());
                let was_error = poll_one(repo_path, &mut repo_states, &conn);
                record_repo_outcome(&mut failed_set, repo_path, was_error);
                metrics_dirty = true;
            }

            // Handle newly-discovered worktrees
            for wt_path in &events.new_worktrees {
                log::info!("New worktree detected: {}", wt_path.display());
                let was_error = poll_one(wt_path, &mut repo_states, &conn);
                record_repo_outcome(&mut failed_set, wt_path, was_error);
                watcher.watch_repo(wt_path);
                metrics_dirty = true;
            }

            // Clean up stale worktrees
            let stale = remove_stale_worktrees(&mut repo_states);
            for path in &stale {
                if failed_set.remove(path) {
                    metrics_dirty = true;
                }
            }

            if metrics_dirty {
                if let Err(e) = write_poll_metrics(&conn, &metrics_from_set(&failed_set)) {
                    log::warn!("Failed to write poll metrics (watcher path): {}", e);
                }
                // Bump heartbeat so doctor sees this as proof of liveness, not
                // as a stalled idle-watcher between full scans.
                write_heartbeat(&conn, repos.len());
            }

            // Periodic full scan for missed events + new repos
            if last_full_scan.elapsed() >= Duration::from_secs(FULL_SCAN_SECS) {
                repos = full_scan(&config, &mut repo_states, &conn, &mut failed_set);
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
            repos = full_scan(&config, &mut repo_states, &conn, &mut failed_set);
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn setup_db() -> (Connection, NamedTempFile) {
        let tmp = NamedTempFile::new().unwrap();
        let conn = crate::db::open_db(tmp.path()).unwrap();
        (conn, tmp)
    }

    fn test_config(enabled: bool, time: &str) -> Config {
        Config {
            notifications_enabled: enabled,
            notification_time: time.to_string(),
            ..Config::default()
        }
    }

    #[test]
    fn notification_disabled_no_db_write() {
        let (conn, _tmp) = setup_db();
        let config = test_config(false, "00:01");
        maybe_send_daily_notification(&config, &conn);
        let today = chrono::Local::now().date_naive().to_string();
        let sent = crate::db::notification_was_sent(&conn, &today, "daily_summary").unwrap();
        assert!(!sent);
    }

    #[test]
    fn notification_future_time_no_db_write() {
        let (conn, _tmp) = setup_db();
        let config = test_config(true, "23:59");
        maybe_send_daily_notification(&config, &conn);
        let today = chrono::Local::now().date_naive().to_string();
        let sent = crate::db::notification_was_sent(&conn, &today, "daily_summary").unwrap();
        assert!(!sent);
    }

    #[test]
    fn notification_past_time_no_activity_no_db_write() {
        let (conn, _tmp) = setup_db();
        let config = test_config(true, "00:01");
        maybe_send_daily_notification(&config, &conn);
        let today = chrono::Local::now().date_naive().to_string();
        let sent = crate::db::notification_was_sent(&conn, &today, "daily_summary").unwrap();
        assert!(!sent);
    }

    #[test]
    fn notification_already_sent_idempotent() {
        let (conn, _tmp) = setup_db();
        let today = chrono::Local::now().date_naive().to_string();
        crate::db::record_notification_sent(&conn, &today, "daily_summary").unwrap();

        let config = test_config(true, "00:01");
        maybe_send_daily_notification(&config, &conn);

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM notification_log WHERE date = ?1",
                rusqlite::params![today],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }
}
