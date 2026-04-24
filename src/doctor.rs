use colored::Colorize;
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Required,
    Optional,
}

pub struct CheckResult {
    pub name: String,
    pub passed: bool,
    pub severity: Severity,
    pub detail: String,
    pub suggestion: Option<String>,
}

/// Format a byte count into a human-readable string (B/KB/MB/GB).
pub fn format_bytes(n: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if n >= GB {
        format!("{:.1} GB", n as f64 / GB as f64)
    } else if n >= MB {
        format!("{:.1} MB", n as f64 / MB as f64)
    } else if n >= KB {
        format!("{:.1} KB", n as f64 / KB as f64)
    } else {
        format!("{n} B")
    }
}

/// Returns true if every Required check passed. Optional failures are ignored.
pub fn all_required_passed(results: &[CheckResult]) -> bool {
    results
        .iter()
        .filter(|r| r.severity == Severity::Required)
        .all(|r| r.passed)
}

pub fn check_config() -> CheckResult {
    let dir = match crate::config::config_dir() {
        Ok(d) => d,
        Err(e) => {
            return CheckResult {
                name: "Config file".into(),
                passed: false,
                severity: Severity::Required,
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
            severity: Severity::Required,
            detail: format!("Not found at {}", path.display()),
            suggestion: Some("Run `blackbox init` to create config".into()),
        };
    }

    match crate::config::load_config() {
        Ok(_) => CheckResult {
            name: "Config file".into(),
            passed: true,
            severity: Severity::Required,
            detail: format!("Valid at {}", path.display()),
            suggestion: None,
        },
        Err(e) => CheckResult {
            name: "Config file".into(),
            passed: false,
            severity: Severity::Required,
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
                    severity: Severity::Required,
                    detail: "Exists".into(),
                    suggestion: None,
                }
            } else if dir.exists() {
                CheckResult {
                    name: format!("Watch dir: {}", dir.display()),
                    passed: false,
                    severity: Severity::Required,
                    detail: "Not a directory".into(),
                    suggestion: Some("Update watch_dirs in config.toml".into()),
                }
            } else {
                CheckResult {
                    name: format!("Watch dir: {}", dir.display()),
                    passed: false,
                    severity: Severity::Required,
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
                severity: Severity::Required,
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
                severity: Severity::Required,
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
        let size_str = std::fs::metadata(&db_path)
            .ok()
            .map(|m| format_bytes(m.len()))
            .unwrap_or_else(|| "size unknown".into());
        CheckResult {
            name: "Database".into(),
            passed: true,
            severity: Severity::Required,
            detail: format!("OK at {} ({size_str})", db_path.display()),
            suggestion: None,
        }
    } else {
        CheckResult {
            name: "Database".into(),
            passed: false,
            severity: Severity::Required,
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
                severity: Severity::Optional,
                detail: format!("Cannot determine data dir: {e}"),
                suggestion: None,
            };
        }
    };

    match crate::daemon::is_daemon_running(&dir) {
        Ok(Some(pid)) => CheckResult {
            name: "Daemon".into(),
            passed: true,
            severity: Severity::Optional,
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
                        severity: Severity::Optional,
                        detail: format!("Running via launchd (PID {pid})"),
                        suggestion: None,
                    };
                } else {
                    return CheckResult {
                        name: "Daemon".into(),
                        passed: true,
                        severity: Severity::Optional,
                        detail: "Loaded in launchd (not yet started)".into(),
                        suggestion: None,
                    };
                }
            }
            CheckResult {
                name: "Daemon".into(),
                passed: false,
                severity: Severity::Optional,
                detail: "Not running".into(),
                suggestion: Some("Run `blackbox start` to start daemon".into()),
            }
        }
        Err(e) => CheckResult {
            name: "Daemon".into(),
            passed: false,
            severity: Severity::Optional,
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
            severity: Severity::Optional,
            detail: "gh not found on PATH".into(),
            suggestion: Some("Install: brew install gh (enables PR enrichment)".into()),
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
            severity: Severity::Optional,
            detail: "Authenticated".into(),
            suggestion: None,
        }
    } else {
        CheckResult {
            name: "GitHub CLI".into(),
            passed: false,
            severity: Severity::Optional,
            detail: "Not authenticated".into(),
            suggestion: Some("Run `gh auth login` (enables PR enrichment)".into()),
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
                severity: Severity::Optional,
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
                severity: Severity::Optional,
                detail: format!("Found in {}", rc.display()),
                suggestion: None,
            };
        }
    }

    CheckResult {
        name: "Shell hook".into(),
        passed: false,
        severity: Severity::Optional,
        detail: "Not found in any shell rc file".into(),
        suggestion: Some("Add to rc: eval \"$(blackbox hook zsh)\"".into()),
    }
}

pub fn check_llm(config: &crate::config::Config) -> CheckResult {
    check_llm_with_env(config, |k| std::env::var(k).ok())
}

/// Testable variant: env reader injected so tests can simulate env var state
/// without mutating the process environment.
pub fn check_llm_with_env<F>(config: &crate::config::Config, env: F) -> CheckResult
where
    F: Fn(&str) -> Option<String>,
{
    let provider = config.llm_provider.as_deref().unwrap_or("anthropic");

    // Precedence: config.llm_api_key > env var (ANTHROPIC_API_KEY / OPENAI_API_KEY).
    // Whitespace-only config key counts as "present but broken" — don't silently fall
    // back to env, user likely made a typo they want to fix.
    let (key, source) = match config.llm_api_key.as_deref() {
        Some(k) if !k.trim().is_empty() => (k.to_string(), "config"),
        Some(_) => {
            return CheckResult {
                name: "LLM API key".into(),
                passed: false,
                severity: Severity::Optional,
                detail: "Key is empty or whitespace".into(),
                suggestion: Some("Set llm_api_key in config.toml or remove the field".into()),
            };
        }
        None => match env_key_name(provider).and_then(env) {
            Some(k) if !k.trim().is_empty() => (k, "env"),
            _ => {
                return CheckResult {
                    name: "LLM API key".into(),
                    passed: true,
                    severity: Severity::Optional,
                    detail: "Not configured (optional — enables `insights`, `perf-review`, `--summarize`)".into(),
                    suggestion: None,
                };
            }
        },
    };

    let (expected_prefix, min_len) = match provider {
        "anthropic" => ("sk-ant-", 40),
        "openai" => ("sk-", 20),
        _ => {
            return CheckResult {
                name: "LLM API key".into(),
                passed: true,
                severity: Severity::Optional,
                detail: format!("Present ({provider}, {source}, {} chars — format unchecked)", key.len()),
                suggestion: None,
            };
        }
    };

    if !key.starts_with(expected_prefix) || key.len() < min_len {
        return CheckResult {
            name: "LLM API key".into(),
            passed: false,
            severity: Severity::Optional,
            detail: format!("Format mismatch for provider '{provider}' (expected prefix '{expected_prefix}')"),
            suggestion: Some("Check key copied correctly; regenerate at provider console".into()),
        };
    }

    CheckResult {
        name: "LLM API key".into(),
        passed: true,
        severity: Severity::Optional,
        detail: format!("Configured ({provider}, {source}, {} chars)", key.len()),
        suggestion: None,
    }
}

/// Conventional env var name for a given LLM provider.
pub fn env_key_name(provider: &str) -> Option<&'static str> {
    match provider {
        "anthropic" => Some("ANTHROPIC_API_KEY"),
        "openai" => Some("OPENAI_API_KEY"),
        _ => None,
    }
}

pub fn check_notifications() -> CheckResult {
    if crate::notifications::is_available() {
        CheckResult {
            name: "Notifications".into(),
            passed: true,
            severity: Severity::Optional,
            detail: "Desktop notifications available".into(),
            suggestion: None,
        }
    } else {
        CheckResult {
            name: "Notifications".into(),
            passed: false,
            severity: Severity::Optional,
            detail: "Desktop notification system not available".into(),
            suggestion: Some("On Linux: ensure D-Bus + a notification daemon. On macOS: grant Terminal notification perms.".into()),
        }
    }
}

pub fn check_ai_tools() -> Vec<CheckResult> {
    crate::ai_tracking::all_detectors()
        .iter()
        .map(|d| {
            let name = d.tool_name();
            if d.is_installed() {
                CheckResult {
                    name: format!("AI tool: {name}"),
                    passed: true,
                    severity: Severity::Optional,
                    detail: "Installed (sessions tracked)".into(),
                    suggestion: None,
                }
            } else {
                CheckResult {
                    name: format!("AI tool: {name}"),
                    passed: false,
                    severity: Severity::Optional,
                    detail: "Not installed".into(),
                    suggestion: None,
                }
            }
        })
        .collect()
}

/// Parse launchctl list output to determine if service is loaded.
/// Returns Some(pid) if running with a PID, Some(0) if loaded but no PID, None if not found.
#[cfg(target_os = "macos")]
#[allow(clippy::collapsible_if)]
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

/// Run all doctor checks. Returns true if all required checks passed.
/// Optional failures are surfaced as warnings but do not affect the return value.
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

    if let Some(ref cfg) = loaded_config {
        results.extend(check_watch_dirs(cfg));
    }

    results.push(check_database());
    results.push(check_daemon());
    results.push(check_gh_cli());
    results.push(check_shell_hook());

    if let Some(ref cfg) = loaded_config {
        results.push(check_llm(cfg));
    }
    results.push(check_notifications());
    results.extend(check_ai_tools());

    println!();
    for r in &results {
        if r.passed {
            println!("  {} {}: {}", "✓".green(), r.name, r.detail);
        } else {
            let (mark, detail_color) = match r.severity {
                Severity::Required => ("✗".red().to_string(), r.detail.red().to_string()),
                Severity::Optional => ("!".yellow().to_string(), r.detail.yellow().to_string()),
            };
            println!("  {} {}: {}", mark, r.name, detail_color);
            if let Some(ref suggestion) = r.suggestion {
                println!("    → {}", suggestion.yellow());
            }
        }
    }
    println!();

    let required_fails = results
        .iter()
        .filter(|r| r.severity == Severity::Required && !r.passed)
        .count();
    let optional_fails = results
        .iter()
        .filter(|r| r.severity == Severity::Optional && !r.passed)
        .count();

    if required_fails == 0 {
        if optional_fails == 0 {
            println!("{}", "All checks passed!".green().bold());
        } else {
            println!(
                "{} {}",
                "Required checks passed.".green().bold(),
                format!("({optional_fails} optional check(s) with warnings)").yellow()
            );
        }
    } else {
        println!(
            "{}",
            format!("{required_fails} required check(s) failed").red().bold()
        );
    }

    Ok(all_required_passed(&results))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_required_passed_ignores_optional_failures() {
        let results = vec![
            CheckResult {
                name: "req-ok".into(),
                passed: true,
                severity: Severity::Required,
                detail: String::new(),
                suggestion: None,
            },
            CheckResult {
                name: "opt-fail".into(),
                passed: false,
                severity: Severity::Optional,
                detail: String::new(),
                suggestion: None,
            },
        ];
        assert!(all_required_passed(&results));
    }

    fn cfg_with_llm(key: Option<&str>, provider: Option<&str>) -> crate::config::Config {
        let mut cfg = crate::config::Config::default();
        cfg.llm_api_key = key.map(|s| s.to_string());
        cfg.llm_provider = provider.map(|s| s.to_string());
        cfg
    }

    #[test]
    fn check_llm_absent_passes_as_optional() {
        let cfg = cfg_with_llm(None, None);
        let r = check_llm(&cfg);
        assert!(r.passed, "absent key should not fail");
        assert_eq!(r.severity, Severity::Optional);
        assert!(r.detail.to_lowercase().contains("not configured"));
    }

    #[test]
    fn check_llm_empty_string_fails() {
        let cfg = cfg_with_llm(Some(""), None);
        let r = check_llm(&cfg);
        assert!(!r.passed);
        assert_eq!(r.severity, Severity::Optional);
    }

    #[test]
    fn check_llm_whitespace_fails() {
        let cfg = cfg_with_llm(Some("   \t\n"), None);
        let r = check_llm(&cfg);
        assert!(!r.passed);
    }

    #[test]
    fn check_llm_anthropic_valid_prefix_passes() {
        let key = "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789ABCDEF";
        let cfg = cfg_with_llm(Some(key), Some("anthropic"));
        let r = check_llm(&cfg);
        assert!(r.passed, "valid anthropic key should pass, got: {}", r.detail);
        assert!(r.detail.contains("anthropic"));
    }

    #[test]
    fn check_llm_anthropic_wrong_prefix_fails() {
        let cfg = cfg_with_llm(Some("sk-proj-wrong-prefix-for-anthropic"), Some("anthropic"));
        let r = check_llm(&cfg);
        assert!(!r.passed);
        assert!(r.detail.to_lowercase().contains("format"));
    }

    #[test]
    fn check_llm_openai_valid_prefix_passes() {
        let cfg = cfg_with_llm(Some("sk-proj-validlengthkey1234567"), Some("openai"));
        let r = check_llm(&cfg);
        assert!(r.passed, "valid openai key should pass, got: {}", r.detail);
    }

    #[test]
    fn check_llm_unknown_provider_passes_with_warning() {
        let cfg = cfg_with_llm(Some("whatever-format-key-12345"), Some("weird-provider"));
        let r = check_llm(&cfg);
        assert!(r.passed, "unknown provider should not fail format check");
        assert!(r.detail.contains("weird-provider"));
    }

    #[test]
    fn check_ai_tools_returns_one_result_per_detector() {
        let results = check_ai_tools();
        assert!(
            results.len() >= 5,
            "expected ≥5 AI tool checks, got {}",
            results.len()
        );
    }

    #[test]
    fn check_ai_tools_all_optional_severity() {
        let results = check_ai_tools();
        for r in &results {
            assert_eq!(
                r.severity,
                Severity::Optional,
                "AI tool check '{}' should be Optional",
                r.name
            );
        }
    }

    #[test]
    fn check_ai_tools_names_prefixed_with_ai_tool() {
        let results = check_ai_tools();
        for r in &results {
            assert!(
                r.name.starts_with("AI tool:"),
                "expected 'AI tool:' prefix, got: {}",
                r.name
            );
        }
    }

    #[test]
    fn check_ai_tools_covers_known_detectors() {
        let results = check_ai_tools();
        let names: Vec<&str> = results.iter().map(|r| r.name.as_str()).collect();
        let joined = names.join(",");
        for expected in &["claude-code", "codex", "copilot-cli", "cursor", "windsurf"] {
            assert!(
                joined.contains(expected),
                "missing detector '{}' in: {}",
                expected,
                joined
            );
        }
    }

    #[test]
    fn check_llm_env_var_fallback_anthropic() {
        let cfg = cfg_with_llm(None, Some("anthropic"));
        let env = |k: &str| {
            (k == "ANTHROPIC_API_KEY")
                .then(|| "sk-ant-api03-envvarfallback12345678901234567890".to_string())
        };
        let r = check_llm_with_env(&cfg, env);
        assert!(r.passed, "env var should satisfy check, got: {}", r.detail);
        assert!(r.detail.contains("env"), "detail should indicate env source, got: {}", r.detail);
    }

    #[test]
    fn check_llm_config_wins_over_env() {
        let cfg = cfg_with_llm(
            Some("sk-ant-api03-from-config-000000000000000000000"),
            Some("anthropic"),
        );
        let env = |_: &str| Some("sk-ant-api03-from-env-000000000000000000000000".to_string());
        let r = check_llm_with_env(&cfg, env);
        assert!(r.passed);
        assert!(r.detail.contains("config"), "config should win over env, got: {}", r.detail);
    }

    #[test]
    fn check_llm_no_key_no_env_is_optional_pass() {
        let cfg = cfg_with_llm(None, Some("anthropic"));
        let r = check_llm_with_env(&cfg, |_| None);
        assert!(r.passed);
        assert_eq!(r.severity, Severity::Optional);
    }

    #[test]
    fn format_bytes_scales_correctly() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(2048), "2.0 KB");
        assert_eq!(format_bytes(1024 * 1024 * 3), "3.0 MB");
        assert_eq!(format_bytes(1024u64.pow(3) * 2), "2.0 GB");
    }

    #[test]
    fn env_key_name_maps_providers() {
        assert_eq!(env_key_name("anthropic"), Some("ANTHROPIC_API_KEY"));
        assert_eq!(env_key_name("openai"), Some("OPENAI_API_KEY"));
        assert_eq!(env_key_name("unknown"), None);
    }

    #[test]
    fn check_llm_never_leaks_raw_key_in_detail() {
        let key = "sk-ant-supersecretkey1234567890abcdefABCDEFGHIJK";
        let cfg = cfg_with_llm(Some(key), Some("anthropic"));
        let r = check_llm(&cfg);
        assert!(!r.detail.contains(key), "detail must not contain raw key: {}", r.detail);
    }

    #[test]
    fn all_required_passed_fails_on_required_failure() {
        let results = vec![CheckResult {
            name: "req-fail".into(),
            passed: false,
            severity: Severity::Required,
            detail: String::new(),
            suggestion: None,
        }];
        assert!(!all_required_passed(&results));
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
