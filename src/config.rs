use anyhow::Context;
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
fn default_work_hours_start() -> u8 {
    8
}
fn default_work_hours_end() -> u8 {
    18
}
fn default_streak_rest_days() -> Vec<u8> {
    vec![5, 6]
}
fn default_ticket_patterns() -> Vec<String> {
    vec![r"[A-Z]+-\d+".to_string()]
}
fn default_deep_work_threshold_minutes() -> u64 {
    60
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
    #[serde(default = "default_work_hours_start")]
    pub work_hours_start: u8,
    #[serde(default = "default_work_hours_end")]
    pub work_hours_end: u8,
    #[serde(default = "default_streak_rest_days")]
    pub streak_rest_days: Vec<u8>,
    #[serde(default)]
    pub standup_webhook_url: Option<String>,
    #[serde(default = "default_ticket_patterns")]
    pub ticket_patterns: Vec<String>,
    #[serde(default)]
    pub track_file_changes: bool,
    #[serde(default = "default_deep_work_threshold_minutes")]
    pub deep_work_threshold_minutes: u64,
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
            work_hours_start: default_work_hours_start(),
            work_hours_end: default_work_hours_end(),
            streak_rest_days: default_streak_rest_days(),
            standup_webhook_url: None,
            ticket_patterns: default_ticket_patterns(),
            track_file_changes: false,
            deep_work_threshold_minutes: default_deep_work_threshold_minutes(),
        }
    }
}

impl Config {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.poll_interval_secs < 10 {
            anyhow::bail!("poll_interval_secs must be >= 10, got {}", self.poll_interval_secs);
        }
        if self.work_hours_start > 23 {
            anyhow::bail!("work_hours_start must be 0..=23, got {}", self.work_hours_start);
        }
        if self.work_hours_end > 23 {
            anyhow::bail!("work_hours_end must be 0..=23, got {}", self.work_hours_end);
        }
        if self.work_hours_start >= self.work_hours_end {
            anyhow::bail!(
                "work_hours_start ({}) must be less than work_hours_end ({})",
                self.work_hours_start, self.work_hours_end
            );
        }
        Ok(())
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
