//! Niri window border recoloring.
//!
//! Writes border colors to ~/.config/niri/noctalia.kdl from the active
//! Noctalia palette, then signals niri to reload via:
//!   niri msg action load-config-file
//!
//! This is safe — it only edits a config file and sends a socket msg,
//! does NOT kill or restart niri.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// Path to the niri border colors include file.
pub fn border_config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config")
        .join("niri")
        .join("noctalia.kdl")
}

/// Write border colors from a NoctaliaTheme to the niri include file.
/// The file is included by niri's config.kdl via `include "noctalia.kdl"`.
pub fn write_border_config(theme: &crate::gtkd::theme::NoctaliaTheme) -> Result<(), String> {
    // Border color = on_surface_variant (the readable text color on dark bg)
    let [r1, g1, b1] = theme.on_surface_variant;
    // Inactive border = surface_variant (dimmer)
    let [r2, g2, b2] = theme.surface_variant;

    let kdl = format!(
        r#"window {{
    border: {r1} {g1} {b1};
    border-radius: 8;
    titlebar: {r1} {g1} {b1};
    inactive-border: {r2} {g2} {b2};
    inactive-titlebar: {r2} {g2} {b2};
}}
"#,
    );

    let path = border_config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create niri config dir: {}", e))?;
    }
    fs::write(&path, kdl).map_err(|e| format!("failed to write {}: {}", path.display(), e))?;

    eprintln!("[gtkd] wrote niri border config to {}", path.display());
    Ok(())
}

/// Signal niri to reload its config via socket IPC.
pub fn reload_niri_config() {
    let output = Command::new("niri")
        .args(["msg", "action", "load-config-file"])
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
