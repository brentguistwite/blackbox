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
fn default_session_gap() -> u64 {
    120
}
fn default_first_commit() -> u64 {
    30
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_watch_dirs")]
    pub watch_dirs: Vec<PathBuf>,
    #[serde(default = "default_poll_interval")]
    pub poll_interval_secs: u64,
    #[serde(default = "default_session_gap")]
    pub session_gap_minutes: u64,
    #[serde(default = "default_first_commit")]
    pub first_commit_minutes: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            watch_dirs: default_watch_dirs(),
            poll_interval_secs: default_poll_interval(),
            session_gap_minutes: default_session_gap(),
            first_commit_minutes: default_first_commit(),
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

impl Config {
    pub fn save_to(&self, path: &std::path::Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }
}

pub fn run_init(watch_dirs: Option<String>, poll_interval: Option<u64>) -> anyhow::Result<()> {
    let config_path = config_dir()?.join("config.toml");

    if config_path.exists() {
        println!("Config already exists at {}", config_path.display());
        return Ok(());
    }

    let config = if let (Some(dirs), Some(interval)) = (watch_dirs, poll_interval) {
        let watch_dirs: Vec<PathBuf> = dirs.split(',').map(|s| PathBuf::from(s.trim())).collect();
        Config {
            watch_dirs,
            poll_interval_secs: interval,
            ..Config::default()
        }
    } else {
        // Interactive mode using dialoguer
        let dirs_input: String = dialoguer::Input::new()
            .with_prompt("Watch directories (comma-separated)")
            .default("~/code".to_string())
            .interact_text()?;
        let watch_dirs: Vec<PathBuf> = dirs_input.split(',').map(|s| PathBuf::from(s.trim())).collect();

        let poll_interval: u64 = dialoguer::Input::new()
            .with_prompt("Poll interval (seconds)")
            .default(300u64)
            .interact_text()?;

        Config {
            watch_dirs,
            poll_interval_secs: poll_interval,
            ..Config::default()
        }
    };

    config.validate()?;
    config.save_to(&config_path)?;
    println!("Config created at {}", config_path.display());
    Ok(())
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
