use std::path::{Path, PathBuf};

use crate::config::{self, Config};
use crate::db;
use crate::repo_scanner;
use crate::shell_hook;

/// Shell detection result for setup wizard.
#[derive(Debug)]
pub enum ShellDetection {
    Supported { name: String, rc_path: PathBuf },
    Unsupported(String),
    NotSet,
}

/// Detect the user's shell from $SHELL and classify it.
pub fn detect_shell_type() -> ShellDetection {
    let shell_var = std::env::var("SHELL").ok();
    detect_shell_type_from(shell_var.as_deref())
}

/// Testable core: classify a shell variable value.
pub fn detect_shell_type_from(shell_var: Option<&str>) -> ShellDetection {
    match shell_var {
        None => ShellDetection::NotSet,
        Some("") => ShellDetection::NotSet,
        Some(s) => {
            let name = s.rsplit('/').next().unwrap_or(s).to_string();
            match rc_file_for_shell(&name) {
                Some(rc_path) => ShellDetection::Supported { name, rc_path },
                None => ShellDetection::Unsupported(name),
            }
        }
    }
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

/// Descriptive comment block for rc file.
pub fn hook_comment_block(shell: &str) -> String {
    format!(
        "# blackbox — tracks which repos you're working in for time estimation\n\
         # remove this line to disable, or run: blackbox setup\n\
         {}",
        hook_eval_line(shell)
    )
}

/// Message for unsupported shell detection.
pub fn unsupported_shell_message(shell_name: &str) -> String {
    format!(
        "Detected shell: {}\n\
         Auto-install only supports zsh, bash, and fish.\n\
         To set up manually, add this to your shell config:\n  \
         eval \"$(blackbox hook {})\"",
        shell_name, shell_name
    )
}

/// Message when $SHELL is not set.
pub fn notset_shell_message() -> String {
    "Could not detect your shell ($SHELL is not set).\n\
     Set $SHELL or manually add the hook to your shell config:\n  \
     eval \"$(blackbox hook <your-shell>)\""
        .to_string()
}

/// Contract an absolute path with ~ for display.
fn contract_tilde(path: &Path) -> String {
    if let Ok(home) = etcetera::home_dir()
        && let Ok(suffix) = path.strip_prefix(&home)
    {
        return format!("~/{}", suffix.display());
    }
    path.display().to_string()
}

/// Run the full interactive setup wizard.
pub fn run_setup() -> anyhow::Result<()> {
    println!("blackbox setup wizard\n");

    // Try loading existing config for scan_dirs
    let existing_config = config::load_config().ok();

    // === Step 1: Collect scan dirs and accumulate repos ===
    let mut scan_dirs: Vec<PathBuf> = Vec::new();
    let mut all_repos: Vec<PathBuf> = Vec::new();

    match existing_config.as_ref().and_then(|c| c.scan_dirs.as_ref()) {
        Some(dirs) if !dirs.is_empty() => {
            println!("Scanning previously configured directories...");
            scan_dirs = dirs.clone();
            for dir in &scan_dirs {
                all_repos.extend(repo_scanner::scan_directory(dir));
            }
        }
        Some(_) => {
            // scan_dirs is Some([]) — skip auto-scan
            println!("No scan directories configured.");
        }
        None => {
            // Fresh setup or legacy config — auto-scan
            println!("Scanning for git repos...");
            let scan_results = repo_scanner::auto_scan_repos();
            for (parent, repos) in &scan_results {
                scan_dirs.push(parent.clone());
                all_repos.extend(repos.iter().cloned());
            }
        }
    }

    // "Scan another directory?" loop — BEFORE MultiSelect
    loop {
        let add_more = dialoguer::Confirm::new()
            .with_prompt("Scan another directory?")
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
            let repos = repo_scanner::scan_directory(&expanded);
            println!(
                "  Found {} repo{}",
                repos.len(),
                if repos.len() == 1 { "" } else { "s" }
            );
            all_repos.extend(repos);
            scan_dirs.push(expanded);
        } else {
            println!("  Not a valid directory: {}", path_input);
        }
    }

    // Dedup repos
    all_repos.sort();
    all_repos.dedup();

    // === Step 2: Select individual repos ===
    let selected_repos: Vec<PathBuf> = if all_repos.is_empty() {
        println!("No git repos found in scanned directories.");
        Vec::new()
    } else {
        let items: Vec<String> = all_repos.iter().map(|p| contract_tilde(p)).collect();
        let defaults: Vec<bool> = vec![true; items.len()];

        let selections = dialoguer::MultiSelect::new()
            .with_prompt("Select repos to watch")
            .items(&items)
            .defaults(&defaults)
            .interact()?;

        selections.iter().map(|&idx| all_repos[idx].clone()).collect()
    };

    if selected_repos.is_empty() {
        println!("No repos selected — you can edit config.toml later.");
    }

    // === Step 3: Shell hook ===
    let mut hook_installed = false;
    match detect_shell_type() {
        ShellDetection::Supported { name, rc_path } => {
            let line = hook_eval_line(&name);

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
                    let hook_script = shell_hook::generate_hook(&name)?;
                    let mut contents = if rc_path.exists() {
                        std::fs::read_to_string(&rc_path)?
                    } else {
                        String::new()
                    };
                    if !contents.ends_with('\n') && !contents.is_empty() {
                        contents.push('\n');
                    }
                    contents.push_str(&format!("\n{}\n", hook_comment_block(&name)));
                    // We don't inline the hook script — eval loads it at shell startup
                    let _ = hook_script; // just validated it generates ok
                    std::fs::write(&rc_path, contents)?;
                    println!("  Added to {}", rc_path.display());
                    hook_installed = true;
                }
            }
        }
        ShellDetection::Unsupported(shell_name) => {
            println!("\n{}", unsupported_shell_message(&shell_name));
        }
        ShellDetection::NotSet => {
            println!("\n{}", notset_shell_message());
        }
    }

    // === Step 3: OS service ===
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
        watch_dirs: selected_repos.clone(),
        scan_dirs: Some(scan_dirs),
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
        "  Repos: {}",
        if selected_repos.is_empty() {
            "none (edit config.toml)".to_string()
        } else {
            format!("{}", selected_repos.len())
        }
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
