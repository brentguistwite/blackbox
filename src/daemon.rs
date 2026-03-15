use std::path::PathBuf;

pub fn pid_file_path() -> anyhow::Result<PathBuf> {
    todo!()
}

pub fn is_daemon_running() -> anyhow::Result<Option<u32>> {
    todo!()
}

pub fn start_daemon(config: crate::config::Config) -> anyhow::Result<()> {
    todo!()
}

pub fn stop_daemon() -> anyhow::Result<()> {
    todo!()
}

pub fn daemon_status() -> anyhow::Result<()> {
    todo!()
}
