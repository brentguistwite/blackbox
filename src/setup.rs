use std::path::{Path, PathBuf};

use colored::Colorize;
use dialoguer::theme::ColorfulTheme;

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

/// Format "N repo(s)" label.
fn repo_count_label(n: usize) -> String {
    format!("{} repo{}", n, if n == 1 { "" } else { "s" })
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

/// Format a step indicator string: "[n/total] label"
pub fn format_step(n: u8, total: u8, label: &str) -> String {
    format!(
        "\n{} {}",
        format!("[{}/{}]", n, total).cyan().bold(),
        label.bold()
    )
}

/// Calculate total setup steps based on platform.
/// Steps: 1=scan, 2=select, 3=worktree dir, 4=shell hook, 5=service (macOS/Linux only)
pub fn total_setup_steps() -> u8 {
    let has_service = cfg!(target_os = "macos") || cfg!(target_os = "linux");
    if has_service { 5 } else { 4 }
}

/// Run the full interactive setup wizard.
pub fn run_setup() -> anyhow::Result<()> {
    // Styled header
    println!();
    println!("{}", "  blackbox setup  ".bold().on_bright_black().white());
    println!("{}", "  flight recorder for your dev day".dimmed());
    println!();

    let total = total_setup_steps();

    // Try loading existing config for scan_dirs
    let existing_config = config::load_config().ok();

    // === Step 1: Scan for repositories ===
    println!("{}", format_step(1, total, "Scan for repositories"));

    let mut scan_dirs: Vec<PathBuf> = Vec::new();
    let mut all_repos: Vec<PathBuf> = Vec::new();

    match existing_config.as_ref().and_then(|c| c.scan_dirs.as_ref()) {
        Some(dirs) if !dirs.is_empty() => {
            println!("  Scanning previously configured directories...");
            scan_dirs = dirs.clone();
            for dir in &scan_dirs {
                let repos = repo_scanner::scan_directory(dir);
                let count_label = repo_count_label(repos.len());
                println!(
                    "  {} {count_label} in {}",
                    "found".green(),
                    contract_tilde(dir).dimmed()
                );
                all_repos.extend(repos);
            }
        }
        Some(_) => {
            println!("  {}", "No scan directories configured.".dimmed());
        }
        None => {
            // Fresh setup or legacy config -- auto-scan
            let scan_results = repo_scanner::auto_scan_repos();
            for (parent, repos) in &scan_results {
                let count_label = repo_count_label(repos.len());
                println!(
                    "  {} {count_label} in {}",
                    "found".green(),
                    contract_tilde(parent).dimmed()
                );
                scan_dirs.push(parent.clone());
                all_repos.extend(repos.iter().cloned());
            }
            if all_repos.is_empty() {
                println!("  {}", "No repos found in common directories.".dimmed());
            } else {
                println!(
                    "  {} total repos found.",
                    all_repos.len().to_string().green().bold()
                );
            }
        }
    }

    // "Scan another directory?" loop -- BEFORE MultiSelect
    loop {
        let add_more = dialoguer::Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt("Scan another directory?")
            .interact()?;

        if !add_more {
            break;
        }

        let path_input: String = dialoguer::Input::with_theme(&ColorfulTheme::default())
            .with_prompt("Directory path")
            .interact_text()?;

        let expanded = expand_tilde(&path_input);
        if expanded.is_dir() {
            let repos = repo_scanner::scan_directory(&expanded);
            println!(
                "  {} {} repo{}",
                "found".green(),
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

    // === Step 2: Select repos to watch ===
    println!("{}", format_step(2, total, "Select repos to watch"));

    let selected_repos: Vec<PathBuf> = if all_repos.is_empty() {
        println!(
            "  {}",
            "No git repos found in scanned directories.".dimmed()
        );
        Vec::new()
    } else {
        let items: Vec<String> = all_repos.iter().map(|p| contract_tilde(p)).collect();
        let defaults: Vec<bool> = vec![true; items.len()];

        println!(
            "  {}",
            "\u{2191}/\u{2193} navigate  \u{00b7}  space select  \u{00b7}  enter confirm".dimmed()
        );

        let selections = dialoguer::MultiSelect::with_theme(&ColorfulTheme::default())
            .with_prompt("Select repos to watch")
            .items(&items)
            .defaults(&defaults)
            .interact()?;

        selections
            .iter()
            .map(|&idx| all_repos[idx].clone())
            .collect()
    };

    if selected_repos.is_empty() {
        println!(
            "  {}",
            "No repos selected -- you can edit config.toml later.".dimmed()
        );
    }

    // === Step 3: Worktree directory ===
    println!("{}", format_step(3, total, "Worktree directory"));
    println!(
        "  {}",
        "Directory name inside repos where worktrees are created".dimmed()
    );

    let worktree_input: String = dialoguer::Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Worktree dir name")
        .default(".worktrees".to_string())
        .allow_empty(true)
        .interact_text()?;

    let worktree_dir_name = if worktree_input.is_empty() {
        println!("  {}", "Worktree watching disabled".dimmed());
        None
    } else {
        println!("  Worktree dir: {}", worktree_input.bold());
        Some(worktree_input)
    };

    // === Step 4: Shell hook ===
    println!("{}", format_step(4, total, "Shell hook"));

    let mut hook_installed = false;
    match detect_shell_type() {
        ShellDetection::Supported { name, rc_path } => {
            let line = hook_eval_line(&name);

            println!("  Detected shell: {}", name.bold());
            println!(
                "  {}",
                "Tracks cd into watched repos via shell hook".dimmed()
            );

            // Check if already present
            let already = rc_path
                .exists()
                .then(|| std::fs::read_to_string(&rc_path).unwrap_or_default())
                .map(|c| c.contains("blackbox hook"))
                .unwrap_or(false);

            if already {
                println!(
                    "  Shell hook already in {}",
                    contract_tilde(&rc_path).dimmed()
                );
                hook_installed = true;
            } else {
                println!("  This line will be added to {}:", contract_tilde(&rc_path));
                println!("    {}", line.dimmed());

                let install_hook = dialoguer::Confirm::with_theme(&ColorfulTheme::default())
                    .with_prompt("Install shell hook?")
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
                    // We don't inline the hook script -- eval loads it at shell startup
                    let _ = hook_script; // just validated it generates ok
                    std::fs::write(&rc_path, contents)?;
                    println!(
                        "  {} Added to {}",
                        "\u{2713}".green(),
                        contract_tilde(&rc_path)
                    );
                    hook_installed = true;
                }
            }
        }
        ShellDetection::Unsupported(shell_name) => {
            println!("  {}", unsupported_shell_message(&shell_name));
        }
        ShellDetection::NotSet => {
            println!("  {}", notset_shell_message());
        }
    }

    // === Step 4: OS service (macOS/Linux only) ===
    let mut service_registered = false;
    let os_name = if cfg!(target_os = "macos") {
        "launchd"
    } else if cfg!(target_os = "linux") {
        "systemd"
    } else {
        ""
    };

    if !os_name.is_empty() {
        println!("{}", format_step(5, total, "Background service"));
        println!(
            "  {}",
            "Starts blackbox on login, runs in background".dimmed()
        );

        let install_service = dialoguer::Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt(format!("Register {} service?", os_name))
            .interact()?;

        if install_service {
            // Config must exist before install since service runs run-foreground
            // which loads config -- we'll write config first, then install
            service_registered = true;
        }
    }

    // === Write config ===
    let config = Config {
        watch_dirs: selected_repos.clone(),
        scan_dirs: Some(scan_dirs),
        worktree_dir_name,
        ..Config::default()
    };

    let config_path = config::config_dir()?.join("config.toml");
    config.save_to(&config_path)?;

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
                println!(
                    "  Service registration failed: {}. You can try `blackbox install` later.",
                    e
                );
                service_registered = false;
            }
        }
    }

    // === Start daemon (if no service -- service auto-starts) ===
    if !service_registered {
        let mut loaded_config = config::load_config()?;
        loaded_config.expand_paths();
        match crate::daemon::start_daemon(loaded_config, &data_dir) {
            Ok(()) => {} // daemon forks and parent returns
            Err(e) => println!(
                "  Could not start daemon: {}. Run `blackbox start` later.",
                e
            ),
        }
    }

    // === Summary ===
    println!();
    println!("{}", "  Setup complete  ".bold().on_green().black());
    println!();

    // Checkmark/cross for each component
    let (repo_icon, repo_text) = if selected_repos.is_empty() {
        (
            "\u{2717}".red().to_string(),
            "none (edit config.toml)".to_string(),
        )
    } else {
        (
            "\u{2713}".green().to_string(),
            format!("{} repos", selected_repos.len()),
        )
    };
    println!("  {} Repos: {}", repo_icon, repo_text);

    let (wt_icon, wt_text) = match &config.worktree_dir_name {
        Some(d) if !d.is_empty() => ("\u{2713}".green().to_string(), d.clone()),
        _ => ("\u{2717}".red().to_string(), "disabled".to_string()),
    };
    println!("  {} Worktree dir: {}", wt_icon, wt_text);

    let (hook_icon, hook_text) = if hook_installed {
        ("\u{2713}".green().to_string(), "installed")
    } else {
        ("\u{2717}".red().to_string(), "skipped")
    };
    println!("  {} Shell hook: {}", hook_icon, hook_text);

    let (svc_icon, svc_text) = if service_registered {
        ("\u{2713}".green().to_string(), "registered")
    } else if os_name.is_empty() {
        ("\u{2014}".dimmed().to_string(), "n/a")
    } else {
        ("\u{2717}".red().to_string(), "not registered")
    };
    println!("  {} Service: {}", svc_icon, svc_text);

    println!("\n  Config: {}", contract_tilde(&config_path).dimmed());

    println!("\n  {}:", "Next steps".bold());
    if !hook_installed {
        println!(
            "    {}",
            "blackbox hook zsh  # install shell hook manually".dimmed()
        );
    }
    println!("    {}", "blackbox doctor     # check health".dimmed());
    println!("    {}", "blackbox today      # view activity".dimmed());

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
