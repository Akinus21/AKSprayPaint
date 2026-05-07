use crate::NoctaliaTheme;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

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
        "noctalia colors.json not found at ~/.config/noctalia/colors.json".to_string()
    })?;
    let content =
        std::fs::read_to_string(&path).map_err(|e| format!("failed to read colors.json: {}", e))?;
    let theme = crate::parse_theme(&content)
        .ok_or_else(|| "failed to parse colors.json".to_string())?;
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
