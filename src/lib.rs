use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct NoctaliaTheme {
    pub primary: [u8; 3],
    pub on_primary: [u8; 3],
    pub surface: [u8; 3],
    pub on_surface: [u8; 3],
    pub surface_variant: [u8; 3],
    pub on_surface_variant: [u8; 3],
    pub error: [u8; 3],
}

impl NoctaliaTheme {
    pub fn palette(&self) -> Vec<[u8; 3]> {
        vec![
            self.primary,
            self.on_primary,
            self.surface,
            self.on_surface,
            self.surface_variant,
            self.on_surface_variant,
            self.error,
        ]
    }
}

#[derive(Debug, Deserialize)]
struct RawColors {
    #[serde(rename = "mPrimary")]
    primary: Option<String>,
    #[serde(rename = "mOnPrimary")]
    on_primary: Option<String>,
    #[serde(rename = "mSurface")]
    surface: Option<String>,
    #[serde(rename = "mOnSurface")]
    on_surface: Option<String>,
    #[serde(rename = "mSurfaceVariant")]
    surface_variant: Option<String>,
    #[serde(rename = "mOnSurfaceVariant")]
    on_surface_variant: Option<String>,
    #[serde(rename = "mError")]
    error: Option<String>,
}

fn parse_hex_color(hex: &str) -> Option<[u8; 3]> {
    let hex = hex.strip_prefix('#')?;
    let (r, g, b) = match hex.len() {
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            (r, g, b)
        }
        8 => {
            let _a = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let r = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let g = u8::from_str_radix(&hex[4..6], 16).ok()?;
            let b = u8::from_str_radix(&hex[6..8], 16).ok()?;
            (r, g, b)
        }
        _ => return None,
    };
    Some([r, g, b])
}

pub fn parse_theme(content: &str) -> Option<NoctaliaTheme> {
    let raw: RawColors = serde_json::from_str(content).ok()?;
    Some(NoctaliaTheme {
        primary: parse_hex_color(raw.primary.as_deref()?)?,
        on_primary: parse_hex_color(raw.on_primary.as_deref()?)?,
        surface: parse_hex_color(raw.surface.as_deref()?)?,
        on_surface: parse_hex_color(raw.on_surface.as_deref()?)?,
        surface_variant: parse_hex_color(raw.surface_variant.as_deref()?)?,
        on_surface_variant: parse_hex_color(raw.on_surface_variant.as_deref()?)?,
        error: parse_hex_color(raw.error.as_deref()?)?,
    })
}
