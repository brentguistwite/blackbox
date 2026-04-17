use anyhow::Context;
use chrono::Weekday;
use etcetera::{choose_base_strategy, BaseStrategy};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

fn default_poll_interval() -> u64 {
    1800
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
fn default_worktree_dir_name() -> Option<String> {
    Some(".worktrees".to_string())
}
fn default_streak_exclude_weekends() -> bool {
    false
}
fn default_churn_window_days() -> u32 {
    14
}
fn default_notification_time() -> String {
    "17:00".to_string()
}
fn default_show_hints() -> bool {
    true
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
    #[serde(default)]
    pub llm_provider: Option<String>,
    #[serde(default)]
    pub llm_api_key: Option<String>,
    #[serde(default)]
    pub llm_model: Option<String>,
    #[serde(default)]
    pub llm_base_url: Option<String>,
    #[serde(default)]
    pub scan_dirs: Option<Vec<PathBuf>>,
    #[serde(default = "default_worktree_dir_name")]
    pub worktree_dir_name: Option<String>,
    #[serde(default = "default_streak_exclude_weekends")]
    pub streak_exclude_weekends: bool,
    #[serde(default = "default_churn_window_days")]
    pub churn_window_days: u32,
    #[serde(default)]
    pub notifications_enabled: bool,
    #[serde(default = "default_notification_time")]
    pub notification_time: String,
    #[serde(default)]
    pub insights_max_tokens: Option<u32>,
    #[serde(default)]
    pub insights_window: Option<String>,
    #[serde(default)]
    pub week_start_day: Option<String>,
    #[serde(default = "default_show_hints")]
    pub show_hints: bool,
    #[serde(default)]
    pub standup_lookback_days: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            watch_dirs: default_watch_dirs(),
            poll_interval_secs: default_poll_interval(),
            session_gap_minutes: default_session_gap(),
            first_commit_minutes: default_first_commit(),
            llm_provider: None,
            llm_api_key: None,
            llm_model: None,
            llm_base_url: None,
            scan_dirs: None,
            worktree_dir_name: default_worktree_dir_name(),
            streak_exclude_weekends: false,
            churn_window_days: default_churn_window_days(),
            notifications_enabled: false,
            notification_time: default_notification_time(),
            insights_max_tokens: None,
            insights_window: None,
            week_start_day: None,
            show_hints: default_show_hints(),
            standup_lookback_days: 0,
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

    pub fn week_start_weekday(&self) -> Weekday {
        match self.week_start_day.as_deref() {
            Some("sunday") => Weekday::Sun,
            Some("monday") | None => Weekday::Mon,
            Some(other) => {
                eprintln!("Warning: invalid week_start_day '{other}', falling back to monday");
                Weekday::Mon
            }
        }
    }

    pub fn expand_paths(&mut self) {
        let home = etcetera::home_dir().ok();
        self.watch_dirs = self.watch_dirs.iter().map(|p| expand_tilde_path(p, &home)).collect();
        if let Some(ref dirs) = self.scan_dirs {
            self.scan_dirs = Some(dirs.iter().map(|p| expand_tilde_path(p, &home)).collect());
        }
    }
}

fn expand_tilde_path(p: &Path, home: &Option<PathBuf>) -> PathBuf {
    let s = p.to_string_lossy();
    if (s.starts_with("~/") || s == "~")
        && let Some(h) = home
    {
        return h.join(s.strip_prefix("~/").unwrap_or(""));
    }
    p.to_path_buf()
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

pub fn config_exists() -> bool {
    config_dir()
        .map(|dir| dir.join("config.toml").exists())
        .unwrap_or(false)
}

fn load_config_from_path(path: &Path) -> anyhow::Result<Config> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read config at {}", path.display()))?;
    let mut config: Config = toml::from_str(&content)
        .with_context(|| format!("Invalid config at {}", path.display()))?;
    config.expand_paths();
    config.validate()?;
    Ok(config)
}

pub fn load_config() -> anyhow::Result<Config> {
    let path = config_dir()?.join("config.toml");
    load_config_from_path(&path)
}

/// Re-reads config from XDG path. Returns Err on parse/validation failure.
pub fn reload_config() -> anyhow::Result<Config> {
    load_config()
}

/// Reload config from a specific path (for testing).
pub fn reload_config_from(path: &Path) -> anyhow::Result<Config> {
    load_config_from_path(path)
}
