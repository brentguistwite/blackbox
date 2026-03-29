use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::poller;

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

pub fn daemon_status(data_dir: &Path) -> anyhow::Result<()> {
    match is_daemon_running(data_dir)? {
        Some(pid) => println!("Running (PID {})", pid),
        None => println!("Stopped"),
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
