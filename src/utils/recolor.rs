use image::{Rgb, RgbImage};
use palette::{FromColor, IntoColor, Oklch, OklabHue, Srgb};

use akspraypaint::NoctaliaTheme;

// ─────────────────────────────────────────────────────────────────────────────
// CONSTANTS
// ─────────────────────────────────────────────────────────────────────────────

const SHARPNESS: f32 = 8.0;
const EPSILON: f32 = 1e-6;

/// Ollama endpoint. Override with AKSPRAYPAINT_OLLAMA_URL env var.
const OLLAMA_DEFAULT_URL: &str = "https://ollama.akinus21.com";

/// Vision model to use for color mapping.
const OLLAMA_MODEL: &str = "kimi-k2.6:cloud";

/// Timeout for the vision call in seconds.
const VISION_TIMEOUT_SECS: u64 = 30;

/// Max dimension to resize image to before sending to vision model.
const VISION_MAX_DIM: u32 = 512;

// ─────────────────────────────────────────────────────────────────────────────
// PUBLIC API
// ─────────────────────────────────────────────────────────────────────────────

/// Recolor `input` so its colors match the noctalia `theme`.
pub fn recolor_wallpaper(input: &RgbImage, theme: &NoctaliaTheme) -> RgbImage {
    let mappings = match vision_color_mappings(input, theme) {
        Ok(m) if !m.is_empty() => {
            eprintln!("Vision mapping: {} anchors from LLM", m.len());
            m
        }
        Ok(_) | Err(_) => {
            eprintln!("Falling back to hue-family");
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

// ─────────────────────────────────────────────────────────────────────────────
// VISION MODEL COLOR MAPPING
// ─────────────────────────────────────────────────────────────────────────────

fn vision_color_mappings(
    input: &RgbImage,
    theme: &NoctaliaTheme,
) -> Result<Vec<(Oklch<f32>, Oklch<f32>)>, String> {
    let base64_img = encode_image_for_vision(input)?;
    let theme_desc = theme_description(theme);
    let prompt = build_prompt(&theme_desc);

    let response = call_ollama(&base64_img, &prompt)?;
    parse_mappings(&response, theme)
}

fn encode_image_for_vision(input: &RgbImage) -> Result<String, String> {
    let (w, h) = input.dimensions();
    let scale = (VISION_MAX_DIM as f32 / w.max(h) as f32).min(1.0);
    let nw = (w as f32 * scale) as u32;
    let nh = (h as f32 * scale) as u32;

    let resized = image::imageops::resize(input, nw, nh, image::imageops::FilterType::Lanczos3);

    let mut buf = std::io::Cursor::new(Vec::new());
    resized.write_to(&mut buf, image::ImageFormat::Png)
        .map_err(|e| format!("failed to encode image: {}", e))?;

    Ok(base64_encode(buf.get_ref()))
}

fn theme_description(theme: &NoctaliaTheme) -> String {
    format!(
        "primary: {}\non_primary: {}\nsurface: {}\non_surface: {}\nsurface_variant: {}\non_surface_variant: {}\nerror: {}",
        rgb_to_hex(theme.primary),
        rgb_to_hex(theme.on_primary),
        rgb_to_hex(theme.surface),
        rgb_to_hex(theme.on_surface),
        rgb_to_hex(theme.surface_variant),
        rgb_to_hex(theme.on_surface_variant),
        rgb_to_hex(theme.error),
    )
}

fn rgb_to_hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

fn build_prompt(theme_desc: &str) -> String {
    let prompt = format!(
        "You are a color mapping assistant. I will show you a wallpaper image and a color theme palette.\n\
         \n\
         Your task:\n\
         1. Identify the dominant color regions in the image (background, main subject, outlines, highlights)\n\
         2. For each region, pick the most semantically appropriate theme slot\n\
         \n\
         Theme palette (slot_name: hex_color):\n\
         {}\n\
         \n\
         Respond ONLY with a JSON array. No explanation, no markdown.\n\
         Each element must have exactly these fields:\n\
           source_hex: the hex color of the region in the original image (example: 1a1e3d)\n\
           theme_slot: one of: primary, on_primary, surface, on_surface, surface_variant, on_surface_variant, error\n\
         \n\
         Example response format:\n\
         [{{\"source_hex\": \"1a1a1f\", \"theme_slot\": \"surface\"}}, {{\"source_hex\": \"eff08a\", \"theme_slot\": \"on_surface_variant\"}}, {{\"source_hex\": \"8890d0\", \"theme_slot\": \"primary\"}}]\n\
         \n\
         Identify at least 4 and at most 8 color regions. Include the background, the main subject, outlines, and any highlights.",
        theme_desc
    );
    prompt
}

fn call_ollama(base64_img: &str, prompt: &str) -> Result<String, String> {
    let url = std::env::var("AKSPRAYPAINT_OLLAMA_URL")
        .unwrap_or_else(|_| OLLAMA_DEFAULT_URL.to_string());
    let endpoint = format!("{}/api/generate", url);

    let body = format!(
        "{{\"model\":\"{}\",\"prompt\":{},\"images\":[\"{}\"],\"stream\":false}}",
        OLLAMA_MODEL,
        serde_json::to_string(prompt).map_err(|e| e.to_string())?,
        base64_img,
    );

    let output = std::process::Command::new("curl")
        .args(["-s", "-X", "POST", "-H", "Content-Type: application/json",
               "--max-time", &VISION_TIMEOUT_SECS.to_string(), "-d", &body, &endpoint])
        .output()
        .map_err(|e| format!("curl failed: {}", e))?;

    if !output.status.success() {
        return Err(format!(
            "curl exited {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    extract_ollama_response(&stdout)
}

fn extract_ollama_response(raw: &str) -> Result<String, String> {
    let key = "\"response\":\"";
    let start = raw.find(key)
        .ok_or_else(|| format!("no 'response' field in: {}", &raw[..raw.len().min(200)]))?
        + key.len();

    let rest = &raw[start..];
    if !rest.starts_with('"') {
        return Err(format!("response value is not a string: {}", &rest[..rest.len().min(100)]));
    }

    let chars: Vec<char> = rest.chars().collect();
    let mut i = 1;
    while i < chars.len() {
        if chars[i] == '"' && chars[i-1] != '\\' {
            return Ok(chars[1..i].iter().collect());
        }
        i += 1;
    }
    Err("could not find closing quote".to_string())
}

fn parse_mappings(
    response: &str,
    theme: &NoctaliaTheme,
) -> Result<Vec<(Oklch<f32>, Oklch<f32>)>, String> {
    let start = response.find('[')
        .ok_or_else(|| format!("no JSON array found in response: {}", &response[..response.len().min(300)]))?;
    let end = response.rfind(']')
        .ok_or_else(|| "no closing ] found".to_string())?
        + 1;

    let json_str = &response[start..end];

    let items: Vec<serde_json::Value> = serde_json::from_str(json_str)
        .map_err(|e| format!("JSON parse error: {} — raw: {}", e, &json_str[..json_str.len().min(300)]))?;

    let mut mappings = Vec::new();
    for item in &items {
        let source_hex = item["source_hex"].as_str()
            .ok_or("missing source_hex")?;
        let theme_slot = item["theme_slot"].as_str()
            .ok_or("missing theme_slot")?;

        let source = parse_hex_to_oklch(source_hex)
            .ok_or_else(|| format!("invalid source_hex: {}", source_hex))?;
        let target = slot_to_oklch(theme_slot, theme)
            .ok_or_else(|| format!("unknown theme_slot: {}", theme_slot))?;

        mappings.push((source, target));
    }

    Ok(mappings)
}

fn parse_hex_to_oklch(hex: &str) -> Option<Oklch<f32>> {
    let hex = hex.trim();
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(rgb_arr_to_oklch([r, g, b]))
}

fn slot_to_oklch(slot: &str, theme: &NoctaliaTheme) -> Option<Oklch<f32>> {
    match slot {
        "primary" => Some(rgb_arr_to_oklch(theme.primary)),
        "on_primary" => Some(rgb_arr_to_oklch(theme.on_primary)),
        "surface" => Some(rgb_arr_to_oklch(theme.surface)),
        "on_surface" => Some(rgb_arr_to_oklch(theme.on_surface)),
        "surface_variant" => Some(rgb_arr_to_oklch(theme.surface_variant)),
        "on_surface_variant" => Some(rgb_arr_to_oklch(theme.on_surface_variant)),
        "error" => Some(rgb_arr_to_oklch(theme.error)),
        _ => None,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// FALLBACK — HUE-FAMILY MATCHING
// ─────────────────────────────────────────────────────────────────────────────

fn fallback_mappings(
    input: &RgbImage,
    theme: &NoctaliaTheme,
) -> Vec<(Oklch<f32>, Oklch<f32>)> {
    const K: usize = 7;
    const KMEANS_ITER: usize = 12;
    const MAX_SAMPLES: usize = 40_000;

    let samples = sample_pixels(input, MAX_SAMPLES);
    let clusters = kmeans(&samples, K, KMEANS_ITER);
    let theme_colors: Vec<Oklch<f32>> = theme.palette()
        .into_iter()
        .map(rgb_arr_to_oklch)
        .collect();
    match_clusters_hue_family(&clusters, &theme_colors)
}

fn match_clusters_hue_family(
    clusters: &[Oklch<f32>],
    theme_colors: &[Oklch<f32>],
) -> Vec<(Oklch<f32>, Oklch<f32>)> {
    clusters.iter().map(|&src| {
        let target = if src.chroma < 0.06 {
            *theme_colors.iter()
                .min_by(|a, b| {
                    let da = (a.l - src.l).abs();
                    let db = (b.l - src.l).abs();
                    da.partial_cmp(&db).unwrap()
                })
                .unwrap()
        } else {
            *theme_colors.iter()
                .min_by(|a, b| {
                    let da = hue_dist(src.hue, a.hue);
                    let db = hue_dist(src.hue, b.hue);
                    da.partial_cmp(&db).unwrap()
                })
                .unwrap()
        };
        (src, target)
    }).collect()
}

fn sample_pixels(img: &RgbImage, max_samples: usize) -> Vec<Oklch<f32>> {
    let (w, h) = img.dimensions();
    let stride = ((w * h) as usize / max_samples).max(1);
    img.pixels().enumerate()
        .filter(|(i, _)| i % stride == 0)
        .map(|(_, p)| rgb_to_oklch(p))
        .collect()
}

fn kmeans(points: &[Oklch<f32>], k: usize, iters: usize) -> Vec<Oklch<f32>> {
    if points.is_empty() || k == 0 {
        return vec![];
    }
    let mut c = vec![points[points.len() / 4]];
    for _ in 1..k {
        let dists: Vec<f32> = points.iter()
            .map(|p| c.iter().map(|q| oklch_dist(p, q)).fold(f32::MAX, f32::min))
            .collect();
        let next = dists.iter().enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
            .map(|(i, _)| i).unwrap_or(0);
        c.push(points[next]);
    }
    for _ in 0..iters {
        let assignments: Vec<usize> = points.iter()
            .map(|p| c.iter().enumerate()
                .min_by(|(_, a), (_, b)| oklch_dist(p, a).partial_cmp(&oklch_dist(p, b)).unwrap())
                .map(|(i, _)| i).unwrap_or(0))
            .collect();
        let mut sums = vec![[0.0f32; 3]; k];
        let mut counts = vec![0usize; k];
        for (p, &ci) in points.iter().zip(&assignments) {
            let (a, b) = hue_to_ab(p.chroma, p.hue);
            sums[ci][0] += p.l;
            sums[ci][1] += a;
            sums[ci][2] += b;
            counts[ci] += 1;
        }
        for i in 0..k {
            if counts[i] == 0 {
                continue;
            }
            let n = counts[i] as f32;
            let (l, a, b) = (sums[i][0] / n, sums[i][1] / n, sums[i][2] / n);
            c[i] = Oklch {
                l,
                chroma: (a * a + b * b).sqrt(),
                hue: OklabHue::from_degrees(b.atan2(a).to_degrees()),
            };
        }
    }
    c
}

// ─────────────────────────────────────────────────────────────────────────────
// TRANSFER
// ─────────────────────────────────────────────────────────────────────────────

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

// ─────────────────────────────────────────────────────────────────────────────
// BASE64
// ─────────────────────────────────────────────────────────────────────────────

fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(CHARS[((n >> 18) & 63) as usize] as char);
        out.push(CHARS[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { CHARS[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { CHARS[(n & 63) as usize] as char } else { '=' });
    }
    out
}

// ─────────────────────────────────────────────────────────────────────────────
// HELPERS
// ─────────────────────────────────────────────────────────────────────────────

fn hue_dist(a: OklabHue<f32>, b: OklabHue<f32>) -> f32 {
    let diff = (a.into_degrees() - b.into_degrees()).abs() % 360.0;
    if diff > 180.0 {
        360.0 - diff
    } else {
        diff
    }
}

fn oklch_dist(a: &Oklch<f32>, b: &Oklch<f32>) -> f32 {
    let (aa, ab) = hue_to_ab(a.chroma, a.hue);
    let (ba, bb) = hue_to_ab(b.chroma, b.hue);
    let dl = a.l - b.l;
    ((dl * dl) + (aa - ba).powi(2) + (ab - bb).powi(2)).sqrt()
}

fn hue_to_ab(chroma: f32, hue: OklabHue<f32>) -> (f32, f32) {
    let r = hue.into_radians();
    (chroma * r.cos(), chroma * r.sin())
}

fn rgb_arr_to_oklch(arr: [u8; 3]) -> Oklch<f32> {
    rgb_to_oklch(&Rgb(arr))
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

// ─────────────────────────────────────────────────────────────────────────────
// TESTS
// ─────────────────────────────────────────────────────────────────────────────

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
    fn test_recolor_runs_fallback() {
        let mut img = RgbImage::new(32, 32);
        for p in img.pixels_mut() {
            *p = Rgb([100, 64, 192]);
        }
        assert_eq!(recolor_wallpaper(&img, &test_theme()).dimensions(), (32, 32));
    }

    #[test]
    fn test_parse_mappings_valid() {
        let theme = test_theme();
        let response = r#"[
            {"source_hex": "1a1a1f", "theme_slot": "surface"},
            {"source_hex": "eff08a", "theme_slot": "on_surface_variant"},
            {"source_hex": "8890d0", "theme_slot": "primary"}
        ]"#;
        let result = parse_mappings(response, &theme).unwrap();
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_parse_hex_valid() {
        let c = parse_hex_to_oklch("ff0000").unwrap();
        assert!(c.chroma > 0.1, "red should have high chroma");
    }

    #[test]
    fn test_slot_to_oklch_all_slots() {
        let theme = test_theme();
        for slot in &["primary", "on_primary", "surface", "on_surface",
                      "surface_variant", "on_surface_variant", "error"] {
            assert!(slot_to_oklch(slot, &theme).is_some(), "slot {} should resolve", slot);
        }
        assert!(slot_to_oklch("bogus", &theme).is_none());
    }

    #[test]
    fn test_base64_roundtrip_known() {
        assert_eq!(base64_encode(b"Man"), "TWFu");
        assert_eq!(base64_encode(b"Ma"), "TWE=");
        assert_eq!(base64_encode(b"M"), "TQ==");
    }

    #[test]
    fn test_rgb_to_hex() {
        assert_eq!(rgb_to_hex([255, 0, 128]), "#ff0080");
        assert_eq!(rgb_to_hex([0, 0, 0]), "#000000");
    }
}
