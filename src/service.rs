use std::path::PathBuf;

use crate::config;
use crate::daemon;

const LABEL: &str = "com.blackbox.agent";

/// Format an error message for a failed service command, including stderr.
fn format_command_error(cmd_desc: &str, stderr: &[u8]) -> String {
    let stderr_str = String::from_utf8_lossy(stderr);
    let stderr_trimmed = stderr_str.trim();
    if stderr_trimmed.is_empty() {
        format!("{cmd_desc} failed (no stderr)")
    } else {
        format!("{cmd_desc} failed: {stderr_trimmed}")
    }
}

pub fn generate_plist(exe_path: &str, data_dir: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe}</string>
        <string>run-foreground</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>EnvironmentVariables</key>
    <dict>
        <key>PATH</key>
        <string>{path}</string>
    </dict>
    <key>StandardOutPath</key>
    <string>{data}/blackbox.log</string>
    <key>StandardErrorPath</key>
    <string>{data}/blackbox.err.log</string>
</dict>
</plist>"#,
        label = LABEL,
        exe = exe_path,
        data = data_dir,
        path = std::env::var("PATH")
            .unwrap_or_else(|_| "/usr/local/bin:/opt/homebrew/bin:/usr/bin:/bin".to_string()),
    )
}

pub fn generate_unit_file(exe_path: &str) -> String {
    format!(
        r#"[Unit]
Description=Blackbox git activity recorder
After=default.target

[Service]
Type=simple
ExecStart={exe} run-foreground
Restart=on-failure
RestartSec=10

[Install]
WantedBy=default.target"#,
        exe = exe_path,
    )
}

pub fn plist_path() -> PathBuf {
    let home = etcetera::home_dir().expect("Cannot determine home directory");
    home.join("Library/LaunchAgents/com.blackbox.agent.plist")
}

pub fn unit_path() -> PathBuf {
    let home = etcetera::home_dir().expect("Cannot determine home directory");
    home.join(".config/systemd/user/blackbox.service")
}

#[cfg(target_os = "macos")]
pub fn install() -> anyhow::Result<()> {
    let data_dir = config::data_dir()?;
    std::fs::create_dir_all(&data_dir)?;

    // Stop any manually-started daemon first
    if daemon::is_daemon_running(&data_dir)?.is_some() {
        daemon::stop_daemon(&data_dir)?;
    }

    let exe = std::env::current_exe()?
        .canonicalize()?
        .to_string_lossy()
        .to_string();

    let plist_content = generate_plist(&exe, &data_dir.to_string_lossy());
    let path = plist_path();

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, &plist_content)?;

    let uid = nix::unistd::getuid().as_raw();
    let path_str = path.to_string_lossy().to_string();

    // Try modern bootstrap, fallback to legacy load
    let output = std::process::Command::new("launchctl")
        .args(["bootstrap", &format!("gui/{}", uid), &path_str])
        .output()?;

    if !output.status.success() {
        let fallback = std::process::Command::new("launchctl")
            .args(["load", &path_str])
            .output()?;
        if !fallback.status.success() {
            anyhow::bail!(
                "{}",
                format_command_error("launchctl load", &fallback.stderr)
            );
        }
    }

    println!("Service installed and loaded (launchd)");
    println!("Plist: {}", path_str);
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn install() -> anyhow::Result<()> {
    let data_dir = config::data_dir()?;
    if daemon::is_daemon_running(&data_dir)?.is_some() {
        daemon::stop_daemon(&data_dir)?;
    }

    let exe = std::env::current_exe()?
        .canonicalize()?
        .to_string_lossy()
        .to_string();

    let unit_content = generate_unit_file(&exe);
    let path = unit_path();

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, &unit_content)?;

    let reload = std::process::Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .output()?;
    if !reload.status.success() {
        anyhow::bail!(
            "{}",
            format_command_error("systemctl daemon-reload", &reload.stderr)
        );
    }

    let enable = std::process::Command::new("systemctl")
        .args(["--user", "enable", "--now", "blackbox.service"])
        .output()?;
    if !enable.status.success() {
        anyhow::bail!(
            "{}",
            format_command_error("systemctl enable", &enable.stderr)
        );
    }

    println!("Service installed and enabled (systemd)");
    println!("Unit: {}", path.display());
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn install() -> anyhow::Result<()> {
    anyhow::bail!("Service install not supported on this OS");
}

#[cfg(target_os = "macos")]
pub fn uninstall() -> anyhow::Result<()> {
    let path = plist_path();
    if !path.exists() {
        println!("Service not installed");
        return Ok(());
    }

    let uid = nix::unistd::getuid().as_raw();
    let path_str = path.to_string_lossy().to_string();

    // Try modern bootout, fallback to legacy unload
    let output = std::process::Command::new("launchctl")
        .args(["bootout", &format!("gui/{}", uid), &path_str])
        .output()?;

    if !output.status.success() {
        let fallback = std::process::Command::new("launchctl")
            .args(["unload", &path_str])
            .output()?;
        if !fallback.status.success() {
            anyhow::bail!(
                "{}",
                format_command_error("launchctl unload", &fallback.stderr)
            );
        }
    }

    std::fs::remove_file(&path)?;
    println!("Service uninstalled (launchd)");
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn uninstall() -> anyhow::Result<()> {
    let path = unit_path();
    if !path.exists() {
        println!("Service not installed");
        return Ok(());
    }

    let disable = std::process::Command::new("systemctl")
        .args(["--user", "disable", "--now", "blackbox.service"])
        .output()?;
    if !disable.status.success() {
        anyhow::bail!(
            "{}",
            format_command_error("systemctl disable", &disable.stderr)
        );
    }

    std::fs::remove_file(&path)?;
    println!("Service uninstalled (systemd)");
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn uninstall() -> anyhow::Result<()> {
    anyhow::bail!("Service uninstall not supported on this OS");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_plist_contains_expected_fragments() {
        let plist = generate_plist(
            "/usr/local/bin/blackbox",
            "/home/user/.local/share/blackbox",
        );
        assert!(plist.contains("<string>com.blackbox.agent</string>"));
        assert!(plist.contains("<string>/usr/local/bin/blackbox</string>"));
        assert!(plist.contains("<string>run-foreground</string>"));
        assert!(plist.contains("<true/>"));
        assert!(plist.contains("/home/user/.local/share/blackbox/blackbox.log"));
        assert!(plist.contains("/home/user/.local/share/blackbox/blackbox.err.log"));
        assert!(plist.contains("RunAtLoad"));
        assert!(plist.contains("KeepAlive"));
        assert!(plist.contains("ProgramArguments"));
    }

    #[test]
    fn test_generate_unit_file_contains_expected_fragments() {
        let unit = generate_unit_file("/usr/local/bin/blackbox");
        assert!(unit.contains("ExecStart=/usr/local/bin/blackbox run-foreground"));
        assert!(unit.contains("Type=simple"));
        assert!(unit.contains("Restart=on-failure"));
        assert!(unit.contains("WantedBy=default.target"));
        assert!(unit.contains("Description=Blackbox git activity recorder"));
    }

    #[test]
    fn test_plist_path_correct() {
        let path = plist_path();
        assert!(path.ends_with("Library/LaunchAgents/com.blackbox.agent.plist"));
    }

    #[test]
    fn test_unit_path_correct() {
        let path = unit_path();
        assert!(path.ends_with(".config/systemd/user/blackbox.service"));
    }

    #[test]
    fn format_command_error_includes_stderr() {
        let err = format_command_error("launchctl bootstrap", b"Permission denied\n");
        assert!(err.contains("launchctl bootstrap"));
        assert!(err.contains("Permission denied"));
    }

    #[test]
    fn format_command_error_empty_stderr() {
        let err = format_command_error("launchctl load", b"");
        assert!(err.contains("launchctl load"));
        assert!(err.contains("no stderr"));
    }
}
