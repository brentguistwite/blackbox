use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::output::OutputFormat;
use crate::poller;

#[derive(Debug, serde::Serialize)]
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

    let (last_poll_at, repos_watched, events_today) = if db_path.exists() {
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
                let et = crate::db::count_events_today(&conn).ok();
                (lp, rw, et)
            }
            Err(_) => (None, None, None),
        }
    } else {
        (None, None, None)
    };

    let health = compute_health(running, last_poll_at);

    Ok(DaemonStatus {
        running,
        pid,
        uptime_secs,
        last_poll_at,
        repos_watched,
        db_size_bytes,
        events_today,
        health,
    })
}

fn compute_health(
    running: bool,
    last_poll_at: Option<chrono::DateTime<chrono::Utc>>,
) -> HealthIndicator {
    if !running {
        return HealthIndicator::Red;
    }
    match last_poll_at {
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
    }
}

fn render_status_pretty(status: &DaemonStatus) {
    use colored::Colorize;
    let (icon, label) = match status.health {
        HealthIndicator::Green => ("\u{2713}".green().bold(), "Running".green().bold()),
        HealthIndicator::Yellow => {
            ("\u{26a0}".yellow().bold(), "Running (stale)".yellow().bold())
        }
        HealthIndicator::Red => ("\u{2717}".red().bold(), "Stopped".red().bold()),
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
