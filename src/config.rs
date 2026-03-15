use std::path::PathBuf;

pub struct Config {
    pub watch_dirs: Vec<PathBuf>,
    pub poll_interval_secs: u64,
}

pub fn config_dir() -> anyhow::Result<PathBuf> {
    todo!()
}

pub fn data_dir() -> anyhow::Result<PathBuf> {
    todo!()
}

pub fn load_config() -> anyhow::Result<Config> {
    todo!()
}
