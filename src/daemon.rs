use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::output::OutputFormat;
use crate::poller;

#[derive(Debug, PartialEq, Eq, serde::Serialize)]
pub enum HealthIndicator {
    Green,
    Yellow,
    Red,
}

#[derive(Debug, serde::Serialize)]
pub struct DaemonStatus {
    pub running: bool,
    pub pid: Option<u32>,
    pub uptime_secs: Option<u64>,
    pub last_poll_at: Option<chrono::DateTime<chrono::Utc>>,
    pub repos_watched: Option<u64>,
    pub repos_failed_last_poll: Option<u64>,
    pub failed_sample: Vec<String>,
    pub db_size_bytes: Option<u64>,
    pub events_today: Option<u64>,
    pub health: HealthIndicator,
}

pub fn pid_file_path(data_dir: &Path) -> PathBuf {
    data_dir.join("blackbox.pid")
}

pub fn is_daemon_running(data_dir: &Path) -> anyhow::Result<Option<u32>> {
    let path = pid_file_path(data_dir);
    if !path.exists() {
        return Ok(None);
    }
    let pid_str = std::fs::read_to_string(&path)?;
    let pid: u32 = pid_str.trim().parse()?;
    match nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), None) {
        Ok(_) => Ok(Some(pid)),
        Err(_) => {
            // Stale PID file -- process is dead, clean up
            std::fs::remove_file(&path)?;
            Ok(None)
        }
    }
}

pub fn start_daemon(config: Config, data_dir: &Path) -> anyhow::Result<()> {
    if let Some(pid) = is_daemon_running(data_dir)? {
        anyhow::bail!("Daemon already running (PID {})", pid);
    }

    let pid_path = pid_file_path(data_dir);
    std::fs::create_dir_all(data_dir)?;

    let log_path = data_dir.join("blackbox.log");

    let daemonize = daemonize::Daemonize::new()
        .pid_file(&pid_path)
        .working_directory("/")
        .stdout(std::fs::File::create(&log_path)?)
        .stderr(std::fs::File::create(data_dir.join("blackbox.err.log"))?);

    match daemonize.start() {
        Ok(()) => {
            // We're in the child (daemon) process now
            env_logger::Builder::from_default_env()
                .filter_level(log::LevelFilter::Info)
                .init();
            log::info!("Daemon started (PID {})", std::process::id());
            if let Err(e) = poller::run_poll_loop(config) {
                log::error!("Poll loop error: {}", e);
            }
            Ok(())
        }
        Err(e) => {
            anyhow::bail!("Failed to daemonize: {}", e);
        }
    }
}

pub fn stop_daemon(data_dir: &Path) -> anyhow::Result<()> {
    match is_daemon_running(data_dir)? {
        Some(pid) => {
            nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid as i32),
                nix::sys::signal::Signal::SIGTERM,
            )?;
            // Remove PID file
            let path = pid_file_path(data_dir);
            if path.exists() {
                std::fs::remove_file(&path)?;
            }
            println!("Daemon stopped (PID {})", pid);
        }
        None => {
            println!("Daemon not running");
        }
    }
    Ok(())
}

/// RAII guard that writes a PID file on creation and removes it on drop.
pub struct PidGuard {
    path: PathBuf,
}

impl PidGuard {
    pub fn new(data_dir: &Path) -> anyhow::Result<Self> {
        std::fs::create_dir_all(data_dir)?;
        let path = pid_file_path(data_dir);
        std::fs::write(&path, std::process::id().to_string())?;
        Ok(Self { path })
    }
}

impl Drop for PidGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub fn run_foreground(config: Config, data_dir: &Path) -> anyhow::Result<()> {
    let _pid_guard = PidGuard::new(data_dir)?;
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .init();
    log::info!("Running in foreground (PID {})", std::process::id());
    poller::run_poll_loop(config)
}

pub fn reload_daemon(data_dir: &Path) -> anyhow::Result<()> {
    match is_daemon_running(data_dir)? {
        Some(pid) => {
            nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid as i32),
                nix::sys::signal::Signal::SIGHUP,
            )?;
            println!("Reloading config (PID {})", pid);
        }
        None => println!("Daemon not running"),
    }
    Ok(())
}

pub fn get_daemon_status(data_dir: &Path) -> anyhow::Result<DaemonStatus> {
    let pid = is_daemon_running(data_dir)?;
    let running = pid.is_some();

    let uptime_secs = if running {
        let pid_path = pid_file_path(data_dir);
        std::fs::metadata(&pid_path)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.elapsed().ok())
            .map(|d| d.as_secs())
    } else {
        None
    };

    let db_path = data_dir.join("blackbox.db");
    let db_size_bytes = std::fs::metadata(&db_path).ok().map(|m| m.len());

    let (last_poll_at, repos_watched, repos_failed_last_poll, failed_sample, events_today) =
        if db_path.exists() {
            match crate::db::open_db(&db_path) {
                Ok(conn) => {
                    let lp = crate::db::get_daemon_state(&conn, "last_poll_at")
                        .ok()
                        .flatten()
                        .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                        .map(|dt| dt.with_timezone(&chrono::Utc));
                    let rw = crate::db::get_daemon_state(&conn, "repos_watched")
                        .ok()
                        .flatten()
                        .and_then(|s| s.parse::<u64>().ok());
                    let rf = crate::db::get_daemon_state(&conn, "last_poll_repos_failed")
                        .ok()
                        .flatten()
                        .and_then(|s| s.parse::<u64>().ok());
                    let fs: Vec<String> = crate::db::get_daemon_state(&conn, "last_poll_failed_sample")
                        .ok()
                        .flatten()
                        .map(|s| s.lines().filter(|l| !l.is_empty()).map(String::from).collect())
                        .unwrap_or_default();
                    let et = crate::db::count_events_today(&conn).ok();
                    (lp, rw, rf, fs, et)
                }
                Err(_) => (None, None, None, Vec::new(), None),
            }
        } else {
            (None, None, None, Vec::new(), None)
        };

    let health = compute_health(
        running,
        last_poll_at,
        repos_watched.unwrap_or(0),
        repos_failed_last_poll,
    );

    Ok(DaemonStatus {
        running,
        pid,
        uptime_secs,
        last_poll_at,
        repos_watched,
        repos_failed_last_poll,
        failed_sample,
        db_size_bytes,
        events_today,
        health,
    })
}

fn compute_health(
    running: bool,
    last_poll_at: Option<chrono::DateTime<chrono::Utc>>,
    repos_watched: u64,
    repos_failed: Option<u64>,
) -> HealthIndicator {
    if !running {
        return HealthIndicator::Red;
    }
    let base = match last_poll_at {
        None => HealthIndicator::Yellow,
        Some(t) => {
            let age = chrono::Utc::now().signed_duration_since(t);
            if age <= chrono::Duration::minutes(5) {
                HealthIndicator::Green
            } else if age <= chrono::Duration::minutes(30) {
                HealthIndicator::Yellow
            } else {
                HealthIndicator::Red
            }
        }
    };
    // Daemon predates the failure metric — we cannot say it's healthy. Cap at
    // Yellow so users see "unknown" rather than a misleading green.
    let failed = match repos_failed {
        None => {
            // Cap Green at Yellow when we don't have a failure metric to trust.
            return match base {
                HealthIndicator::Green => HealthIndicator::Yellow,
                other => other,
            };
        }
        Some(n) => n,
    };
    // Process alive + polling, but every repo errored — silent data loss.
    if repos_watched > 0 && failed >= repos_watched {
        return HealthIndicator::Red;
    }
    // Partial poll failures degrade Green to Yellow.
    if failed > 0 && base == HealthIndicator::Green {
        return HealthIndicator::Yellow;
    }
    base
}

fn render_status_pretty(status: &DaemonStatus) {
    use colored::Colorize;
    let (icon, label) = match status.health {
        HealthIndicator::Green => ("\u{2713}".green().bold(), "Running".green().bold()),
        HealthIndicator::Yellow => {
            // Distinguish "stale poll", "missing metric", "all-running-fine-but-failures"
            // so users know which corrective action applies.
            let text = if !status.running {
                "Stopped"
            } else if status.repos_failed_last_poll.is_none() {
                "Running (metrics unknown — restart daemon to enable)"
            } else if status.repos_failed_last_poll.unwrap_or(0) > 0 {
                "Running (poll failures)"
            } else {
                "Running (stale)"
            };
            ("\u{26a0}".yellow().bold(), text.yellow().bold())
        }
        HealthIndicator::Red => {
            let text = if !status.running {
                "Stopped"
            } else if status.repos_watched.unwrap_or(0) > 0
                && status.repos_failed_last_poll.unwrap_or(0) >= status.repos_watched.unwrap_or(0)
            {
                "Running (all polls failing)"
            } else {
                "Stopped"
            };
            ("\u{2717}".red().bold(), text.red().bold())
        }
    };
    println!("{} {}", icon, label);
    if let Some(pid) = status.pid {
        println!("  PID:           {}", pid);
    }
    if let Some(secs) = status.uptime_secs {
        println!("  Uptime:        {}", format_uptime(secs));
    }
    match status.last_poll_at {
        Some(t) => {
            let age = chrono::Utc::now().signed_duration_since(t);
            println!("  Last poll:     {} ago", format_duration_ago(age));
        }
        None => println!("  Last poll:     never"),
    }
    match status.repos_watched {
        Some(n) => println!("  Repos watched: {}", n),
        None => println!("  Repos watched: unknown"),
    }
    if let Some(failed) = status.repos_failed_last_poll {
        if failed > 0 {
            let total = status.repos_watched.unwrap_or(0).max(failed);
            let suffix = if status.failed_sample.is_empty() {
                String::new()
            } else {
                format!(" — {}", status.failed_sample.join(", "))
            };
            let line = format!("  Poll failures: {failed} of {total}{suffix}");
            println!("{}", line.yellow());
        }
    }
    match status.db_size_bytes {
        Some(b) => println!("  DB size:       {:.1} KB", b as f64 / 1024.0),
        None => println!("  DB size:       no DB yet"),
    }
    println!(
        "  Events today:  {}",
        status.events_today.unwrap_or(0)
    );
}

fn format_uptime(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    match (h, m) {
        (0, 0) => format!("{}s", s),
        (0, _) => format!("{}m {}s", m, s),
        _ => format!("{}h {}m", h, m),
    }
}

fn format_duration_ago(d: chrono::Duration) -> String {
    let total_secs = d.num_seconds().max(0);
    let mins = total_secs / 60;
    let hours = mins / 60;
    if hours > 0 {
        format!("{}h {}m", hours, mins % 60)
    } else if mins > 0 {
        format!("{}m", mins)
    } else {
        format!("{}s", total_secs)
    }
}

pub fn daemon_status(data_dir: &Path, format: OutputFormat) -> anyhow::Result<()> {
    let status = get_daemon_status(data_dir)?;
    match format {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&status)?),
        _ => render_status_pretty(&status),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_health_red_when_all_repos_failing() {
        // Daemon alive + recent poll, but every repo errored on the last cycle.
        // Status must not show green — that's silent data loss.
        let h = compute_health(true, Some(chrono::Utc::now()), 5, Some(5));
        assert!(matches!(h, HealthIndicator::Red), "expected Red, got {:?}", h);
    }

    #[test]
    fn compute_health_yellow_on_partial_failures() {
        let h = compute_health(true, Some(chrono::Utc::now()), 5, Some(2));
        assert!(matches!(h, HealthIndicator::Yellow), "expected Yellow, got {:?}", h);
    }

    #[test]
    fn compute_health_green_when_no_failures_and_recent_poll() {
        let h = compute_health(true, Some(chrono::Utc::now()), 5, Some(0));
        assert!(matches!(h, HealthIndicator::Green), "expected Green, got {:?}", h);
    }

    #[test]
    fn compute_health_red_when_not_running_regardless_of_failures() {
        // Daemon dead trumps everything else.
        let h = compute_health(false, Some(chrono::Utc::now()), 5, Some(0));
        assert!(matches!(h, HealthIndicator::Red));
    }

    #[test]
    fn compute_health_legacy_daemon_recent_poll_caps_at_yellow() {
        // Codex regression: missing failure metric must NOT silently render
        // as Green. Pre-fix, compute_health collapsed None → 0 and returned
        // Green for a recent poll. Now it must stay Yellow ("unknown") so
        // users in the rollout window see "needs daemon restart".
        let h = compute_health(true, Some(chrono::Utc::now()), 5, None);
        assert_eq!(h, HealthIndicator::Yellow,
            "missing failure metric should cap health at Yellow, got {:?}", h);
    }

    #[test]
    fn compute_health_legacy_daemon_old_poll_stays_red() {
        // If the legacy daemon's last poll is also stale, severity should not
        // be downgraded — old poll Red trumps the unknown-metric Yellow cap.
        let stale = chrono::Utc::now() - chrono::Duration::hours(2);
        let h = compute_health(true, Some(stale), 5, None);
        assert_eq!(h, HealthIndicator::Red);
    }

    #[test]
    fn pid_guard_writes_pid_file_on_creation() {
        let dir = tempfile::tempdir().unwrap();
        let _guard = PidGuard::new(dir.path()).unwrap();
        let pid_path = pid_file_path(dir.path());
        assert!(pid_path.exists(), "PID file should exist after guard creation");
        let content = std::fs::read_to_string(&pid_path).unwrap();
        let pid: u32 = content.trim().parse().unwrap();
        assert_eq!(pid, std::process::id());
    }

    #[test]
    fn pid_guard_removes_pid_file_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let pid_path = pid_file_path(dir.path());
        {
            let _guard = PidGuard::new(dir.path()).unwrap();
            assert!(pid_path.exists());
        }
        assert!(!pid_path.exists(), "PID file should be removed after guard is dropped");
    }

    #[test]
    fn pid_guard_creates_data_dir_if_missing() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("sub").join("dir");
        let _guard = PidGuard::new(&nested).unwrap();
        let pid_path = pid_file_path(&nested);
        assert!(pid_path.exists());
    }
}
