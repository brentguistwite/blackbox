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
                severity: Severity::Required,
                detail: format!("Cannot determine data dir: {e}"),
                suggestion: None,
            };
        }
    };

    match crate::daemon::is_daemon_running(&dir) {
        Ok(Some(pid)) => CheckResult {
            name: "Daemon".into(),
            passed: true,
            severity: Severity::Required,
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
                        severity: Severity::Required,
                        detail: format!("Running via launchd (PID {pid})"),
                        suggestion: None,
                    };
                } else {
                    // Loaded but no active PID → service registered but not polling.
                    // No data is being collected. Fail so `doctor` surfaces it.
                    return CheckResult {
                        name: "Daemon".into(),
                        passed: false,
                        severity: Severity::Required,
                        detail: "Loaded in launchd but not running (no polling)".into(),
                        suggestion: Some("Run `launchctl kickstart gui/$(id -u)/com.blackbox.agent` or `blackbox start`".into()),
                    };
                }
            }
            CheckResult {
                name: "Daemon".into(),
                passed: false,
                severity: Severity::Required,
                detail: "Not running".into(),
                suggestion: Some("Run `blackbox start` to start daemon".into()),
            }
        }
        Err(e) => CheckResult {
            name: "Daemon".into(),
            passed: false,
            severity: Severity::Required,
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
    // Mirror build_llm_config: auto-detect provider from env vars when config.llm_provider is unset.
    // Without this, `OPENAI_API_KEY` + no explicit provider would show "Configured (anthropic, ...)"
    // and runtime would still try to auth anthropic with an openai key.
    let provider_owned = match config.llm_provider.as_deref() {
        Some(p) => p.to_string(),
        None => {
            if env("ANTHROPIC_API_KEY").is_some() {
                "anthropic".to_string()
            } else if env("OPENAI_API_KEY").is_some() {
                "openai".to_string()
            } else {
                "anthropic".to_string()
            }
        }
    };
    let provider = provider_owned.as_str();

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

    if !crate::llm::is_supported_provider(provider) {
        return CheckResult {
            name: "LLM API key".into(),
            passed: false,
            severity: Severity::Optional,
            detail: format!(
                "Unsupported llm_provider '{provider}'. Supported: {}",
                crate::llm::SUPPORTED_PROVIDERS.join(", ")
            ),
            suggestion: Some("Set llm_provider to 'anthropic' or 'openai' in config.toml".into()),
        };
    }

    let (expected_prefix, min_len) = match provider {
        "anthropic" => ("sk-ant-", 40),
        "openai" => ("sk-", 20),
        _ => unreachable!("is_supported_provider guards this match"),
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

/// Stall threshold for a daemon running in watcher mode. Liveness is proven
/// only by full_scan (every FULL_SCAN_SECS) or watcher events; the user's
/// `poll_interval_secs` is irrelevant in this mode and would falsely widen
/// the threshold for users who set it large.
pub fn watcher_stall_threshold_secs() -> u64 {
    crate::poller::FULL_SCAN_SECS.saturating_mul(2)
}

/// Stall threshold for the pure-polling fallback mode. Floored at 120s so a
/// misconfigured `poll_interval_secs = 1` doesn't make stall fire instantly.
pub fn polling_stall_threshold_secs(poll_interval_secs: u64) -> u64 {
    std::cmp::max(poll_interval_secs.saturating_mul(3), 120)
}

/// Pick the stall threshold based on the mode the daemon last reported.
/// `None` (or unrecognized value) keeps backward-compat for daemons that
/// predate the `last_poll_mode` key.
pub fn stall_threshold_for_mode(mode: Option<&str>, poll_interval_secs: u64) -> u64 {
    match mode {
        Some("watcher") => watcher_stall_threshold_secs(),
        Some("polling") => polling_stall_threshold_secs(poll_interval_secs),
        _ => std::cmp::max(
            poll_interval_secs.saturating_mul(3),
            crate::poller::FULL_SCAN_SECS.saturating_mul(2),
        ),
    }
}

/// Inputs required to judge poll health. Pulled out as a struct so
/// `evaluate_poll_health` is pure (testable without DB / time mocking).
pub struct PollHealthInput {
    pub last_poll_at: Option<chrono::DateTime<chrono::Utc>>,
    pub repos_watched: usize,
    /// `None` = daemon never wrote this metric (predates the metric).
    /// `Some(0)` = daemon wrote it and reported zero failures.
    /// Treating these the same hides "running daemon is too old to surface failures".
    pub failed_count: Option<usize>,
    pub failed_sample: Vec<String>,
    /// Number of watch dirs that couldn't be read (TCC denial, permission
    /// errors, missing path). `None` = legacy daemon that didn't write the
    /// metric. `Some(0)` = clean discovery.
    pub discovery_failed_count: Option<usize>,
    pub discovery_failed_sample: Vec<String>,
    /// Longest expected gap between heartbeat updates. In watcher mode the
    /// daemon may sit idle between full scans for FULL_SCAN_SECS, so the
    /// stall threshold has to accommodate that — using `poll_interval_secs`
    /// alone would false-flag healthy idle watchers.
    pub max_expected_gap_secs: u64,
    pub now: chrono::DateTime<chrono::Utc>,
}

/// Decide poll health from raw inputs. Pure function — no DB or clock.
pub fn evaluate_poll_health(input: &PollHealthInput) -> CheckResult {
    let last = match input.last_poll_at {
        Some(t) => t,
        None => {
            // Render as a yellow warning — passed:false + Optional. We have no
            // signal yet; rendering as a green pass would silently mask a
            // daemon that's stuck before its first heartbeat.
            return CheckResult {
                name: "Poll health".into(),
                passed: false,
                severity: Severity::Optional,
                detail: "No poll cycle completed yet — daemon may still be starting".into(),
                suggestion: Some("Wait one poll cycle, then re-run `blackbox doctor`".into()),
            };
        }
    };

    // Stalled? Daemon process may be alive but its poll loop is not advancing
    // (deadlock, hung syscall, etc.). Required because no data is being recorded.
    let elapsed_secs = (input.now - last).num_seconds().max(0) as u64;
    if elapsed_secs >= input.max_expected_gap_secs {
        let elapsed_min = elapsed_secs / 60;
        return CheckResult {
            name: "Poll health".into(),
            passed: false,
            severity: Severity::Required,
            detail: format!("Daemon poll loop stalled — last poll {elapsed_min}m ago (threshold {}m)",
                input.max_expected_gap_secs / 60),
            suggestion: Some("Restart the daemon: `blackbox stop && blackbox start`".into()),
        };
    }

    let elapsed_min = elapsed_secs / 60;

    // Discovery failure = a configured watch_dir is unreadable. Means we never
    // even try to poll those repos, so per-repo failure metrics would silently
    // hide the problem. Required severity.
    if let Some(discovery_failed) = input.discovery_failed_count {
        if discovery_failed > 0 {
            let sample = if input.discovery_failed_sample.is_empty() {
                String::new()
            } else {
                format!(" Sample: {}", input.discovery_failed_sample.join(", "))
            };
            return CheckResult {
                name: "Poll health".into(),
                passed: false,
                severity: Severity::Required,
                detail: format!(
                    "Cannot read {discovery_failed} configured watch dir(s) — repos there are not being discovered.{sample}"
                ),
                suggestion: Some(
                    "Grant the daemon read access (macOS: System Settings → Privacy → Files & Folders) \
                     or remove the entries from watch_dirs in config.toml. Restart daemon afterwards."
                        .into(),
                ),
            };
        }
    }

    // Daemon ran a poll but did not write the failure metric → it's running an
    // older binary that doesn't surface this signal. We can't know whether
    // polling is healthy. Render as Optional warning (not green pass) so users
    // see "unknown" instead of a misleading checkmark.
    let failed = match input.failed_count {
        Some(n) => n,
        None => {
            return CheckResult {
                name: "Poll health".into(),
                passed: false,
                severity: Severity::Optional,
                detail: format!(
                    "Running daemon predates poll-failure metrics — health unknown (last poll {elapsed_min}m ago)"
                ),
                suggestion: Some("Restart daemon to enable per-cycle failure reporting".into()),
            };
        }
    };

    // Every repo failing while daemon stays alive = silent data loss
    // (this is exactly the TCC permission scenario).
    let total = input.repos_watched.max(failed);
    if failed > 0 && failed == total && total > 0 {
        let sample = if input.failed_sample.is_empty() {
            String::new()
        } else {
            format!(" Sample: {}", input.failed_sample.join(", "))
        };
        return CheckResult {
            name: "Poll health".into(),
            passed: false,
            severity: Severity::Required,
            detail: format!("All {total} watched repos failed to poll on last cycle.{sample}"),
            suggestion: Some(
                "Check daemon log: `tail ~/.local/share/blackbox/blackbox.err.log`. \
                 Common cause on macOS: TCC denying file access. Restart daemon to retry."
                    .into(),
            ),
        };
    }

    if failed > 0 {
        let sample = if input.failed_sample.is_empty() {
            String::new()
        } else {
            format!(" ({})", input.failed_sample.join(", "))
        };
        return CheckResult {
            name: "Poll health".into(),
            passed: false,
            severity: Severity::Optional,
            detail: format!("{failed} of {total} repos failing to poll{sample}"),
            suggestion: Some("Check daemon log for per-repo errors.".into()),
        };
    }

    // No repos discovered + no discovery errors = unconfigured. Don't claim
    // "All 0 repos polled successfully" — that misleads users into thinking
    // the daemon is tracking when it has nothing to track.
    if total == 0 {
        return CheckResult {
            name: "Poll health".into(),
            passed: false,
            severity: Severity::Optional,
            detail: "No repos discovered — check `watch_dirs` in config".into(),
            suggestion: Some("Add directories to watch_dirs and reload daemon: `blackbox reload`".into()),
        };
    }

    CheckResult {
        name: "Poll health".into(),
        passed: true,
        severity: Severity::Required,
        detail: format!("All {total} repos polled successfully (last poll {elapsed_min}m ago)"),
        suggestion: None,
    }
}

/// Read poll metrics from the DB and evaluate health.
pub fn check_poll_health(config: &crate::config::Config) -> CheckResult {
    let dir = match crate::config::data_dir() {
        Ok(d) => d,
        Err(e) => {
            return CheckResult {
                name: "Poll health".into(),
                passed: false,
                severity: Severity::Optional,
                detail: format!("Cannot determine data dir: {e}"),
                suggestion: None,
            };
        }
    };
    let db_path = dir.join("blackbox.db");
    let conn = match crate::db::open_db(&db_path) {
        Ok(c) => c,
        Err(_) => {
            return CheckResult {
                name: "Poll health".into(),
                passed: true,
                severity: Severity::Optional,
                detail: "DB unavailable — skipping poll health check".into(),
                suggestion: None,
            };
        }
    };

    let last_poll_at = crate::db::get_daemon_state(&conn, "last_poll_at")
        .ok()
        .flatten()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc));

    let repos_watched = crate::db::get_daemon_state(&conn, "repos_watched")
        .ok()
        .flatten()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);

    let failed_count = crate::db::get_daemon_state(&conn, "last_poll_repos_failed")
        .ok()
        .flatten()
        .and_then(|s| s.parse::<usize>().ok());

    let failed_sample: Vec<String> = crate::db::get_daemon_state(&conn, "last_poll_failed_sample")
        .ok()
        .flatten()
        .map(|s| s.lines().filter(|l| !l.is_empty()).map(String::from).collect())
        .unwrap_or_default();

    let discovery_failed_count = crate::db::get_daemon_state(&conn, "last_poll_discovery_failed")
        .ok()
        .flatten()
        .and_then(|s| s.parse::<usize>().ok());

    let discovery_failed_sample: Vec<String> = crate::db::get_daemon_state(&conn, "last_poll_discovery_failed_sample")
        .ok()
        .flatten()
        .map(|s| s.lines().filter(|l| !l.is_empty()).map(String::from).collect())
        .unwrap_or_default();

    // Stall threshold derived from the mode the daemon last reported. Watcher
    // mode proves liveness only via full_scan (FULL_SCAN_SECS) or events;
    // poll_interval_secs is unused there and folding it into the threshold
    // lets a user with poll_interval_secs=7200 mask a 4hr stall behind a 6hr
    // budget. Polling-mode daemons keep the legacy 3× interval bound.
    // Missing mode key (legacy daemon) → backward-compat max-of-both.
    let poll_mode = crate::db::get_daemon_state(&conn, "last_poll_mode")
        .ok()
        .flatten();
    // Prefer the daemon's own persisted poll_interval over the reader's
    // config. If the user's config.toml is mid-edit / malformed when status
    // or doctor runs, computing thresholds from a default-fallback misclassifies
    // health for any daemon running a non-default interval.
    let effective_interval = crate::db::get_daemon_state(&conn, "effective_poll_interval_secs")
        .ok()
        .flatten()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(config.poll_interval_secs);
    let max_expected_gap_secs =
        stall_threshold_for_mode(poll_mode.as_deref(), effective_interval);

    let input = PollHealthInput {
        last_poll_at,
        repos_watched,
        failed_count,
        failed_sample,
        discovery_failed_count,
        discovery_failed_sample,
        max_expected_gap_secs,
        now: chrono::Utc::now(),
    };
    evaluate_poll_health(&input)
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
    if let Some(ref cfg) = loaded_config {
        results.push(check_poll_health(cfg));
    }
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
    fn check_llm_unknown_provider_fails() {
        // Regression: runtime call_llm_streaming rejects unknown providers.
        // Doctor must fail early so it cannot greenlight a config the runtime
        // will reject on first API call.
        let cfg = cfg_with_llm(Some("whatever-format-key-12345"), Some("weird-provider"));
        let r = check_llm(&cfg);
        assert!(!r.passed, "unknown provider must fail doctor check");
        assert!(r.detail.to_lowercase().contains("unsupported"));
        assert!(r.detail.contains("weird-provider"));
    }

    #[test]
    fn check_llm_shares_provider_list_with_runtime() {
        // Every provider doctor accepts must be one runtime supports.
        for p in crate::llm::SUPPORTED_PROVIDERS {
            assert!(
                crate::llm::is_supported_provider(p),
                "SUPPORTED_PROVIDERS must all pass is_supported_provider"
            );
        }
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
    fn check_llm_auto_detects_openai_from_env() {
        // No explicit provider, only OPENAI_API_KEY set.
        // Must not mis-report as anthropic.
        let cfg = cfg_with_llm(None, None);
        let env = |k: &str| {
            (k == "OPENAI_API_KEY").then(|| "sk-proj-validkeylongenough12345".to_string())
        };
        let r = check_llm_with_env(&cfg, env);
        assert!(r.passed, "OPENAI_API_KEY alone should pass, got: {}", r.detail);
        assert!(r.detail.contains("openai"), "should detect openai provider, got: {}", r.detail);
    }

    #[test]
    fn check_llm_no_key_no_env_is_optional_pass() {
        let cfg = cfg_with_llm(None, Some("anthropic"));
        let r = check_llm_with_env(&cfg, |_| None);
        assert!(r.passed);
        assert_eq!(r.severity, Severity::Optional);
    }

    fn ts(s: &str) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&chrono::Utc)
    }

    #[test]
    fn poll_health_no_poll_yet_renders_as_warning_not_green_pass() {
        // Code-reviewer round 3: pre-first-poll must NOT render as a green
        // checkmark. We have no data to certify the daemon is healthy, so it
        // should be Optional+passed:false (yellow !) for consistency with the
        // legacy-daemon and discovery-error branches.
        let input = PollHealthInput {
            last_poll_at: None,
            repos_watched: 0,
            failed_count: Some(0),
            failed_sample: vec![],
            max_expected_gap_secs: 3600, discovery_failed_count: Some(0), discovery_failed_sample: vec![],
            now: ts("2026-04-28T15:00:00Z"),
        };
        let r = evaluate_poll_health(&input);
        assert!(!r.passed, "no-poll-yet must not render as green");
        assert_eq!(r.severity, Severity::Optional, "must not gate exit code");
        let d = r.detail.to_lowercase();
        assert!(d.contains("no poll") || d.contains("not yet") || d.contains("starting"),
            "detail should signal pre-first-poll state, got: {}", r.detail);
        assert!(r.suggestion.is_some(), "should suggest waiting + retry");
    }

    #[test]
    fn poll_health_zero_repos_zero_failures_is_optional_warning() {
        // General-agent round 3: empty watch_dirs would have shown "All 0
        // repos polled successfully" as green. Fix renders it as a yellow
        // warning so users notice their config is empty.
        let input = PollHealthInput {
            last_poll_at: Some(ts("2026-04-28T14:55:00Z")),
            repos_watched: 0,
            failed_count: Some(0),
            failed_sample: vec![],
            discovery_failed_count: Some(0),
            discovery_failed_sample: vec![],
            max_expected_gap_secs: 3600,
            now: ts("2026-04-28T15:00:00Z"),
        };
        let r = evaluate_poll_health(&input);
        assert!(!r.passed);
        assert_eq!(r.severity, Severity::Optional);
        assert!(r.detail.to_lowercase().contains("no repos") || r.detail.to_lowercase().contains("watch_dirs"));
    }

    #[test]
    fn poll_health_stall_threshold_uses_gte_at_boundary() {
        // impl-reviewer round 3: `> threshold` lets a daemon frozen exactly at
        // max_expected_gap_secs slip through as green. Use `>=`.
        let input = PollHealthInput {
            last_poll_at: Some(ts("2026-04-28T14:00:00Z")),
            repos_watched: 5,
            failed_count: Some(0),
            failed_sample: vec![],
            discovery_failed_count: Some(0),
            discovery_failed_sample: vec![],
            max_expected_gap_secs: 3600, // exactly 60 min
            now: ts("2026-04-28T15:00:00Z"), // exactly 60 min later
        };
        let r = evaluate_poll_health(&input);
        assert!(!r.passed, "exact-threshold gap must trip stall, got: {}", r.detail);
        assert!(r.detail.to_lowercase().contains("stalled"));
    }

    #[test]
    fn poll_health_stalled_is_required_fail() {
        // last poll 2 hours ago, poll_interval = 30 min → 4× interval → stalled
        let input = PollHealthInput {
            last_poll_at: Some(ts("2026-04-28T13:00:00Z")),
            repos_watched: 5,
            failed_count: Some(0),
            failed_sample: vec![],
            max_expected_gap_secs: 3600, discovery_failed_count: Some(0), discovery_failed_sample: vec![],
            now: ts("2026-04-28T15:00:00Z"),
        };
        let r = evaluate_poll_health(&input);
        assert!(!r.passed);
        assert_eq!(r.severity, Severity::Required);
        assert!(r.detail.to_lowercase().contains("stalled"));
    }

    #[test]
    fn poll_health_all_repos_failing_is_required_fail() {
        // Daemon alive (recent poll) but every repo errored — silent data loss.
        // This is the exact scenario the user hit (TCC denying file reads).
        let input = PollHealthInput {
            last_poll_at: Some(ts("2026-04-28T14:50:00Z")),
            repos_watched: 22,
            failed_count: Some(22),
            failed_sample: vec!["/Users/me/code/a".into(), "/Users/me/code/b".into()],
            max_expected_gap_secs: 3600, discovery_failed_count: Some(0), discovery_failed_sample: vec![],
            now: ts("2026-04-28T15:00:00Z"),
        };
        let r = evaluate_poll_health(&input);
        assert!(!r.passed);
        assert_eq!(r.severity, Severity::Required);
        assert!(r.detail.contains("22"));
        assert!(r.suggestion.is_some());
    }

    #[test]
    fn poll_health_partial_failures_is_optional_warn() {
        let input = PollHealthInput {
            last_poll_at: Some(ts("2026-04-28T14:50:00Z")),
            repos_watched: 22,
            failed_count: Some(3),
            failed_sample: vec!["/repo/a".into(), "/repo/b".into(), "/repo/c".into()],
            max_expected_gap_secs: 3600, discovery_failed_count: Some(0), discovery_failed_sample: vec![],
            now: ts("2026-04-28T15:00:00Z"),
        };
        let r = evaluate_poll_health(&input);
        assert!(!r.passed);
        assert_eq!(r.severity, Severity::Optional);
        assert!(r.detail.contains("3"));
        assert!(r.detail.contains("22"));
    }

    #[test]
    fn poll_health_all_passing_is_pass() {
        let input = PollHealthInput {
            last_poll_at: Some(ts("2026-04-28T14:55:00Z")),
            repos_watched: 5,
            failed_count: Some(0),
            failed_sample: vec![],
            max_expected_gap_secs: 3600, discovery_failed_count: Some(0), discovery_failed_sample: vec![],
            now: ts("2026-04-28T15:00:00Z"),
        };
        let r = evaluate_poll_health(&input);
        assert!(r.passed);
        assert_eq!(r.severity, Severity::Required);
        assert!(r.detail.contains("5"));
    }

    #[test]
    fn poll_health_legacy_daemon_without_metrics_renders_as_warning() {
        // Codex finding [high]: do NOT render legacy daemon as a green pass.
        // Pre-fix, run_doctor renders every passed:true with a green checkmark
        // and excludes it from the optional-fail tally. That defeats the
        // safety check this PR is adding. The legacy state must render as a
        // yellow warning (passed:false + Severity::Optional).
        let input = PollHealthInput {
            last_poll_at: Some(ts("2026-04-28T14:55:00Z")),
            repos_watched: 22,
            failed_count: None,
            failed_sample: vec![],
            max_expected_gap_secs: 3600, discovery_failed_count: Some(0), discovery_failed_sample: vec![],
            now: ts("2026-04-28T15:00:00Z"),
        };
        let r = evaluate_poll_health(&input);
        assert!(!r.passed, "legacy/unknown state must NOT render as green pass");
        assert_eq!(r.severity, Severity::Optional,
            "must be Optional so exit code is unaffected, not Required (red)");
        let d = r.detail.to_lowercase();
        assert!(d.contains("unknown") || d.contains("predates"),
            "detail should signal unknown health, got: {}", r.detail);
        assert!(r.suggestion.is_some(), "should suggest a restart");
    }

    #[test]
    fn poll_health_watcher_mode_idle_at_25_min_not_stalled() {
        // Codex round 2 [high]: watcher mode bumps last_poll_at on full_scan
        // (every 30 min) plus events. With poll_interval_secs=300 a 25-minute
        // gap was being flagged as Required stalled, which would block doctor
        // on healthy idle daemons. Stall threshold must use FULL_SCAN_SECS.
        let input = PollHealthInput {
            last_poll_at: Some(ts("2026-04-28T14:35:00Z")), // 25m ago
            repos_watched: 5,
            failed_count: Some(0),
            failed_sample: vec![],
            discovery_failed_count: Some(0),
            discovery_failed_sample: vec![],
            // Caller picks max(3*300, 2*1800) = 3600
            max_expected_gap_secs: 3600,
            now: ts("2026-04-28T15:00:00Z"),
        };
        let r = evaluate_poll_health(&input);
        assert!(r.passed, "25min idle in watcher mode must not be stalled, got: {}", r.detail);
    }

    #[test]
    fn poll_health_actually_stalled_past_threshold() {
        // Beyond the 1-hour watcher-mode threshold → still Required fail.
        let input = PollHealthInput {
            last_poll_at: Some(ts("2026-04-28T13:30:00Z")), // 90 min ago
            repos_watched: 5,
            failed_count: Some(0),
            failed_sample: vec![],
            discovery_failed_count: Some(0),
            discovery_failed_sample: vec![],
            max_expected_gap_secs: 3600,
            now: ts("2026-04-28T15:00:00Z"),
        };
        let r = evaluate_poll_health(&input);
        assert!(!r.passed);
        assert_eq!(r.severity, Severity::Required);
        assert!(r.detail.to_lowercase().contains("stalled"));
    }

    #[test]
    fn poll_health_discovery_errors_are_required_fail() {
        // Codex round 2 [high]: full_scan can erase failures when
        // discover_repos silently drops unreadable watch_dirs. doctor must
        // surface "couldn't read N watch dirs" as Required, otherwise the
        // exact permission scenario this PR exists to catch slips through.
        let input = PollHealthInput {
            last_poll_at: Some(ts("2026-04-28T14:55:00Z")),
            repos_watched: 0, // discovery returned nothing!
            failed_count: Some(0), // and "0 failed" looks healthy
            failed_sample: vec![],
            discovery_failed_count: Some(2),
            discovery_failed_sample: vec![
                "/Users/me/Documents/flosports".into(),
                "/Users/me/Documents/personal".into(),
            ],
            max_expected_gap_secs: 3600,
            now: ts("2026-04-28T15:00:00Z"),
        };
        let r = evaluate_poll_health(&input);
        assert!(!r.passed, "discovery errors must NOT show as healthy green");
        assert_eq!(r.severity, Severity::Required);
        assert!(r.detail.to_lowercase().contains("watch dir"));
        assert!(r.detail.contains("/Users/me/Documents/flosports"));
    }

    #[test]
    fn poll_health_legacy_daemon_no_discovery_metric_falls_through() {
        // Daemon predates discovery metric (None) → should flow to legacy
        // failed_count handling, not block exit.
        let input = PollHealthInput {
            last_poll_at: Some(ts("2026-04-28T14:55:00Z")),
            repos_watched: 5,
            failed_count: None,
            failed_sample: vec![],
            discovery_failed_count: None,
            discovery_failed_sample: vec![],
            max_expected_gap_secs: 3600,
            now: ts("2026-04-28T15:00:00Z"),
        };
        let r = evaluate_poll_health(&input);
        // Legacy state: warning, not Required.
        assert!(!r.passed);
        assert_eq!(r.severity, Severity::Optional);
    }

    #[test]
    fn watcher_threshold_is_two_full_scans() {
        assert_eq!(watcher_stall_threshold_secs(), crate::poller::FULL_SCAN_SECS * 2);
    }

    #[test]
    fn polling_threshold_is_3x_interval() {
        assert_eq!(polling_stall_threshold_secs(300), 900);
    }

    #[test]
    fn polling_threshold_floors_at_120s() {
        // Misconfigured 1s interval must not let stall fire instantly.
        assert_eq!(polling_stall_threshold_secs(1), 120);
        assert_eq!(polling_stall_threshold_secs(40), 120);
    }

    #[test]
    fn stall_threshold_watcher_mode_ignores_large_poll_interval() {
        // Watcher daemon, user has poll_interval_secs=7200 (2hr). Threshold
        // must still be 2*FULL_SCAN_SECS = 1hr, NOT 6hr.
        let t = stall_threshold_for_mode(Some("watcher"), 7200);
        assert_eq!(t, watcher_stall_threshold_secs());
    }

    #[test]
    fn stall_threshold_polling_mode_uses_interval() {
        let t = stall_threshold_for_mode(Some("polling"), 600);
        assert_eq!(t, 1800);
    }

    #[test]
    fn stall_threshold_missing_mode_is_backward_compat_max() {
        // Legacy daemon predating last_poll_mode: keep current behavior so the
        // existing watcher-idle leniency isn't lost during rollout.
        let t = stall_threshold_for_mode(None, 300);
        let expected = std::cmp::max(900, crate::poller::FULL_SCAN_SECS * 2);
        assert_eq!(t, expected);
    }

    #[test]
    fn stall_threshold_unrecognized_mode_falls_back() {
        let t = stall_threshold_for_mode(Some("garbage"), 300);
        let expected = std::cmp::max(900, crate::poller::FULL_SCAN_SECS * 2);
        assert_eq!(t, expected);
    }

    #[test]
    fn poll_health_sample_paths_appear_in_suggestion() {
        let input = PollHealthInput {
            last_poll_at: Some(ts("2026-04-28T14:55:00Z")),
            repos_watched: 4,
            failed_count: Some(4),
            failed_sample: vec!["/repo/alpha".into()],
            max_expected_gap_secs: 3600, discovery_failed_count: Some(0), discovery_failed_sample: vec![],
            now: ts("2026-04-28T15:00:00Z"),
        };
        let r = evaluate_poll_health(&input);
        let blob = format!("{} {}", r.detail, r.suggestion.unwrap_or_default());
        assert!(blob.contains("/repo/alpha"), "sample paths should surface to user");
    }

    #[test]
    fn check_daemon_is_required_severity() {
        // Regression guard: daemon-down means silent data loss for a
        // tracker that depends on background polling. Must stay Required
        // so `doctor` surfaces it as a blocking failure.
        let r = check_daemon();
        assert_eq!(
            r.severity,
            Severity::Required,
            "check_daemon severity must remain Required to catch stopped daemons"
        );
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
