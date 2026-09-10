//! GTK bridge daemon configuration.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const CONFIG_FILE_NAME: &str = "gtk-bridge.toml";

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct SettingsToggle {
    pub icons: bool,
    pub wallpaper: bool,
    pub gtk_theme: bool,
    pub cursor_theme: bool,
    pub font: bool,
    pub color_scheme: bool,
    pub flatpak: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct WatchConfig {
    pub apps: Vec<String>,
    /// Restart each watched app after injecting settings so it picks up the new theme.
    pub restart_after_inject: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PollConfig {
    pub interval_ms: u64,
    pub launch_grace_ms: u64,
}

impl Default for PollConfig {
    fn default() -> Self {
        Self {
            interval_ms: 1000,
            launch_grace_ms: 2500,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct GtkBridgeConfig {
    pub settings: SettingsToggle,
    pub watch: WatchConfig,
    pub poll: PollConfig,
}

impl GtkBridgeConfig {
    /// Load config from file, creating with defaults if missing.
    pub fn load_or_create(path: &PathBuf) -> Result<Self, String> {
        if path.exists() {
            let content = std::fs::read_to_string(path)
                .map_err(|e| format!("failed to read config: {}", e))?;
            toml::from_str(&content).map_err(|e| format!("failed to parse config: {}", e))
        } else {
            let config = GtkBridgeConfig::default();
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("failed to create config dir: {}", e))?;
            }
            let content = toml::to_string_pretty(&config)
                .map_err(|e| format!("failed to serialize config: {}", e))?;
            std::fs::write(path, content).map_err(|e| format!("failed to write config: {}", e))?;
            Ok(config)
        }
    }
}

/// Return the default config file path: ~/.config/akspraypaint/gtk-bridge.toml
pub fn default_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("akspraypaint")
        .join(CONFIG_FILE_NAME)
}
