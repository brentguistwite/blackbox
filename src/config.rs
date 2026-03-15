use anyhow::Context;
use etcetera::{choose_base_strategy, BaseStrategy};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

fn default_poll_interval() -> u64 {
    300
}
fn default_watch_dirs() -> Vec<PathBuf> {
    vec![]
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_watch_dirs")]
    pub watch_dirs: Vec<PathBuf>,
    #[serde(default = "default_poll_interval")]
    pub poll_interval_secs: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            watch_dirs: default_watch_dirs(),
            poll_interval_secs: default_poll_interval(),
        }
    }
}

impl Config {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.poll_interval_secs < 10 {
            anyhow::bail!("poll_interval_secs must be >= 10, got {}", self.poll_interval_secs);
        }
        Ok(())
    }

    pub fn expand_paths(&mut self) {
        let home = etcetera::home_dir().ok();
        self.watch_dirs = self
            .watch_dirs
            .iter()
            .map(|p| {
                let s = p.to_string_lossy();
                if s.starts_with("~/") || s == "~" {
                    if let Some(ref h) = home {
                        h.join(s.strip_prefix("~/").unwrap_or(""))
                    } else {
                        p.clone()
                    }
                } else {
                    p.clone()
                }
            })
            .collect();
    }
}

pub fn config_dir() -> anyhow::Result<PathBuf> {
    let strategy = choose_base_strategy()?;
    Ok(strategy.config_dir().join("blackbox"))
}

pub fn data_dir() -> anyhow::Result<PathBuf> {
    let strategy = choose_base_strategy()?;
    Ok(strategy.data_dir().join("blackbox"))
}

pub fn load_config() -> anyhow::Result<Config> {
    let path = config_dir()?.join("config.toml");
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read config at {}", path.display()))?;
    let mut config: Config = toml::from_str(&content)
        .with_context(|| format!("Invalid config at {}", path.display()))?;
    config.expand_paths();
    config.validate()?;
    Ok(config)
}
