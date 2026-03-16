use std::path::PathBuf;

use crate::config::{self, Config};
use crate::db;
use crate::repo_scanner;
use crate::shell_hook;

const POLL_INTERVAL_OPTIONS: &[(u64, &str)] = &[
    (60, "1 minute"),
    (120, "2 minutes"),
    (300, "5 minutes (recommended)"),
    (600, "10 minutes"),
    (900, "15 minutes"),
];

/// Detect the user's current shell from $SHELL env var.
fn detect_shell() -> Option<String> {
    std::env::var("SHELL").ok().and_then(|s| {
        let name = s.rsplit('/').next()?.to_string();
        match name.as_str() {
            "zsh" | "bash" | "fish" => Some(name),
            _ => None,
        }
    })
}

/// Get the rc file path for a given shell.
fn rc_file_for_shell(shell: &str) -> Option<PathBuf> {
    let home = etcetera::home_dir().ok()?;
    match shell {
        "zsh" => Some(home.join(".zshrc")),
        "bash" => Some(home.join(".bashrc")),
        "fish" => Some(home.join(".config/fish/config.fish")),
        _ => None,
    }
}

/// The hook eval line to add to the rc file.
fn hook_eval_line(shell: &str) -> String {
    format!("eval \"$(blackbox hook {})\"", shell)
}

/// Run the full interactive setup wizard.
pub fn run_setup() -> anyhow::Result<()> {
    println!("blackbox setup wizard\n");

    // === Step 1: Scan and select directories ===
    println!("Scanning for git repos...");
    let scan_results = repo_scanner::auto_scan_repos();

    let mut selected_dirs: Vec<PathBuf> = Vec::new();

    if scan_results.is_empty() {
        println!("No git repos found in common directories.");
    } else {
        let items: Vec<String> = scan_results
            .iter()
            .map(|(dir, repos)| {
                format!(
                    "{} ({} repo{})",
                    dir.display(),
                    repos.len(),
                    if repos.len() == 1 { "" } else { "s" }
                )
            })
            .collect();

        let defaults: Vec<bool> = vec![true; items.len()];

        let selections = dialoguer::MultiSelect::new()
            .with_prompt("Select directories to watch")
            .items(&items)
            .defaults(&defaults)
            .interact()?;

        for idx in selections {
            selected_dirs.push(scan_results[idx].0.clone());
        }
    }

    // Step 1b: Add more directories
    loop {
        let add_more = dialoguer::Confirm::new()
            .with_prompt("Add another directory?")
            .default(false)
            .interact()?;

        if !add_more {
            break;
        }

        let path_input: String = dialoguer::Input::new()
            .with_prompt("Directory path")
            .interact_text()?;

        let expanded = expand_tilde(&path_input);
        if expanded.is_dir() {
            selected_dirs.push(expanded);
        } else {
            println!("  Not a valid directory: {}", path_input);
        }
    }

    if selected_dirs.is_empty() {
        println!("No directories selected — you can edit config.toml later.");
    }

    // === Step 2: Poll interval ===
    let interval_labels: Vec<&str> = POLL_INTERVAL_OPTIONS.iter().map(|(_, l)| *l).collect();
    let default_idx = POLL_INTERVAL_OPTIONS
        .iter()
        .position(|(v, _)| *v == 300)
        .unwrap_or(2);

    let interval_idx = dialoguer::Select::new()
        .with_prompt("Poll interval")
        .items(&interval_labels)
        .default(default_idx)
        .interact()?;

    let poll_interval = POLL_INTERVAL_OPTIONS[interval_idx].0;

    // === Step 3: Shell hook ===
    let mut hook_installed = false;
    if let Some(shell) = detect_shell()
        && let Some(rc_path) = rc_file_for_shell(&shell)
    {
        let line = hook_eval_line(&shell);

        // Check if already present
        let already = rc_path
            .exists()
            .then(|| std::fs::read_to_string(&rc_path).unwrap_or_default())
            .map(|c| c.contains("blackbox hook"))
            .unwrap_or(false);

        if already {
            println!("\nShell hook already in {}", rc_path.display());
            hook_installed = true;
        } else {
            println!("\nThis line will be added to {}:", rc_path.display());
            println!("  {}", line);

            let install_hook = dialoguer::Confirm::new()
                .with_prompt("Install shell hook?")
                .default(true)
                .interact()?;

            if install_hook {
                let hook_script = shell_hook::generate_hook(&shell)?;
                let mut contents = if rc_path.exists() {
                    std::fs::read_to_string(&rc_path)?
                } else {
                    String::new()
                };
                if !contents.ends_with('\n') && !contents.is_empty() {
                    contents.push('\n');
                }
                contents.push_str(&format!("\n# blackbox directory tracking\n{}\n", line));
                // We don't inline the hook script — eval loads it at shell startup
                let _ = hook_script; // just validated it generates ok
                std::fs::write(&rc_path, contents)?;
                println!("  Added to {}", rc_path.display());
                hook_installed = true;
            }
        }
    }

    // === Step 4: OS service ===
    let mut service_registered = false;
    let os_name = if cfg!(target_os = "macos") {
        "launchd"
    } else if cfg!(target_os = "linux") {
        "systemd"
    } else {
        ""
    };

    if !os_name.is_empty() {
        println!();
        println!(
            "Register as {} service? This starts blackbox automatically on login.",
            os_name
        );
        let install_service = dialoguer::Confirm::new()
            .with_prompt("Register service?")
            .default(false)
            .interact()?;

        if install_service {
            // Config must exist before install since service runs run-foreground
            // which loads config — we'll write config first, then install
            service_registered = true;
        }
    }

    // === Write config ===
    let config = Config {
        watch_dirs: selected_dirs.clone(),
        poll_interval_secs: poll_interval,
        ..Config::default()
    };

    let config_path = config::config_dir()?.join("config.toml");
    config.save_to(&config_path)?;
    println!("\nConfig saved to {}", config_path.display());

    // === Create DB ===
    let data_dir = config::data_dir()?;
    std::fs::create_dir_all(&data_dir)?;
    let db_path = data_dir.join("blackbox.db");
    let _conn = db::open_db(&db_path)?;

    // === Register service if requested ===
    if service_registered {
        match crate::service::install() {
            Ok(()) => {}
            Err(e) => {
                println!("Service registration failed: {}. You can try `blackbox install` later.", e);
                service_registered = false;
            }
        }
    }

    // === Start daemon (if no service — service auto-starts) ===
    if !service_registered {
        let mut loaded_config = config::load_config()?;
        loaded_config.expand_paths();
        match crate::daemon::start_daemon(loaded_config, &data_dir) {
            Ok(()) => {} // daemon forks and parent returns
            Err(e) => println!("Could not start daemon: {}. Run `blackbox start` later.", e),
        }
    }

    // === Summary ===
    println!("\n--- Setup complete ---");
    println!(
        "  Watch dirs: {}",
        if selected_dirs.is_empty() {
            "none (edit config.toml)".to_string()
        } else {
            format!("{}", selected_dirs.len())
        }
    );
    println!(
        "  Poll interval: {}",
        POLL_INTERVAL_OPTIONS[interval_idx].1
    );
    println!(
        "  Shell hook: {}",
        if hook_installed {
            "installed"
        } else {
            "skipped"
        }
    );
    println!(
        "  Service: {}",
        if service_registered {
            "registered"
        } else {
            "not registered"
        }
    );
    println!("\nNext steps:");
    if !hook_installed {
        println!("  - Install shell hook: eval \"$(blackbox hook zsh)\"");
    }
    println!("  - Check health: blackbox doctor");
    println!("  - View activity: blackbox today");

    Ok(())
}

fn expand_tilde(path: &str) -> PathBuf {
    if (path.starts_with("~/") || path == "~")
        && let Ok(home) = etcetera::home_dir()
    {
        return home.join(path.strip_prefix("~/").unwrap_or(""));
    }
    PathBuf::from(path)
}
