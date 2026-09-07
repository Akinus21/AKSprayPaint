use std::process::Command;

/// Upgrade akspraypaint via Homebrew.
pub fn upgrade() {
    println!("Checking for updates...");

    let update = match Command::new("brew").args(["update"]).output() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("failed to run brew update: {}", e);
            return;
        }
    };

    if !update.status.success() {
        let stderr = String::from_utf8_lossy(&update.stderr);
        eprintln!("brew update failed: {}", stderr);
        return;
    }

    if !update.stdout.is_empty() {
        println!("{}", String::from_utf8_lossy(&update.stdout).trim());
    }

    println!("Upgrading akspraypaint...");

    let upgrade = match Command::new("brew").args(["upgrade", "akspraypaint"]).output() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("failed to run brew upgrade: {}", e);
            return;
        }
    };

    let stdout = String::from_utf8_lossy(&upgrade.stdout);
    let stderr = String::from_utf8_lossy(&upgrade.stderr);

    if !upgrade.status.success() {
        eprintln!("brew upgrade failed:\n{}{}", stdout, stderr);
        return;
    }

    if stdout.contains("Upgraded 1") {
        println!("Upgraded akspraypaint successfully.");
    } else if stdout.contains("Already up-to-date") {
        // silent
    } else if !stdout.is_empty() {
        println!("{}", stdout.trim());
    }
}
