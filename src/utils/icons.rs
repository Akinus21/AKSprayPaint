use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::io::Cursor;

use quick_xml::Reader;
use quick_xml::Writer;
use quick_xml::events::{Event, BytesStart};

use akspraypaint::{NoctaliaTheme, parse_theme};
use crate::utils::theme;

pub const ICON_THEME_NAME: &str = "PurpleHaze";

/// Icons are cached by theme hash only (not wallpaper), since a single icon set
/// serves any wallpaper.
#[allow(dead_code)]
pub fn icon_cache_root() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("akspraypaint")
        .join("icons")
}

pub fn icon_theme_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("/home"))
        .join("icons")
        .join(ICON_THEME_NAME)
}

fn theme_hash_for_icons(theme_data: &NoctaliaTheme) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(format!(
        "{:?}{:?}{:?}{:?}{:?}{:?}{:?}",
        theme_data.primary,
        theme_data.on_primary,
        theme_data.surface,
        theme_data.on_surface,
        theme_data.surface_variant,
        theme_data.on_surface_variant,
        theme_data.error,
    ));
    hex::encode(&hasher.finalize()[..4])
}

/// Reusable OKLCH color transfer — used by both wallpaper and icon paths.
pub fn recolor_image(
    input: &image::RgbImage,
    theme_data: &NoctaliaTheme,
    verbose: bool,
) -> image::RgbImage {
    crate::utils::recolor::recolor_wallpaper(input, theme_data, verbose)
}

/// Find the currently active system icon theme via gsettings.
pub fn get_active_icon_theme() -> Option<String> {
    let output = Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "icon-theme"])
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Some(stdout.trim_matches('\'').to_string())
}

/// Pick the best available icon theme to use as the source for recoloring.
/// Searches in priority order: the currently active gsettings theme (if it
/// exists on disk), then Adwaita, then the first theme found on disk.
pub fn find_best_base_theme() -> String {
    // Try the currently-active theme first
    if let Some(active) = get_active_icon_theme() {
        if !active.eq_ignore_ascii_case("PurpleHaze")
            && find_icon_theme_root(&active).is_some()
        {
            return active;
        }
    }
    // Fall back to Adwaita if it's installed
    if find_icon_theme_root("Adwaita").is_some() {
        return "Adwaita".to_string();
    }
    // Last resort: first theme found on disk
    let search_dirs: Vec<std::path::PathBuf> = std::iter::empty()
        .chain(dirs::data_dir().map(|p| p.join("icons")))
        .chain(
            ["/usr/share/icons", "/usr/local/share/icons"]
                .iter()
                .map(std::path::PathBuf::from),
        )
        .filter_map(|p| if p.exists() { Some(p) } else { None })
        .collect();

    for dir in search_dirs {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    if let Some(name) = entry.path().file_name().and_then(|n| n.to_str()) {
                        if name != "icons" && name != "CursorThemes" {
                            return name.to_string();
                        }
                    }
                }
            }
        }
    }
    "Adwaita".to_string()
}

/// Find the root directory of an installed icon theme by name.
pub fn find_icon_theme_root(name: &str) -> Option<PathBuf> {
    let search_dirs: Vec<PathBuf> = std::iter::empty()
        .chain(dirs::data_dir().map(|p| p.join("icons")))
        .chain(["/usr/share/icons", "/usr/local/share/icons"].iter().map(PathBuf::from))
        .filter_map(|p| if p.exists() { Some(p) } else { None })
        .collect();

    for dir in search_dirs {
        let candidate = dir.join(name);
        if candidate.is_dir()
        && (candidate.join("index.theme").exists()
            || candidate.join("16x16").is_dir()
            || candidate.join("scalable").is_dir())
    {
        return Some(candidate);
    }
    }
    None
}

/// Recolor all icons from the base theme and write to the PurpleHaze output dir.
pub fn recolor_icons(base_theme_name: &str, verbose: bool) -> Result<String, String> {
    let (_, theme_content) = theme::read_theme()?;
    let theme_data = parse_theme(&theme_content)
        .ok_or_else(|| "failed to parse theme from colors.json".to_string())?;
    let hash = theme_hash_for_icons(&theme_data);

    let base_root = find_icon_theme_root(base_theme_name)
        .ok_or_else(|| format!("icon theme '{}' not found", base_theme_name))?;

    let output_dir = icon_theme_dir();
    std::fs::create_dir_all(&output_dir)
        .map_err(|e| format!("failed to create icon output dir: {}", e))?;

    if verbose {
        eprintln!("Base theme: {} ({})", base_theme_name, base_root.display());
        eprintln!("Output dir: {}", output_dir.display());
        eprintln!("Theme hash: {}", hash);
    }

    // Collect sizes to process
    let sizes = ["16x16", "22x22", "24x24", "32x32", "48x48", "64x64", "128x128", "256x256"];
    let mut total = 0usize;
    let mut errors = 0usize;

    for size in &sizes {
        let size_base = base_root.join(size);
        if !size_base.is_dir() {
            continue;
        }
        let out_size = output_dir.join(size);
        std::fs::create_dir_all(&out_size)
            .map_err(|e| format!("failed to create {} dir: {}", size, e))?;

        let entries = match std::fs::read_dir(&size_base) {
            Ok(e) => e,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_symlink() {
                continue;
            }
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            if stem.ends_with("_recolored") {
                continue;
            }

            let result: Result<(), String> = if ext == "svg" {
                recolor_svg_icon(&path, &theme_data, verbose).map(|_| ())
            } else if matches!(ext, "png" | "jpg" | "jpeg" | "webp" | "bmp") {
                recolor_raster_icon(&path, &theme_data).map(|_| ())
            } else {
                // Copy non-image files as-is
                let dst = out_size.join(path.file_name().unwrap());
                std::fs::copy(&path, &dst)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            };

            match result {
                Ok(_) => total += 1,
                Err(e) => {
                    errors += 1;
                    if verbose {
                        eprintln!("  [{}] {}: {}", ext, path.display(), e);
                    }
                }
            }
        }
    }

    // Process scalable SVGs by subdirectory
    let scalable_base = base_root.join("scalable");
    if scalable_base.is_dir() {
        let out_scalable = output_dir.join("scalable");
        std::fs::create_dir_all(&out_scalable)
            .map_err(|e| format!("failed to create scalable dir: {}", e))?;

        let subdirs = [
            "actions", "apps", "categories", "devices", "emblems",
            "mimetypes", "places", "status",
        ];

        for sub in &subdirs {
            let src_sub = scalable_base.join(sub);
            let dst_sub = out_scalable.join(sub);
            if !src_sub.is_dir() {
                continue;
            }
            std::fs::create_dir_all(&dst_sub).ok();

            let entries = match std::fs::read_dir(&src_sub) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("svg") {
                    continue;
                }
                match recolor_svg_icon(&path, &theme_data, verbose) {
                    Ok(_) => {
                        let name = path.file_name().unwrap();
                        let src_size_dir = path
                            .parent()
                            .and_then(|p| p.file_name())
                            .unwrap_or(".".as_ref());
                        let src_recolored = output_dir.join(src_size_dir).join(name);
                        std::fs::copy(&src_recolored, dst_sub.join(name)).ok();
                        total += 1;
                    }
                    Err(e) => {
                        errors += 1;
                        if verbose {
                            eprintln!("  [scalable/{}] {}: {}", sub, path.display(), e);
                        }
                    }
                }
            }
        }
    }

    generate_index_theme(&output_dir, base_theme_name)?;

    if verbose {
        eprintln!("Recolored {} icons ({} errors)", total, errors);
    }

    Ok(hash)
}

/// Generate an index.theme file so GTK recognises the icon theme.
fn generate_index_theme(output_dir: &Path, base_name: &str) -> Result<(), String> {
    let index_content = format!(
        "[Icon Theme]\n\
         Name={}\n\
         Comment=Recolored by AKSprayPaint from {}\n\
         DisplayName=Purple Haze\n\
         Inherits={}\n\
         Example=folder\n\
         FollowsNav=True\n\
         \n\
         [Directories]\n\
         16x16=status\n\
         22x22=status\n\
         24x24=status\n\
         32x32=actions\n\
         48x48=devices\n\
         64x64=actions\n\
         128x128=mimetypes\n\
         256x256=apps\n\
         scalable/actions=svg\n\
         scalable/apps=svg\n\
         scalable/categories=svg\n\
         scalable/devices=svg\n\
         scalable/emblems=svg\n\
         scalable/mimetypes=svg\n\
         scalable/places=svg\n\
         scalable/status=svg\n",
        ICON_THEME_NAME,
        base_name,
        base_name
    );

    std::fs::write(output_dir.join("index.theme"), index_content)
        .map_err(|e| format!("failed to write index.theme: {}", e))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// SVG color extraction & recoloring
// ---------------------------------------------------------------------------

/// Extract all distinct fill/stroke color values from an SVG.
fn extract_svg_palette(svg_bytes: &[u8]) -> Result<Vec<String>, String> {
    let mut reader = Reader::from_reader(Cursor::new(svg_bytes));
    reader.config_mut().trim_text(true);

    let mut hex_colors: Vec<String> = Vec::new();

    loop {
        let mut buf = Vec::new();
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let mut binding = e.attributes();
                let attrs = binding.with_checks(false);
                for attr_result in attrs {
                    let attr = attr_result.map_err(|e| format!("attr error: {}", e))?;
                    let key = std::str::from_utf8(attr.key.as_ref()).unwrap_or("");

                    if key == "fill" || key == "stroke" {
                        if let Ok(val) = attr.unescape_value() {
                            let s = val.as_ref();
                            if !s.is_empty()
                                && s != "none"
                                && s != "transparent"
                                && s != "currentColor"
                            {
                                hex_colors.push(s.to_string());
                            }
                        }
                    }
                    if key == "style" {
                        if let Ok(val) = attr.unescape_value() {
                            for part in val.as_ref().split(';') {
                                let part = part.trim();
                                if (part.starts_with("fill:") || part.starts_with("stroke:"))
                                    && !part.contains("url(")
                                {
                                    let val_part = part.split(':').nth(1).unwrap_or("").trim();
                                    if val_part.starts_with('#') || val_part.starts_with("rgb") {
                                        hex_colors.push(val_part.to_string());
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("SVG parse error: {}", e)),
            _ => {}
        }
    }

    hex_colors.sort();
    hex_colors.dedup();
    Ok(hex_colors)
}

/// Parse a hex string (#rrggbb or #rgb) into RGB bytes.
fn parse_svg_color(s: &str) -> Option<[u8; 3]> {
    let s = s.trim();
    if !s.starts_with('#') {
        return None;
    }
    let hex = s.trim_start_matches('#');
    match hex.len() {
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some([r, g, b])
        }
        3 => {
            let r = u8::from_str_radix(&hex[0..1].repeat(2), 16).ok()?;
            let g = u8::from_str_radix(&hex[1..2].repeat(2), 16).ok()?;
            let b = u8::from_str_radix(&hex[2..3].repeat(2), 16).ok()?;
            Some([r, g, b])
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// OKLCH color math
// ---------------------------------------------------------------------------

use palette::{FromColor, IntoColor, Oklch, OklabHue, Srgb};

fn rgb_to_oklch(rgb: [u8; 3]) -> Oklch<f32> {
    let s = Srgb::new(rgb[0] as f32 / 255.0, rgb[1] as f32 / 255.0, rgb[2] as f32 / 255.0);
    Oklch::from_color(s.into_linear())
}

fn oklch_to_rgb(oklch: Oklch<f32>) -> [u8; 3] {
    let linear: palette::LinSrgb<f32> = oklch.into_color();
    let srgb: Srgb<f32> = linear.into_encoding();
    [
        (srgb.red.clamp(0.0, 1.0) * 255.0).round() as u8,
        (srgb.green.clamp(0.0, 1.0) * 255.0).round() as u8,
        (srgb.blue.clamp(0.0, 1.0) * 255.0).round() as u8,
    ]
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

// ---------------------------------------------------------------------------
// Anchor mapping
// ---------------------------------------------------------------------------

/// Build anchor mappings: each unique SVG color → closest theme color.
fn build_svg_anchor_mappings(
    svg_colors: &[String],
    theme_data: &NoctaliaTheme,
) -> std::collections::HashMap<String, String> {
    let theme_colors: Vec<(Oklch<f32>, [u8; 3])> = vec![
        (rgb_to_oklch(theme_data.primary), theme_data.primary),
        (rgb_to_oklch(theme_data.on_primary), theme_data.on_primary),
        (rgb_to_oklch(theme_data.surface), theme_data.surface),
        (rgb_to_oklch(theme_data.on_surface), theme_data.on_surface),
        (rgb_to_oklch(theme_data.surface_variant), theme_data.surface_variant),
        (rgb_to_oklch(theme_data.on_surface_variant), theme_data.on_surface_variant),
        (rgb_to_oklch(theme_data.error), theme_data.error),
    ];

    let mut result = std::collections::HashMap::new();
    for hex in svg_colors {
        if result.contains_key(hex) {
            continue;
        }
        let rgb = match parse_svg_color(hex) {
            Some(r) => r,
            None => continue,
        };
        let src_oklch = rgb_to_oklch(rgb);

        let closest_rgb = theme_colors
            .iter()
            .min_by(|(a_oklch, _), (b_oklch, _)| {
                let da = oklch_dist(&src_oklch, a_oklch);
                let db = oklch_dist(&src_oklch, b_oklch);
                da.partial_cmp(&db).unwrap()
            })
            .map(|(_, rgb)| *rgb)
            .unwrap_or(rgb);

        let tgt_oklch = rgb_to_oklch(closest_rgb);
        let tgt_rgb = oklch_to_rgb(tgt_oklch);
        let tgt_hex = format!("#{:02x}{:02x}{:02x}", tgt_rgb[0], tgt_rgb[1], tgt_rgb[2]);

        if hex != &tgt_hex {
            result.insert(hex.clone(), tgt_hex);
        }
    }
    result
}

// ---------------------------------------------------------------------------
// SVG recoloring
// ---------------------------------------------------------------------------

/// Recolor a single SVG icon by rewriting fill/stroke color attributes.
fn recolor_svg_icon(src: &Path, theme_data: &NoctaliaTheme, verbose: bool) -> Result<PathBuf, String> {
    let svg_bytes = std::fs::read(src).map_err(|e| format!("failed to read SVG: {}", e))?;

    let palette = extract_svg_palette(&svg_bytes)?;
    if palette.is_empty() {
        return copy_icon_as_is(src);
    }

    let mappings = build_svg_anchor_mappings(&palette, theme_data);
    if mappings.is_empty() {
        return copy_icon_as_is(src);
    }

    if verbose {
        for (src_hex, tgt_hex) in &mappings {
            eprintln!("  {} → {}", src_hex, tgt_hex);
        }
    }

    let recolored = transfer_svg_colors(&svg_bytes, &mappings)?;

    let name = src.file_name().unwrap();
    let size_dir = src
        .parent()
        .and_then(|p| p.file_name())
        .unwrap_or(".".as_ref());
    let out_dir = icon_theme_dir().join(size_dir);
    std::fs::create_dir_all(&out_dir)
        .map_err(|e| format!("failed to create dir: {}", e))?;
    let out_path = out_dir.join(name);
    std::fs::write(&out_path, recolored)
        .map_err(|e| format!("failed to write: {}", e))?;

    Ok(out_path)
}

fn copy_icon_as_is(src: &Path) -> Result<PathBuf, String> {
    let name = src.file_name().unwrap();
    let size_dir = src
        .parent()
        .and_then(|p| p.file_name())
        .unwrap_or(".".as_ref());
    let out_dir = icon_theme_dir().join(size_dir);
    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
    let out_path = out_dir.join(name);
    std::fs::copy(src, &out_path).map_err(|e| e.to_string())?;
    Ok(out_path)
}

/// Apply the color mappings to all fill/stroke attributes in SVG bytes.
fn transfer_svg_colors(
    svg_bytes: &[u8],
    mappings: &std::collections::HashMap<String, String>,
) -> Result<Vec<u8>, String> {
    let mut reader = Reader::from_reader(Cursor::new(svg_bytes));
    reader.config_mut().trim_text(true);

    let mut writer = Writer::new(Cursor::new(Vec::new()));

    loop {
        let mut buf = Vec::new();
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let recolored = rewrite_element_attrs(e, mappings)?;
                writer
                    .write_event(Event::Start(recolored))
                    .map_err(|e| format!("SVG write error: {}", e))?;
            }
            Ok(Event::Empty(e)) => {
                let recolored = rewrite_element_attrs(e, mappings)?;
                writer
                    .write_event(Event::Empty(recolored))
                    .map_err(|e| format!("SVG write error: {}", e))?;
            }
            Ok(Event::Eof) => {
                writer.write_event(Event::Eof).map_err(|e| format!("SVG write error: {}", e))?;
                break;
            }
            Ok(e) => {
                writer
                    .write_event(e)
                    .map_err(|e| format!("SVG write error: {}", e))?;
            }
            Err(e) => return Err(format!("SVG read error: {}", e)),
        }
    }

    Ok(writer.into_inner().into_inner())
}

/// Rewrite fill/stroke attributes on a BytesStart element using the mappings.
fn rewrite_element_attrs(
    elem: BytesStart<'_>,
    mappings: &std::collections::HashMap<String, String>,
) -> Result<BytesStart<'static>, String> {
    // Extract all attribute data into owned types first, then build a new
    // BytesStart from scratch so we avoid borrow conflicts with clear_attributes.
    let elem_name_bytes = elem.name().into_inner().to_vec();

    let mut new_attrs: Vec<(String, String)> = Vec::new();

    let mut binding = elem.attributes();
    let attrs = binding.with_checks(false);
    for attr_result in attrs {
        let attr = attr_result.map_err(|e| format!("attr error: {}", e))?;
        let key = std::str::from_utf8(attr.key.as_ref())
            .unwrap_or("")
            .to_string();
        let value_unescaped = attr
            .unescape_value()
            .map(|v| v.as_ref().to_string())
            .unwrap_or_default();

        if key == "fill" || key == "stroke" {
            if let Some(replacement) = mappings.get(&value_unescaped) {
                new_attrs.push((key, replacement.clone()));
                continue;
            }
        }

        if key == "style" {
            let new_style = rewrite_style_value(&value_unescaped, mappings);
            if new_style != value_unescaped {
                new_attrs.push((key, new_style));
                continue;
            }
        }

        new_attrs.push((key, value_unescaped));
    }

    // Build a new BytesStart with the original name and new attributes.
    // We own elem_name_bytes so we can safely pass it as owned data.
    let elem_name_str = String::from_utf8_lossy(&elem_name_bytes);
    let mut result = BytesStart::new(elem_name_str.into_owned());
    for (key, value) in new_attrs {
        result.push_attribute((key.as_str(), value.as_str()));
    }

    Ok(result)
}

/// Rewrite fill:/stroke: sub-values inside a style attribute string.
fn rewrite_style_value(
    style: &str,
    mappings: &std::collections::HashMap<String, String>,
) -> String {
    let mut result = style.to_string();
    for (src, tgt) in mappings.iter() {
        for prefix in &["fill:", "stroke:"] {
            let pattern = format!("{} {}", prefix, src);
            let replacement = format!("{} {}", prefix, tgt);
            result = result.replace(&pattern, &replacement);
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Raster icon recoloring
// ---------------------------------------------------------------------------

/// Recolor a PNG/JPEG/WebP icon using the full pixel transfer pipeline.
fn recolor_raster_icon(src: &Path, theme_data: &NoctaliaTheme) -> Result<PathBuf, String> {
    let img = image::open(src).map_err(|e| format!("failed to open image: {}", e))?;
    let rgb_img = img.to_rgb8();
    let recolored = recolor_image(&rgb_img, theme_data, false);

    let name = src.file_name().unwrap();
    let size_dir = src
        .parent()
        .and_then(|p| p.file_name())
        .unwrap_or(".".as_ref());
    let out_path = icon_theme_dir().join(size_dir).join(name);

    recolored
        .save(&out_path)
        .map_err(|e| format!("failed to save icon: {}", e))?;
    Ok(out_path)
}

// ---------------------------------------------------------------------------
// Apply theme
// ---------------------------------------------------------------------------

/// Apply the PurpleHaze icon theme via gsettings.
pub fn apply_icon_theme() -> Result<(), String> {
    let output = Command::new("gsettings")
        .args(["set", "org.gnome.desktop.interface", "icon-theme", ICON_THEME_NAME])
        .output()
        .map_err(|e| format!("failed to run gsettings: {}", e))?;
    if !output.status.success() {
        return Err(format!(
            "gsettings failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_svg_color() {
        assert_eq!(parse_svg_color("#ff0000"), Some([255, 0, 0]));
        assert_eq!(parse_svg_color("#f00"), Some([255, 0, 0]));
        assert_eq!(parse_svg_color("#aabbcc"), Some([170, 187, 204]));
        assert_eq!(parse_svg_color("none"), None);
        assert_eq!(parse_svg_color("currentColor"), None);
    }

    #[test]
    fn test_rgb_oklch_roundtrip() {
        let orig = [100u8, 150, 200];
        let oklch = rgb_to_oklch(orig);
        let back = oklch_to_rgb(oklch);
        let diff = (back[0] as i32 - orig[0] as i32).abs()
            + (back[1] as i32 - orig[1] as i32).abs()
            + (back[2] as i32 - orig[2] as i32).abs();
        assert!(diff < 10, "roundtrip should be close, diff={}", diff);
    }
}
