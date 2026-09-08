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
    /// Full Noctalia theme colors, if available.
    pub noctalia_theme: Option<NoctaliaTheme>,
}

/// Minimal Noctalia theme — only the fields needed for niri border recoloring.
#[derive(Debug, Clone)]
pub struct NoctaliaTheme {
    /// mOnSurfaceVariant — used for active window border
    pub on_surface_variant: [u8; 3],
    /// mSurfaceVariant — used for inactive window border
    pub surface_variant: [u8; 3],
}

/// JSON shape of the relevant fields in colors.json.
#[derive(serde::Deserialize)]
struct RawColorsFlat {
    #[serde(rename = "mOnSurfaceVariant")]
    on_surface_variant: Option<String>,
    #[serde(rename = "mSurfaceVariant")]
    surface_variant: Option<String>,
}

#[derive(serde::Deserialize)]
struct RawColorsV5 {
    dark: Option<RawColorsFlat>,
    light: Option<RawColorsFlat>,
}

impl ThemeState {
    /// Detect the current theme from Noctalia's config files.
    pub fn detect() -> Self {
        let icon_theme = crate::utils::icons::get_current_theme_name().ok();
        let is_dark = Self::detect_dark_mode();
        let noctalia_theme = Self::load_noctalia_theme();

        Self {
            icon_theme,
            is_dark,
            noctalia_theme,
        }
    }

    fn detect_dark_mode() -> bool {
        let gtk_theme = std::env::var("GTK_THEME").ok();
        gtk_theme
            .as_ref()
            .map(|t| t.to_lowercase().contains("dark"))
            .unwrap_or(true)
    }

    fn load_noctalia_theme() -> Option<NoctaliaTheme> {
        let colors_path = crate::utils::theme::theme_config_path()?;
        let content = std::fs::read_to_string(colors_path).ok()?;
        Self::parse_theme(&content).ok()
    }

    fn parse_theme(content: &str) -> Result<NoctaliaTheme, String> {
        // Try v5 format first
        if let Ok(v5) = serde_json::from_str::<RawColorsV5>(content) {
            let raw = v5.dark.as_ref().or(v5.light.as_ref());
            if let Some(raw) = raw {
                return Self::colors_from_raw(raw);
            }
        }

        // Try flat format
        if let Ok(flat) = serde_json::from_str::<RawColorsFlat>(content) {
            return Self::colors_from_raw(&flat);
        }

        Err("failed to parse theme".to_string())
    }

    fn parse_hex(hex: &str) -> Option<[u8; 3]> {
        let hex = hex.strip_prefix('#')?;
        let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
        let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
        let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
        Some([r, g, b])
    }

    fn colors_from_raw(raw: &RawColorsFlat) -> Result<NoctaliaTheme, String> {
        let on_sv = raw
            .on_surface_variant
            .as_ref()
            .and_then(|v| Self::parse_hex(v))
            .ok_or_else(|| "missing mOnSurfaceVariant".to_string())?;
        let sv = raw
            .surface_variant
            .as_ref()
            .and_then(|v| Self::parse_hex(v))
            .ok_or_else(|| "missing mSurfaceVariant".to_string())?;
        Ok(NoctaliaTheme {
            on_surface_variant: on_sv,
            surface_variant: sv,
        })
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
