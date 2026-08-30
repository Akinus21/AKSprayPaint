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

pub fn read_theme() -> Result<(NoctaliaTheme, String), String> {
    let path = theme_config_path().ok_or_else(|| {
        "colors.json not found at ~/.config/noctalia/colors.json — make sure the akspraypaint \
         user template is registered (see README: 'Setup') so Noctalia writes it on every theme \
         change."
            .to_string()
    })?;
    let content =
        std::fs::read_to_string(&path).map_err(|e| format!("failed to read theme: {}", e))?;
    let theme = parse_theme(&content)
        .ok_or_else(|| "failed to parse theme".to_string())?;
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
