use std::path::PathBuf;

use crate::config::{self, Config};
use crate::poller;

pub fn pid_file_path() -> anyhow::Result<PathBuf> {
    Ok(config::data_dir()?.join("blackbox.pid"))
}

pub fn is_daemon_running() -> anyhow::Result<Option<u32>> {
    let path = pid_file_path()?;
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

pub fn start_daemon(config: Config) -> anyhow::Result<()> {
    if let Some(pid) = is_daemon_running()? {
        anyhow::bail!("Daemon already running (PID {})", pid);
    }

    let pid_path = pid_file_path()?;
    let data_dir = config::data_dir()?;
    std::fs::create_dir_all(&data_dir)?;

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
            if let Err(e) = poller::run_poll_loop(&config) {
                log::error!("Poll loop error: {}", e);
            }
            Ok(())
        }
        Err(e) => {
            anyhow::bail!("Failed to daemonize: {}", e);
        }
    }
}

pub fn stop_daemon() -> anyhow::Result<()> {
    match is_daemon_running()? {
        Some(pid) => {
            nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid as i32),
                nix::sys::signal::Signal::SIGTERM,
            )?;
            // Remove PID file
            let path = pid_file_path()?;
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

pub fn run_foreground(config: Config) -> anyhow::Result<()> {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .init();
    log::info!("Running in foreground (PID {})", std::process::id());
    poller::run_poll_loop(&config)
}

pub fn daemon_status() -> anyhow::Result<()> {
    match is_daemon_running()? {
        Some(pid) => println!("Running (PID {})", pid),
        None => println!("Stopped"),
    }
    Ok(())
}
