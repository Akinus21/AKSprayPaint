use image::{Rgb, RgbImage};
use palette::{FromColor, IntoColor, Oklch, OklabHue, Srgb};
use quantette::{PaletteSize, Pipeline, QuantizeMethod};

use akspraypaint::NoctaliaTheme;

const SHARPNESS: f32 = 8.0;
const EPSILON: f32 = 1e-6;

pub fn recolor_wallpaper(input: &RgbImage, theme: &NoctaliaTheme, verbose: bool) -> RgbImage {
    let mappings = match extract_wallpaper_theme(input, theme) {
        Ok(source) => {
            if verbose {
                eprintln!("Using matugen extraction for color mapping");
            }
            build_anchor_mappings(&source, theme, verbose)
        }
        Err(e) => {
            eprintln!("Matugen failed ({}), using quantette k-means fallback", e);
            fallback_mappings(input, theme)
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

fn extract_wallpaper_theme(input: &RgbImage, target: &NoctaliaTheme) -> Result<MatugenTheme, String> {
    let mut buf = std::io::Cursor::new(Vec::new());
    input.write_to(&mut buf, image::ImageFormat::Png)
        .map_err(|e| format!("failed to encode image: {}", e))?;

    let tmp_path = std::env::temp_dir().join("akspraypaint_wallpaper.png");
    std::fs::write(&tmp_path, buf.get_ref())
        .map_err(|e| format!("failed to write temp image: {}", e))?;

    let mut extracted_colors: Vec<Vec<[u8; 3]>> = Vec::new();
    for idx in 0..7 {
        let output = std::process::Command::new("matugen")
            .args(["image", &tmp_path.to_string_lossy(), "--json", "hex", "--source-color-index", &idx.to_string()])
            .output()
            .map_err(|e| format!("matugen failed to start: {}", e))?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            if let Ok(colors) = extract_colors_from_json(&stdout) {
                extracted_colors.push(colors);
            }
        }
    }

    std::fs::remove_file(&tmp_path).ok();

    if extracted_colors.is_empty() {
        return Err("matugen failed to extract any colors".to_string());
    }

    let target_colors = vec![
        rgb_to_oklch(&Rgb(target.primary)),
        rgb_to_oklch(&Rgb(target.on_primary)),
        rgb_to_oklch(&Rgb(target.surface)),
        rgb_to_oklch(&Rgb(target.on_surface)),
        rgb_to_oklch(&Rgb(target.surface_variant)),
        rgb_to_oklch(&Rgb(target.on_surface_variant)),
    ];
    let target_error = rgb_to_oklch(&Rgb(target.error));

    let all_extracted: Vec<Oklch<f32>> = extracted_colors.iter()
        .flat_map(|colors| colors.iter().map(|c| rgb_to_oklch(&Rgb(*c))))
        .collect();

    if all_extracted.is_empty() {
        return Err("no colors extracted".to_string());
    }

    let source_primary = find_closest_by_hue_and_lightness(&all_extracted, &target_colors[0]);
    let source_on_primary = find_closest_by_hue_and_lightness(&all_extracted, &target_colors[1]);
    let source_surface = find_closest_by_hue_and_lightness(&all_extracted, &target_colors[2]);
    let source_on_surface = find_closest_by_hue_and_lightness(&all_extracted, &target_colors[3]);
    let source_surface_variant = find_closest_by_hue_and_lightness(&all_extracted, &target_colors[4]);
    let source_on_surface_variant = find_closest_by_hue_and_lightness(&all_extracted, &target_colors[5]);
    let source_error = {
        let srgb: Srgb<f32> = Srgb::from_color(target_error);
        [
            (srgb.red * 255.0).round() as u8,
            (srgb.green * 255.0).round() as u8,
            (srgb.blue * 255.0).round() as u8,
        ]
    };

    Ok(MatugenTheme {
        primary: source_primary,
        on_primary: source_on_primary,
        surface: source_surface,
        on_surface: source_on_surface,
        surface_variant: source_surface_variant,
        on_surface_variant: source_on_surface_variant,
        error: source_error,
    })
}

fn find_closest_by_hue_and_lightness(colors: &[Oklch<f32>], target: &Oklch<f32>) -> [u8; 3] {
    let target_l = target.l;
    let target_h = target.hue;
    let closest = colors.iter()
        .min_by(|a, b| {
            let da_lightness = (a.l - target_l).abs();
            let db_lightness = (b.l - target_l).abs();
            let da_hue = hue_dist(target_h, a.hue) / 180.0;
            let db_hue = hue_dist(target_h, b.hue) / 180.0;
            let score_a = da_lightness * 0.5 + da_hue * 0.5;
            let score_b = db_lightness * 0.5 + db_hue * 0.5;
            score_a.partial_cmp(&score_b).unwrap()
        })
        .unwrap_or(target);
    let srgb: Srgb<f32> = Srgb::from_color(*closest);
    [
        (srgb.red * 255.0).round() as u8,
        (srgb.green * 255.0).round() as u8,
        (srgb.blue * 255.0).round() as u8,
    ]
}

fn extract_colors_from_json(json: &str) -> Result<Vec<[u8; 3]>, String> {
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| format!("failed to parse matugen JSON: {}", e))?;
    let obj = value.as_object().ok_or("not an object")?;
    let colors = obj.get("colors").ok_or("missing colors")?.as_object().ok_or("colors not object")?;

    let mut result = Vec::new();
    for key in &["primary", "on_primary", "surface", "on_surface", "surface_variant", "on_surface_variant", "error"] {
        if let Ok(rgb) = get_hex_from_scheme(colors, key) {
            result.push(rgb);
        }
    }
    Ok(result)
}

fn get_hex_from_scheme(obj: &serde_json::Map<String, serde_json::Value>, key: &str) -> Result<[u8; 3], String> {
    let entry = obj.get(key).ok_or_else(|| format!("missing key: {}", key))?
        .as_object().ok_or_else(|| format!("'{}' is not an object", key))?;
    for try_key in &["default", "dark", "light"] {
        let Some(inner) = entry.get(*try_key) else { continue };
        if let Some(hex) = inner.as_str() {
            return parse_hex(hex);
        }
        if let Some(color_obj) = inner.as_object() {
            if let Some(hex) = color_obj.get("color").and_then(|v| v.as_str()) {
                return parse_hex(hex);
            }
        }
    }
    Err(format!("could not find color in '{}'", key))
}

#[derive(Debug)]
pub struct MatugenTheme {
    pub primary: [u8; 3],
    pub on_primary: [u8; 3],
    pub surface: [u8; 3],
    pub on_surface: [u8; 3],
    pub surface_variant: [u8; 3],
    pub on_surface_variant: [u8; 3],
    pub error: [u8; 3],
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

fn build_anchor_mappings(source: &MatugenTheme, target: &NoctaliaTheme, verbose: bool) -> Vec<(Oklch<f32>, Oklch<f32>)> {
    let target_colors: Vec<Oklch<f32>> = target.palette()
        .into_iter()
        .map(|c| rgb_to_oklch(&Rgb(c)))
        .collect();

    let slots: Vec<(&str, [u8; 3], [u8; 3])> = vec![
        ("primary", source.primary, target.primary),
        ("on_primary", source.on_primary, target.on_primary),
        ("surface", source.surface, target.surface),
        ("on_surface", source.on_surface, target.on_surface),
        ("surface_variant", source.surface_variant, target.surface_variant),
        ("on_surface_variant", source.on_surface_variant, target.on_surface_variant),
    ];

    let mappings: Vec<_> = slots.into_iter().map(|(slot_name, src_rgb, target_rgb)| {
        let src_oklch = rgb_to_oklch(&Rgb(src_rgb));
        let target_oklch = rgb_to_oklch(&Rgb(target_rgb));

        if verbose {
            eprintln!("Source: {} | L:{:.3} C:{:.3} H:{:.1}°", slot_name, src_oklch.l, src_oklch.chroma, src_oklch.hue.into_positive_degrees());
            eprintln!("Target: {} | L:{:.3} C:{:.3} H:{:.1}°", slot_name, target_oklch.l, target_oklch.chroma, target_oklch.hue.into_positive_degrees());
            eprintln!("---");
        }

        (src_oklch, target_oklch)
    }).collect();

    mappings
}

fn fallback_mappings(input: &RgbImage, theme: &NoctaliaTheme) -> Vec<(Oklch<f32>, Oklch<f32>)> {
    const NUM_COLORS: usize = 7;

    let clusters = extract_quantette_palette(input, NUM_COLORS);

    if clusters.is_empty() {
        return build_identity_mappings(theme);
    }

    let theme_colors: Vec<Oklch<f32>> = theme.palette()
        .into_iter()
        .map(|c| rgb_to_oklch(&Rgb(c)))
        .collect();

    clusters.iter().map(|&cluster| {
        let target = if cluster.chroma < 0.06 {
            theme_colors.iter()
                .min_by(|a, b| {
                    let da = (a.l - cluster.l).abs();
                    let db = (b.l - cluster.l).abs();
                    da.partial_cmp(&db).unwrap()
                })
                .copied()
                .unwrap_or(cluster)
        } else {
            theme_colors.iter()
                .min_by(|a, b| {
                    let da = hue_dist(cluster.hue, a.hue);
                    let db = hue_dist(cluster.hue, b.hue);
                    da.partial_cmp(&db).unwrap()
                })
                .copied()
                .unwrap_or(cluster)
        };
        (cluster, target)
    }).collect()
}

fn build_identity_mappings(theme: &NoctaliaTheme) -> Vec<(Oklch<f32>, Oklch<f32>)> {
    theme.palette()
        .into_iter()
        .map(|c| {
            let oklch = rgb_to_oklch(&Rgb(c));
            (oklch, oklch)
        })
        .collect()
}

fn extract_quantette_palette(input: &RgbImage, num_colors: usize) -> Vec<Oklch<f32>> {
    let raw_pixels: Vec<palette::Srgb<u8>> = input.pixels()
        .map(|p| palette::Srgb::new(p[0], p[1], p[2]))
        .collect();

    let palette_size = PaletteSize::from_u8_clamped(num_colors as u8);

    let palette = Pipeline::new()
        .palette_size(palette_size)
        .quantize_method(QuantizeMethod::kmeans())
        .input_slice(&raw_pixels)
        .expect("valid slice input")
        .output_srgb8_palette();

    palette.into_iter().map(|rgb| {
        let s = palette::Srgb::new(rgb.red as f32 / 255.0, rgb.green as f32 / 255.0, rgb.blue as f32 / 255.0);
        Oklch::from_color(s.into_linear())
    }).collect()
}

const SATURATION_BOOST: f32 = 1.5;
const MAX_CHROMA: f32 = 0.4;

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

        let boosted_chroma = tgt.chroma * SATURATION_BOOST;
        let (ta, tb) = hue_to_ab(boosted_chroma.min(MAX_CHROMA), tgt.hue);
        out_l += w * mapped_l;
        out_a += w * ta;
        out_b += w * tb;
        total_w += w;
    }

    let final_l = (out_l / total_w).clamp(0.0, 1.0);
    let a = out_a / total_w;
    let b = out_b / total_w;
    let final_chroma = (a * a + b * b).sqrt().clamp(0.0, MAX_CHROMA);
    oklch_to_rgb(&Oklch {
        l: final_l,
        chroma: final_chroma,
        hue: OklabHue::from_degrees(b.atan2(a).to_degrees()),
    })
}

fn hue_to_ab(chroma: f32, hue: OklabHue<f32>) -> (f32, f32) {
    let r = hue.into_radians();
    (chroma * r.cos(), chroma * r.sin())
}

fn hue_dist(a: OklabHue<f32>, b: OklabHue<f32>) -> f32 {
    let diff = (a.into_degrees() - b.into_degrees()).abs() % 360.0;
    if diff > 180.0 { 360.0 - diff } else { diff }
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