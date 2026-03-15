use std::path::PathBuf;

use crate::config;
use crate::daemon;

const LABEL: &str = "com.blackbox.agent";

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
    <key>StandardOutPath</key>
    <string>{data}/blackbox.log</string>
    <key>StandardErrorPath</key>
    <string>{data}/blackbox.err.log</string>
</dict>
</plist>"#,
        label = LABEL,
        exe = exe_path,
        data = data_dir,
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
    // Stop any manually-started daemon first
    if daemon::is_daemon_running()?.is_some() {
        daemon::stop_daemon()?;
    }

    let exe = std::env::current_exe()?
        .canonicalize()?
        .to_string_lossy()
        .to_string();
    let data_dir = config::data_dir()?;
    std::fs::create_dir_all(&data_dir)?;

    let plist_content = generate_plist(&exe, &data_dir.to_string_lossy());
    let path = plist_path();

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, &plist_content)?;

    let uid = nix::unistd::getuid().as_raw();
    let path_str = path.to_string_lossy().to_string();

    // Try modern bootstrap, fallback to legacy load
    let status = std::process::Command::new("launchctl")
        .args(["bootstrap", &format!("gui/{}", uid), &path_str])
        .status()?;

    if !status.success() {
        std::process::Command::new("launchctl")
            .args(["load", &path_str])
            .status()?;
    }

    println!("Service installed and loaded (launchd)");
    println!("Plist: {}", path_str);
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn install() -> anyhow::Result<()> {
    if daemon::is_daemon_running()?.is_some() {
        daemon::stop_daemon()?;
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

    std::process::Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status()?;

    std::process::Command::new("systemctl")
        .args(["--user", "enable", "--now", "blackbox.service"])
        .status()?;

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
    let status = std::process::Command::new("launchctl")
        .args(["bootout", &format!("gui/{}", uid), &path_str])
        .status()?;

    if !status.success() {
        std::process::Command::new("launchctl")
            .args(["unload", &path_str])
            .status()?;
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

    std::process::Command::new("systemctl")
        .args(["--user", "disable", "--now", "blackbox.service"])
        .status()?;

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
        let plist = generate_plist("/usr/local/bin/blackbox", "/home/user/.local/share/blackbox");
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
}
