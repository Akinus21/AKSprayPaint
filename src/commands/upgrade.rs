use std::process::Command;

/// Upgrade akspraypaint via Homebrew.
pub fn upgrade() -> Result<(), String> {
    println!("Checking for updates...");

    let update = Command::new("brew")
        .args(["update"])
        .output()
        .map_err(|e| format!("failed to run brew update: {}", e))?;

    if !update.status.success() {
        let stderr = String::from_utf8_lossy(&update.stderr);
        return Err(format!("brew update failed: {}", stderr));
    }

    if !update.stdout.is_empty() {
        println!("{}", String::from_utf8_lossy(&update.stdout).trim());
    }

    println!("Upgrading akspraypaint...");

    let upgrade = Command::new("brew")
        .args(["upgrade", "akspraypaint"])
        .output()
        .map_err(|e| format!("failed to run brew upgrade: {}", e))?;

    if !upgrade.status.success() {
        let stderr = String::from_utf8_lossy(&upgrade.stderr);
        return Err(format!("brew upgrade failed: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&upgrade.stdout);
    if !stdout.trim().is_empty() {
        println!("{}", stdout.trim());
    }

    Ok(())
}
