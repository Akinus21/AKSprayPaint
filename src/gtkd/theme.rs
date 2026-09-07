//! Theme detection — thin wrapper around AKSprayPaint's existing theme module.
//!
//! Reuses the existing Noctalia theme detection so the daemon always agrees
//! with the CLI on what the current theme is.


/// Current theme state as detected from Noctalia's config.
#[derive(Debug, Clone, Default)]
pub struct ThemeState {
    /// Current icon theme name, e.g. "Purple_Haze"
    pub icon_theme: Option<String>,
    /// Whether the current theme is dark (for gtk-application-prefer-dark-theme)
    pub is_dark: bool,
}

impl ThemeState {
    /// Detect the current theme from Noctalia's config files.
    pub fn detect() -> Self {
        // Use the existing get_current_theme_name from icons module
        let icon_theme = crate::utils::icons::get_current_theme_name().ok();

        // Check if dark mode from the GTK settings (approximate)
        let is_dark = Self::detect_dark_mode();

        Self {
            icon_theme,
            is_dark,
        }
    }

    fn detect_dark_mode() -> bool {
        // Check if Adwaita-dark is the GTK theme, or check the GTK color-scheme
        // For now, assume dark if using adw-gtk3-dark
        let gtk_theme = std::env::var("GTK_THEME").ok();
        gtk_theme
            .as_ref()
            .map(|t| t.to_lowercase().contains("dark"))
            .unwrap_or(true) // default to dark
    }

    /// Return the icon theme name, or a fallback.
    pub fn icon_theme_name(&self) -> String {
        self.icon_theme.clone().unwrap_or_else(|| "Adwaita".to_string())
    }
}

/// Check if the theme has changed since the last detection.
pub fn has_changed(current: &ThemeState, previous: &ThemeState) -> bool {
    current.icon_theme != previous.icon_theme || current.is_dark != previous.is_dark
}
