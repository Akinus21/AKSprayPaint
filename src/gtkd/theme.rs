//! Theme detection — thin wrapper around AKSprayPaint's existing theme module.
//!
//! Reuses the existing Noctalia theme detection so the daemon always agrees
//! with the CLI on what the current theme is.

/// Current theme state as detected from Noctalia's config.
#[derive(Debug, Clone)]
pub struct ThemeState {
    /// Current icon theme name, e.g. "Purple_Haze"
    pub icon_theme: Option<String>,
}

impl ThemeState {
    /// Detect the current theme from Noctalia's config files.
    pub fn detect() -> Self {
        let icon_theme = crate::utils::icons::get_current_theme_name().ok();
        Self { icon_theme }
    }

    /// Return the icon theme name, or a fallback.
    pub fn icon_theme_name(&self) -> String {
        self.icon_theme
            .clone()
            .unwrap_or_else(|| "Adwaita".to_string())
    }
}

/// Check if the theme has changed since the last detection.
pub fn has_changed(current: &ThemeState, previous: &ThemeState) -> bool {
    current.icon_theme != previous.icon_theme
}
