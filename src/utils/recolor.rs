use image::{Rgb, RgbImage};
use palette::{FromColor, IntoColor, Oklch, OklabHue, Srgb};

use akspraypaint::NoctaliaTheme;

const SHARPNESS: f32 = 8.0;
const EPSILON: f32 = 1e-6;

/// Recolor `input` so its colors match the noctalia `theme`.
///
/// Algorithm:
/// 1. Run `matugen image <wallpaper> --json hex` to extract a theme palette from the wallpaper
/// 2. Read the target theme from `~/.config/noctalia/colors.json`
/// 3. Build anchor pairs by slot name (source.slot → target.slot for all 7 slots)
/// 4. Apply anchor-based inverse-distance transfer for each pixel
pub fn recolor_wallpaper(input: &RgbImage, theme: &NoctaliaTheme) -> RgbImage {
    let mappings = match extract_wallpaper_theme(input) {
        Ok(source) => {
            eprintln!("Using matugen extraction for color mapping");
            build_anchor_mappings(&source, theme)
        }
        Err(e) => {
            eprintln!("Matugen failed ({}), using fallback", e);
            fallback_mappings(theme)
        }
    };

    let (width, height) = input.dimensions();
    let mut output = RgbImage::new(width, height);

    for (x, y, pixel) in input.enumerate_pixels() {
        let orig = rgb_to_oklch(pixel);
        output.put_pixel(x, y, transfer_pixel(orig, &mappings));
    }

    output
}

/// Call `matugen image <path> --json hex` to extract theme from the wallpaper.
fn extract_wallpaper_theme(input: &RgbImage) -> Result<MatugenTheme, String> {
    let (w, h) = input.dimensions();

    let mut buf = std::io::Cursor::new(Vec::new());
    input.write_to(&mut buf, image::ImageFormat::Png)
        .map_err(|e| format!("failed to encode image: {}", e))?;

    let tmp_path = std::env::temp_dir().join("akspraypaint_wallpaper.png");
    std::fs::write(&tmp_path, buf.get_ref())
        .map_err(|e| format!("failed to write temp image: {}", e))?;

    let output = std::process::Command::new("matugen")
        .args(["image", &tmp_path.to_string_lossy(), "--json", "hex"])
        .output()
        .map_err(|e| format!("matugen failed to start: {}", e))?;

    std::fs::remove_file(&tmp_path).ok();

    if !output.status.success() {
        return Err(format!(
            "matugen exited {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    parse_matugen_json(&stdout)
}

#[derive(Debug)]
struct MatugenTheme {
    pub primary: [u8; 3],
    pub on_primary: [u8; 3],
    pub surface: [u8; 3],
    pub on_surface: [u8; 3],
    pub surface_variant: [u8; 3],
    pub on_surface_variant: [u8; 3],
    pub error: [u8; 3],
}

fn parse_matugen_json(json: &str) -> Result<MatugenTheme, String> {
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| format!("failed to parse matugen JSON: {} — raw: {}", e, &json[..json.len().min(500)]))?;

    fn get_hex(obj: &serde_json::Map<String, serde_json::Value>, key: &str) -> Result<[u8; 3], String> {
        let hex = obj.get(key)
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("missing key: {}", key))?;
        parse_hex(hex)
    }

    let obj = value.as_object()
        .ok_or_else(|| "matugen output is not an object".to_string())?;

    Ok(MatugenTheme {
        primary: get_hex(obj, "primary")?,
        on_primary: get_hex(obj, "on_primary")?,
        surface: get_hex(obj, "surface")?,
        on_surface: get_hex(obj, "on_surface")?,
        surface_variant: get_hex(obj, "surface_variant")?,
        on_surface_variant: get_hex(obj, "on_surface_variant")?,
        error: get_hex(obj, "error").unwrap_or([200, 50, 50]),
    })
}

fn parse_hex(hex: &str) -> Result<[u8; 3], String> {
    let hex = hex.trim().trim_start_matches('#');
    if hex.len() != 6 {
        return Err(format!("invalid hex length: {}", hex));
    }
    let r = u8::from_str_radix(&hex[0..2], 16)
        .map_err(|_| format!("invalid hex: {}", hex))?;
    let g = u8::from_str_radix(&hex[2..4], 16)
        .map_err(|_| format!("invalid hex: {}", hex))?;
    let b = u8::from_str_radix(&hex[4..6], 16)
        .map_err(|_| format!("invalid hex: {}", hex))?;
    Ok([r, g, b])
}

fn build_anchor_mappings(source: &MatugenTheme, target: &NoctaliaTheme) -> Vec<(Oklch<f32>, Oklch<f32>)> {
    vec![
        (rgb_to_oklch(&Rgb(source.primary)), rgb_to_oklch(&Rgb(target.primary))),
        (rgb_to_oklch(&Rgb(source.on_primary)), rgb_to_oklch(&Rgb(target.on_primary))),
        (rgb_to_oklch(&Rgb(source.surface)), rgb_to_oklch(&Rgb(target.surface))),
        (rgb_to_oklch(&Rgb(source.on_surface)), rgb_to_oklch(&Rgb(target.on_surface))),
        (rgb_to_oklch(&Rgb(source.surface_variant)), rgb_to_oklch(&Rgb(target.surface_variant))),
        (rgb_to_oklch(&Rgb(source.on_surface_variant)), rgb_to_oklch(&Rgb(target.on_surface_variant))),
        (rgb_to_oklch(&Rgb(source.error)), rgb_to_oklch(&Rgb(target.error))),
    ]
}

fn fallback_mappings(theme: &NoctaliaTheme) -> Vec<(Oklch<f32>, Oklch<f32>)> {
    let palette = theme.palette();
    palette.iter().map(|c| {
        let src = rgb_to_oklch(&Rgb(*c));
        let tgt = src;
        (src, tgt)
    }).collect()
}

fn transfer_pixel(orig: Oklch<f32>, mappings: &[(Oklch<f32>, Oklch<f32>)]) -> Rgb<u8> {
    let mut total_w = 0.0f32;
    let mut out_l = 0.0f32;
    let mut out_a = 0.0f32;
    let mut out_b = 0.0f32;

    for (src, tgt) in mappings {
        let dist = oklch_dist(&orig, src).max(EPSILON);
        let w = 1.0 / dist.powf(SHARPNESS);

        let l_ratio = if src.l > 0.01 {
            (orig.l / src.l).clamp(0.5, 2.0)
        } else {
            1.0
        };
        let mapped_l = (tgt.l * l_ratio).clamp(0.0, 1.0);

        let (ta, tb) = hue_to_ab(tgt.chroma, tgt.hue);
        out_l += w * mapped_l;
        out_a += w * ta;
        out_b += w * tb;
        total_w += w;
    }

    let final_l = (out_l / total_w).clamp(0.0, 1.0);
    let a = out_a / total_w;
    let b = out_b / total_w;

    oklch_to_rgb(&Oklch {
        l: final_l,
        chroma: (a * a + b * b).sqrt().clamp(0.0, 0.5),
        hue: OklabHue::from_degrees(b.atan2(a).to_degrees()),
    })
}

fn hue_to_ab(chroma: f32, hue: OklabHue<f32>) -> (f32, f32) {
    let r = hue.into_radians();
    (chroma * r.cos(), chroma * r.sin())
}

fn oklch_dist(a: &Oklch<f32>, b: &Oklch<f32>) -> f32 {
    let (aa, ab) = hue_to_ab(a.chroma, a.hue);
    let (ba, bb) = hue_to_ab(b.chroma, b.hue);
    let dl = a.l - b.l;
    ((dl * dl) + (aa - ba).powi(2) + (ab - bb).powi(2)).sqrt()
}

fn rgb_to_oklch(p: &Rgb<u8>) -> Oklch<f32> {
    let s = Srgb::new(p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0);
    Oklch::from_color(s.into_linear())
}

fn oklch_to_rgb(c: &Oklch<f32>) -> Rgb<u8> {
    let linear: palette::LinSrgb<f32> = (*c).into_color();
    let s: Srgb<f32> = linear.into_encoding();
    Rgb([
        (s.red * 255.0).round().clamp(0.0, 255.0) as u8,
        (s.green * 255.0).round().clamp(0.0, 255.0) as u8,
        (s.blue * 255.0).round().clamp(0.0, 255.0) as u8,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_theme() -> NoctaliaTheme {
        NoctaliaTheme {
            primary: [100, 50, 200],
            on_primary: [240, 230, 255],
            surface: [20, 15, 35],
            on_surface: [180, 170, 200],
            surface_variant: [45, 35, 65],
            on_surface_variant: [160, 150, 185],
            error: [200, 50, 50],
        }
    }

    #[test]
    fn test_parse_hex() {
        let c = parse_hex("ff0000").unwrap();
        assert_eq!(c, [255, 0, 0]);
        let c = parse_hex("#00ff00").unwrap();
        assert_eq!(c, [0, 255, 0]);
    }

    #[test]
    fn test_rgb_oklch_roundtrip() {
        let orig = Rgb([100, 150, 200]);
        let oklch = rgb_to_oklch(&orig);
        let back = oklch_to_rgb(&oklch);
        let diff = (back[0] as i32 - orig[0] as i32).abs()
            + (back[1] as i32 - orig[1] as i32).abs()
            + (back[2] as i32 - orig[2] as i32).abs();
        assert!(diff < 10, "roundtrip should be close, diff={}", diff);
    }
}
