use colored::Colorize;
use std::process::Command;

pub struct CheckResult {
    pub name: String,
    pub passed: bool,
    pub detail: String,
    pub suggestion: Option<String>,
}

pub fn check_config() -> CheckResult {
    let dir = match crate::config::config_dir() {
        Ok(d) => d,
        Err(e) => {
            return CheckResult {
                name: "Config file".into(),
                passed: false,
                detail: format!("Cannot determine config dir: {e}"),
                suggestion: Some("Check XDG_CONFIG_HOME env var".into()),
            };
        }
    };

    let path = dir.join("config.toml");
    if !path.exists() {
        return CheckResult {
            name: "Config file".into(),
            passed: false,
            detail: format!("Not found at {}", path.display()),
            suggestion: Some("Run `blackbox init` to create config".into()),
        };
    }

    match crate::config::load_config() {
        Ok(_) => CheckResult {
            name: "Config file".into(),
            passed: true,
            detail: format!("Valid at {}", path.display()),
            suggestion: None,
        },
        Err(e) => CheckResult {
            name: "Config file".into(),
            passed: false,
            detail: format!("Parse error: {e}"),
            suggestion: Some("Check config.toml syntax".into()),
        },
    }
}

pub fn check_watch_dirs(config: &crate::config::Config) -> Vec<CheckResult> {
    config
        .watch_dirs
        .iter()
        .map(|dir| {
            if dir.is_dir() {
                CheckResult {
                    name: format!("Watch dir: {}", dir.display()),
                    passed: true,
                    detail: "Exists".into(),
                    suggestion: None,
                }
            } else if dir.exists() {
                CheckResult {
                    name: format!("Watch dir: {}", dir.display()),
                    passed: false,
                    detail: "Not a directory".into(),
                    suggestion: Some("Update watch_dirs in config.toml".into()),
                }
            } else {
                CheckResult {
                    name: format!("Watch dir: {}", dir.display()),
                    passed: false,
                    detail: "Not found".into(),
                    suggestion: Some(format!("Create it or update config: mkdir -p {}", dir.display())),
                }
            }
        })
        .collect()
}

pub fn check_database() -> CheckResult {
    let dir = match crate::config::data_dir() {
        Ok(d) => d,
        Err(e) => {
            return CheckResult {
                name: "Database".into(),
                passed: false,
                detail: format!("Cannot determine data dir: {e}"),
                suggestion: Some("Check XDG_DATA_HOME env var".into()),
            };
        }
    };

    let db_path = dir.join("blackbox.db");
    let conn = match crate::db::open_db(&db_path) {
        Ok(c) => c,
        Err(e) => {
            return CheckResult {
                name: "Database".into(),
                passed: false,
                detail: format!("Cannot open: {e}"),
                suggestion: Some("Run `blackbox start` to create DB".into()),
            };
        }
    };

    let tables: Vec<String> = conn
        .prepare(
            "SELECT name FROM sqlite_master WHERE type='table' AND name IN ('git_activity','directory_presence')",
        )
        .and_then(|mut stmt| {
            stmt.query_map([], |row| row.get(0))
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default();

    if tables.len() == 2 {
        CheckResult {
            name: "Database".into(),
            passed: true,
            detail: format!("OK at {}", db_path.display()),
            suggestion: None,
        }
    } else {
        CheckResult {
            name: "Database".into(),
            passed: false,
            detail: format!("Missing tables (found: {})", tables.join(", ")),
            suggestion: Some("DB may be corrupted; try deleting and restarting daemon".into()),
        }
    }
}

pub fn check_daemon() -> CheckResult {
    let dir = match crate::config::data_dir() {
        Ok(d) => d,
        Err(e) => {
            return CheckResult {
                name: "Daemon".into(),
                passed: false,
                detail: format!("Cannot determine data dir: {e}"),
                suggestion: None,
            };
        }
    };

    match crate::daemon::is_daemon_running(&dir) {
        Ok(Some(pid)) => CheckResult {
            name: "Daemon".into(),
            passed: true,
            detail: format!("Running (PID {pid})"),
            suggestion: None,
        },
        Ok(None) => {
            // Fallback: check if launchd is managing the service
            if let Some(pid) = is_launchd_running() {
                if pid > 0 {
                    return CheckResult {
                        name: "Daemon".into(),
                        passed: true,
                        detail: format!("Running via launchd (PID {pid})"),
                        suggestion: None,
                    };
                } else {
                    return CheckResult {
                        name: "Daemon".into(),
                        passed: true,
                        detail: "Loaded in launchd (not yet started)".into(),
                        suggestion: None,
                    };
                }
            }
            CheckResult {
                name: "Daemon".into(),
                passed: false,
                detail: "Not running".into(),
                suggestion: Some("Run `blackbox start` to start daemon".into()),
            }
        }
        Err(e) => CheckResult {
            name: "Daemon".into(),
            passed: false,
            detail: format!("Error checking: {e}"),
            suggestion: Some("Check PID file permissions".into()),
        },
    }
}

pub fn check_gh_cli() -> CheckResult {
    let gh_exists = Command::new("which")
        .arg("gh")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if !gh_exists {
        return CheckResult {
            name: "GitHub CLI".into(),
            passed: false,
            detail: "gh not found on PATH".into(),
            suggestion: Some("Install: brew install gh".into()),
        };
    }

    let auth_ok = Command::new("gh")
        .args(["auth", "status"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if auth_ok {
        CheckResult {
            name: "GitHub CLI".into(),
            passed: true,
            detail: "Authenticated".into(),
            suggestion: None,
        }
    } else {
        CheckResult {
            name: "GitHub CLI".into(),
            passed: false,
            detail: "Not authenticated".into(),
            suggestion: Some("Run `gh auth login`".into()),
        }
    }
}

pub fn check_shell_hook() -> CheckResult {
    let home = match etcetera::home_dir() {
        Ok(h) => h,
        Err(_) => {
            return CheckResult {
                name: "Shell hook".into(),
                passed: false,
                detail: "Cannot determine home directory".into(),
                suggestion: None,
            };
        }
    };

    let rc_files = [
        home.join(".zshrc"),
        home.join(".bashrc"),
        home.join(".config/fish/config.fish"),
    ];

    for rc in &rc_files {
        if let Ok(content) = std::fs::read_to_string(rc)
            && content.contains("blackbox")
        {
            return CheckResult {
                name: "Shell hook".into(),
                passed: true,
                detail: format!("Found in {}", rc.display()),
                suggestion: None,
            };
        }
    }

    CheckResult {
        name: "Shell hook".into(),
        passed: false,
        detail: "Not found in any shell rc file".into(),
        suggestion: Some("Add to rc: eval \"$(blackbox hook zsh)\"".into()),
    }
}

/// Parse launchctl list output to determine if service is loaded.
/// Returns Some(pid) if running with a PID, Some(0) if loaded but no PID, None if not found.
#[cfg(target_os = "macos")]
pub fn parse_launchctl_output(success: bool, stdout: &str) -> Option<u32> {
    if !success {
        return None;
    }
    // launchctl list <label> output contains "PID" = <number>; if running
    for line in stdout.lines() {
        let trimmed = line.trim().trim_end_matches(';');
        if trimmed.starts_with("\"PID\"") {
            if let Some(val) = trimmed.split('=').nth(1) {
                let val = val.trim().trim_matches('"');
                if let Ok(pid) = val.parse::<u32>() {
                    return Some(pid);
                }
            }
        }
    }
    // Service is loaded but no PID found (not currently running process)
    Some(0)
}

/// Check if blackbox is managed by launchd.
#[cfg(target_os = "macos")]
pub fn is_launchd_running() -> Option<u32> {
    let output = Command::new("launchctl")
        .args(["list", "com.blackbox.agent"])
        .output()
        .ok()?;
    parse_launchctl_output(
        output.status.success(),
        &String::from_utf8_lossy(&output.stdout),
    )
}

#[cfg(not(target_os = "macos"))]
pub fn is_launchd_running() -> Option<u32> {
    None
}

/// Run all doctor checks. Returns true if all passed.
pub fn run_doctor() -> anyhow::Result<bool> {
    let mut results = Vec::new();

    let config_result = check_config();
    let config_ok = config_result.passed;
    results.push(config_result);

    let loaded_config = if config_ok {
        crate::config::load_config().ok()
    } else {
        None
    };

    if let Some(ref config) = loaded_config {
        results.extend(check_watch_dirs(config));
    }

    let db_result = check_database();
    let db_ok = db_result.passed;
    results.push(db_result);

    // Work-hours warning: query last 7 days if both config and DB are available
    if let Some(ref config) = loaded_config {
        if db_ok {
            if let Ok(data_dir) = crate::config::data_dir() {
                let db_path = data_dir.join("blackbox.db");
                if let Ok(conn) = crate::db::open_db(&db_path) {
                    let now = chrono::Utc::now();
                    let week_ago = now - chrono::Duration::days(7);
                    if let Ok(repos) = crate::query::query_activity(
                        &conn,
                        week_ago,
                        now,
                        config.session_gap_minutes,
                        config.first_commit_minutes,
                    ) {
                        let analysis = crate::insights::work_hours_analysis(
                            &repos,
                            config.work_hours_start,
                            config.work_hours_end,
                        );
                        if analysis.total_commits > 0 {
                            results.push(check_work_hours(analysis.after_hours_pct));
                        }
                    }
                }
            }
        }
    }
    results.push(check_daemon());
    results.push(check_gh_cli());
    results.push(check_shell_hook());

    println!();
    for r in &results {
        if r.passed {
            println!("  {} {}: {}", "✓".green(), r.name, r.detail);
        } else {
            println!("  {} {}: {}", "✗".red(), r.name, r.detail.red());
            if let Some(ref suggestion) = r.suggestion {
                println!("    → {}", suggestion.yellow());
            }
        }
    }
    println!();

    let all_passed = results.iter().all(|r| r.passed);
    if all_passed {
        println!("{}", "All checks passed!".green().bold());
    } else {
        let fail_count = results.iter().filter(|r| !r.passed).count();
        println!(
            "{}",
            format!("{fail_count} check(s) failed").red().bold()
        );
    }

    Ok(all_passed)
}

/// Check after-hours commit percentage and warn if above 30%.
pub fn check_work_hours(after_hours_pct: f64) -> CheckResult {
    if after_hours_pct > 30.0 {
        CheckResult {
            name: "Work-life balance".into(),
            passed: false,
            detail: format!("{:.0}% of commits in last 7 days were outside work hours", after_hours_pct),
            suggestion: Some("Consider adjusting work_hours_start/work_hours_end in config, or take a break!".into()),
        }
    } else {
        CheckResult {
            name: "Work-life balance".into(),
            passed: true,
            detail: format!("{:.0}% after-hours commits (last 7 days)", after_hours_pct),
            suggestion: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_work_hours_warns_above_30_pct() {
        let result = check_work_hours(35.0);
        assert!(!result.passed);
        assert!(result.detail.contains("35%"));
        assert!(result.suggestion.is_some());
    }

    #[test]
    fn check_work_hours_passes_at_30_pct() {
        let result = check_work_hours(30.0);
        assert!(result.passed);
    }

    #[test]
    fn check_work_hours_passes_at_zero() {
        let result = check_work_hours(0.0);
        assert!(result.passed);
        assert!(result.detail.contains("0%"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn parse_launchctl_output_with_pid() {
        let stdout = r#"{
    "LimitLoadToSessionType" = "Aqua";
    "Label" = "com.blackbox.agent";
    "LastExitStatus" = 0;
    "PID" = 12345;
    "Program" = "/usr/local/bin/blackbox";
};"#;
        assert_eq!(parse_launchctl_output(true, stdout), Some(12345));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn parse_launchctl_output_loaded_no_pid() {
        let stdout = r#"{
    "LimitLoadToSessionType" = "Aqua";
    "Label" = "com.blackbox.agent";
    "LastExitStatus" = 0;
    "Program" = "/usr/local/bin/blackbox";
};"#;
        assert_eq!(parse_launchctl_output(true, stdout), Some(0));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn parse_launchctl_output_not_found() {
        assert_eq!(parse_launchctl_output(false, ""), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn parse_launchctl_output_empty_success() {
        // Edge case: success but empty output
        assert_eq!(parse_launchctl_output(true, ""), Some(0));
    }
}
