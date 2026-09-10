use sha2::{Digest, Sha256};
use std::path::PathBuf;

use akspraypaint::{parse_theme, NoctaliaTheme};

/// The single source of truth for the active theme's colors, for every
/// source (custom, community, builtin, wallpaper) alike.
///
/// A user template (registered via [theme.templates.user.akspraypaint]
/// in Noctalia's config) renders ~/.config/noctalia/colors.json on every
/// theme change, regardless of source — this replaced an earlier
/// per-source lookup (reading custom/community palette files directly,
/// shelling out to `noctalia msg color-scheme-get`) that had no working
/// answer for "builtin" or "wallpaper" sources, since neither has a
/// static palette file of its own. Letting Noctalia's own template
/// engine resolve the active theme — the same mechanism it already uses
/// to theme every other app — means AKSprayPaint only has to read one
/// fixed file, for any source, with no special-casing.
///
/// This is also the same path v4 always used, so nothing here breaks
/// compatibility with a v4-only setup that never registers the template.
pub fn theme_config_path() -> Option<PathBuf> {
    let config = dirs::config_dir()?;
    let path = config.join("noctalia").join("colors.json");
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

/// Wait for colors.json to be freshly written (content differs from baseline).
/// This solves the Noctalia write-ordering problem: settings.toml may be updated
/// before colors.json, so a naive read can get a stale palette.  We wait up to
/// 2 seconds for the file content to actually change before proceeding.
pub fn ensure_fresh_colors_json(baseline: &str) -> Result<String, String> {
    let path = theme_config_path().ok_or("colors.json not found")?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let content =
            std::fs::read_to_string(&path).map_err(|e| format!("failed to read colors.json: {}", e))?;
        if content != baseline {
            return Ok(content);
        }
        if std::time::Instant::now() >= deadline {
            // No fresh content after 2s — Noctalia may not update colors.json for
            // this source; proceed with what we have (better than infinite wait)
            eprintln!("[theme] colors.json unchanged after 2s wait — proceeding with current content");
            return Ok(content);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// Read the theme from colors.json, waiting for it to be freshly written if
/// necessary so we never read a palette that predates the current theme name.
pub fn read_theme_fresh() -> Result<(NoctaliaTheme, String), String> {
    let path = theme_config_path().ok_or_else(|| {
        "colors.json not found at ~/.config/noctalia/colors.json — make sure the akspraypaint \
         user template is registered (see README: 'Setup') so Noctalia writes it on every theme \
         change."
            .to_string()
    })?;
    let baseline =
        std::fs::read_to_string(&path).map_err(|e| format!("failed to read theme: {}", e))?;
    let content = ensure_fresh_colors_json(&baseline)?;
    let theme = parse_theme(&content).ok_or_else(|| "failed to parse theme".to_string())?;
    Ok((theme, content))
}

pub fn read_theme() -> Result<(NoctaliaTheme, String), String> {
    let path = theme_config_path().ok_or_else(|| {
        "colors.json not found at ~/.config/noctalia/colors.json — make sure the akspraypaint \
         user template is registered (see README: 'Setup') so Noctalia writes it on every theme \
         change."
            .to_string()
    })?;
    let content =
        std::fs::read_to_string(&path).map_err(|e| format!("failed to read theme: {}", e))?;
    let theme = parse_theme(&content).ok_or_else(|| "failed to parse theme".to_string())?;
    Ok((theme, content))
}

pub fn theme_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hex::encode(&hasher.finalize()[..4])
}

pub fn noctalia_dir() -> Option<PathBuf> {
    let config = dirs::config_dir()?;
    let dir = config.join("noctalia");
    if dir.is_dir() {
        Some(dir)
    } else {
        None
    }
}

/// Find ~/.local/state/noctalia (where settings.toml lives).
pub fn noctalia_state_dir() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let dir = home.join(".local").join("state").join("noctalia");
    if dir.is_dir() {
        Some(dir)
    } else {
        None
    }
}
