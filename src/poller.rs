use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use crate::config::{self, Config};
use crate::db;
use crate::git_ops::{self, RepoState};
use crate::repo_scanner;

pub fn run_poll_loop(config: &Config) -> anyhow::Result<()> {
    let db_path = config::data_dir()?.join("blackbox.db");
    let conn = db::open_db(&db_path)?;
    let mut repo_states: HashMap<PathBuf, RepoState> = HashMap::new();

    loop {
        let repos = repo_scanner::discover_repos(&config.watch_dirs);
        for repo_path in &repos {
            let state = repo_states.entry(repo_path.clone()).or_default();
            if let Err(e) = git_ops::poll_repo(repo_path, state, &conn) {
                log::warn!("Error polling {}: {}", repo_path.display(), e);
            }
        }
        std::thread::sleep(Duration::from_secs(config.poll_interval_secs));
    }
}
