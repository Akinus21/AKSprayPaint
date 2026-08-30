use sha2::{Digest, Sha256};
use std::path::PathBuf;

use akspraypaint::{parse_theme, NoctaliaTheme};

fn v4_theme_path() -> Option<PathBuf> {
    let config = dirs::config_dir()?;
    let path = config.join("noctalia").join("colors.json");
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

fn v5_active_palette_name() -> Option<String> {
    let state_dir = std::env::var("NOCTALIA_STATE_HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(|| {
            let state = dirs::data_local_dir()?;
            Some(state.join("noctalia"))
        })?;
    let settings_path = state_dir.join("settings.toml");
    if !settings_path.exists() {
        return None;
    }
    let content = std::fs::read_to_string(&settings_path).ok()?;
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("custom_palette") && line.contains('=') {
            if let Some(name) = line.split('=').nth(1) {
                let name = name.trim().trim_matches('"').trim_matches('\'');
                if !name.is_empty() {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
}

fn v5_theme_path() -> Option<PathBuf> {
    let palette_name = v5_active_palette_name()?;
    let config = dirs::config_dir()?;
    let path = config
        .join("noctalia")
        .join("palettes")
        .join(format!("{}.json", palette_name));
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

pub fn theme_config_path() -> Option<PathBuf> {
    v5_theme_path().or_else(v4_theme_path)
}

pub fn read_theme() -> Result<(NoctaliaTheme, String), String> {
    let path = theme_config_path().ok_or_else(|| {
        "noctalia colors.json not found at ~/.config/noctalia/colors.json".to_string()
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
