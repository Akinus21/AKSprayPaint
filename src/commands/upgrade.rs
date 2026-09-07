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

    let stdout = String::from_utf8_lossy(&upgrade.stdout);
    let stderr = String::from_utf8_lossy(&upgrade.stderr);

    if !upgrade.status.success() {
        return Err(format!("brew upgrade failed:\n{}{}", stdout, stderr));
    }

    if stdout.contains("Upgraded 1") {
        println!("Upgraded akspraypaint successfully.");
    } else if stdout.contains("Already up-to-date") {
        // silent
    } else if !stdout.is_empty() {
        println!("{}", stdout.trim());
    }

    Ok(())
}
