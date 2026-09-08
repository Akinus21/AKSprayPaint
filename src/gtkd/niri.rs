//! Niri window border recoloring.
//!
//! Edits ~/.config/niri/config.kdl in-place to update border colors.
//! Creates a backup at ~/.config/niri/config.kdl.bak before editing.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// Path to the niri config file.
fn config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config")
        .join("niri")
        .join("config.kdl")
}

/// Update border colors in the niri config file.
/// Reads config.kdl, replaces active-color and inactive-color in the border block,
/// writes back. Creates a .bak backup first.
pub fn update_border_colors(
    active_color: [u8; 3],
    inactive_color: [u8; 3],
) -> Result<(), String> {
    let path = config_path();

    // Read original
    let content = fs::read_to_string(&path)
        .map_err(|e| format!("failed to read {}: {}", path.display(), e))?;

    // Create backup
    let bak_path = format!("{}.bak", path.display());
    fs::write(&bak_path, &content)
        .map_err(|e| format!("failed to write backup: {}", e))?;

    // Format new colors
    let active = format!("{:02x}{:02x}{:02x}", active_color[0], active_color[1], active_color[2]);
    let inactive = format!("{:02x}{:02x}{:02x}", inactive_color[0], inactive_color[1], inactive_color[2]);

    // Replace active-color and inactive-color in the border block
    // Handle both "hex" and hex formats
    let updated = replace_color_in_border(&content, &active, &inactive);

    // Write back
    fs::write(&path, updated)
        .map_err(|e| format!("failed to write {}: {}", path.display(), e))?;

    eprintln!("[gtkd] updated niri border colors (backup: {}.bak)", path.display());
    Ok(())
}

/// Replace active-color and inactive-color values in the border block.
fn replace_color_in_border(content: &str, active: &str, inactive: &str) -> String {
    // Match active-color lines: active-color "#rrggbb" or active-color rrggbb
    // and inactive-color lines similarly.
    // We replace the color value (the hex string) while keeping the format.
    let mut result = content.to_string();

    // Replace active-color values
    // Pattern: active-color "xxxxxxxx" or active-color xxxxxxxx
    let active_re = regex::Regex::new(r#"(active-color\s+")([^"]+)(")"#).unwrap();
    result = active_re
        .replace(&result, format!(r#"$1#{active}$3"#))
        .to_string();

    // Also handle bare hex (no quotes) - try to match 6 hex chars after active-color
    let active_re2 = regex::Regex::new(r"(active-color\s+)([0-9a-fA-F]{6})").unwrap();
    result = active_re2
        .replace(&result, format!("$1#{active}"))
        .to_string();

    // Replace inactive-color values
    let inactive_re = regex::Regex::new(r#"(inactive-color\s+")([^"]+)(")"#).unwrap();
    result = inactive_re
        .replace(&result, format!(r#"$1#{inactive}$3"#))
        .to_string();

    let inactive_re2 = regex::Regex::new(r"(inactive-color\s+)([0-9a-fA-F]{6})").unwrap();
    result = inactive_re2
        .replace(&result, format!("$1#{inactive}"))
        .to_string();

    result
}

/// Signal niri to reload its config via socket IPC.
pub fn reload_niri_config() {
    let output = Command::new("sh")
        .args(["-c", "timeout 3 niri msg action load-config-file || true"])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            eprintln!("[gtkd] niri config reloaded");
        }
        Ok(out) => {
            eprintln!(
                "[gtkd] niri msg failed ({}): {}",
                out.status,
                String::from_utf8_lossy(&out.stderr)
            );
        }
        Err(e) => {
            eprintln!("[gtkd] niri msg error: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_replace_color_quoted() {
        let input = r##"layout {
    border {
        active-color "#ff0000"
        inactive-color "#00ff00"
    }
}"##;
        let result = replace_color_in_border(input, "aabbcc", "112233");
        assert!(result.contains(r##"active-color "#aabbcc""##));
        assert!(result.contains(r##"inactive-color "#112233""##));
    }

    #[test]
    fn test_replace_color_bare() {
        let input = r##"border {
    active-color aabbcc
    inactive-color 112233
}"##;
        let result = replace_color_in_border(input, "aabbcc", "112233");
        assert!(result.contains("#aabbcc"));
        assert!(result.contains("#112233"));
    }
}
