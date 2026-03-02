// macOS LaunchAgent management for auto-starting the daemon on login.

#[cfg(target_os = "macos")]
mod macos {
    use std::path::PathBuf;

    const LAUNCH_AGENT_LABEL: &str = "com.crossnotifier.daemon";

    fn launch_agent_path() -> Option<PathBuf> {
        let home = dirs::home_dir()?;
        Some(home.join("Library/LaunchAgents").join(format!("{}.plist", LAUNCH_AGENT_LABEL)))
    }

    pub fn is_autostart_installed() -> bool {
        launch_agent_path()
            .map(|p| p.exists())
            .unwrap_or(false)
    }

    pub fn install_autostart() -> anyhow::Result<()> {
        let plist_path = launch_agent_path()
            .ok_or_else(|| anyhow::anyhow!("Could not determine LaunchAgent path"))?;

        // Resolve executable path (follow symlinks)
        let exec_path = std::env::current_exe()?;
        let exec_path = std::fs::canonicalize(exec_path)?;
        let exec_str = exec_path.to_string_lossy();

        // Log directory
        let home = dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
        let log_path = home.join("Library/Logs");

        // Ensure LaunchAgents directory exists
        if let Some(dir) = plist_path.parent() {
            std::fs::create_dir_all(dir)?;
        }

        // Unload existing agent if present (ignore errors)
        let _ = std::process::Command::new("launchctl")
            .args(["unload", &plist_path.to_string_lossy()])
            .output();

        // Write plist
        let plist_content = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exec}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
    <key>ProcessType</key>
    <string>Interactive</string>
    <key>StandardOutPath</key>
    <string>{log}/cross-notifier.log</string>
    <key>StandardErrorPath</key>
    <string>{log}/cross-notifier.log</string>
</dict>
</plist>
"#,
            label = LAUNCH_AGENT_LABEL,
            exec = exec_str,
            log = log_path.to_string_lossy(),
        );

        std::fs::write(&plist_path, plist_content)?;

        // Load the agent
        let output = std::process::Command::new("launchctl")
            .args(["load", &plist_path.to_string_lossy()])
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("launchctl load failed: {}", stderr);
        }

        tracing::info!("Autostart installed: {}", plist_path.display());
        Ok(())
    }

    pub fn uninstall_autostart() -> anyhow::Result<()> {
        let plist_path = launch_agent_path()
            .ok_or_else(|| anyhow::anyhow!("Could not determine LaunchAgent path"))?;

        // Unload the agent (ignore errors if not loaded)
        let _ = std::process::Command::new("launchctl")
            .args(["unload", &plist_path.to_string_lossy()])
            .output();

        // Remove the plist file
        if plist_path.exists() {
            std::fs::remove_file(&plist_path)?;
        }

        tracing::info!("Autostart uninstalled");
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn test_launch_agent_path() {
            let path = launch_agent_path().unwrap();
            let path_str = path.to_string_lossy();
            assert!(path_str.contains("Library/LaunchAgents"));
            assert!(path_str.contains("com.crossnotifier.daemon.plist"));
        }
    }
}

#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(not(target_os = "macos"))]
pub fn is_autostart_installed() -> bool {
    false
}

#[cfg(not(target_os = "macos"))]
pub fn install_autostart() -> anyhow::Result<()> {
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn uninstall_autostart() -> anyhow::Result<()> {
    Ok(())
}
