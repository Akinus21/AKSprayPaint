use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::io::Cursor;

use quick_xml::Reader;
use quick_xml::Writer;
use quick_xml::events::{Event, BytesStart};

use akspraypaint::{NoctaliaTheme, parse_theme};
use crate::utils::theme;

// --------------------------------------------------------------------------
// Theme name resolution
// --------------------------------------------------------------------------

/// Read the current Noctalia theme name from settings.toml.
/// Resolves the `source` field to find which key holds the active theme name.
pub fn get_current_theme_name() -> Result<String, String> {
    let state_dir = theme::noctalia_state_dir()
        .ok_or_else(|| "noctalia state directory not found (~/.local/state/noctalia)".to_string())?;
    let settings_path = state_dir.join("settings.toml");
    let content = std::fs::read_to_string(&settings_path)
        .map_err(|e| format!("failed to read settings.toml: {}", e))?;

    let source = extract_toml_string(&content, "source")
        .unwrap_or_else(|| "custom".to_string());

    let theme_name = match source.as_str() {
        "builtin" => extract_toml_string(&content, "builtin"),
        "community_palette" => extract_toml_string(&content, "community_palette"),
        "custom" => extract_toml_string(&content, "custom_palette"),
        _ => extract_toml_string(&content, "custom_palette"),
    }
    .unwrap_or_else(|| "Custom".to_string());

    // Convert "Purple Haze" → "Purple_Haze" for use as folder/icon-theme name
    let sanitized = theme_name
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("_");

    Ok(sanitized)
}

/// Extract a string value for a top-level key from TOML content.
fn extract_toml_string(content: &str, key: &str) -> Option<String> {
    for line in content.lines() {
        let line = line.trim();
        if !line.starts_with(key) {
            continue;
        }
        let rest = &line[key.len()..];
        let rest = rest.trim_start();
        if !rest.starts_with('=') {
            continue;
        }
        let rest = rest[1..].trim();
        // Remove surrounding quotes
        let val = rest.trim_matches('"').trim_matches('\'');
        return Some(val.to_string());
    }
    None
}

// --------------------------------------------------------------------------
// Icon theme discovery
// --------------------------------------------------------------------------

/// Find the root directory of an installed icon theme by name.
/// When multiple installs exist (e.g. Adwaita in both ~/.local and /usr/share),
/// prefers the one with actual icon files over an empty placeholder.
pub fn find_icon_theme_root(name: &str) -> Option<PathBuf> {
    let search_dirs: Vec<PathBuf> = std::iter::empty()
        .chain(dirs::data_dir().map(|p| p.join("icons")))
        .chain(["/usr/share/icons", "/usr/local/share/icons"].iter().map(PathBuf::from))
        .filter_map(|p| if p.exists() { Some(p) } else { None })
        .collect();

    let mut best: Option<(PathBuf, bool)> = None;

    for dir in search_dirs {
        let candidate = dir.join(name);
        if !candidate.is_dir() {
            continue;
        }
        let has_index = candidate.join("index.theme").exists();
        let has_size_dirs = candidate.join("16x16").is_dir()
            || candidate.join("scalable").is_dir()
            || candidate.join("symbolic").is_dir();

        if !has_index && !has_size_dirs {
            continue;
        }

        let has_content = check_theme_has_icons(&candidate);
        let is_local = dir.to_string_lossy().contains(".local");

        let should_replace = match &best {
            None => true,
            Some((_, existing_has_content)) => {
                has_content && !*existing_has_content
                    || (has_content == *existing_has_content && is_local)
            }
        };
        if should_replace {
            best = Some((candidate, has_content));
        }
    }

    best.map(|(p, _)| p)
}

/// Check if an icon theme directory has actual icon files.
fn check_theme_has_icons(path: &Path) -> bool {
    // Check scalable subdirs
    let scalable = path.join("scalable");
    if scalable.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&scalable) {
            for entry in entries.flatten() {
                let sub = entry.path();
                if sub.is_dir() {
                    if let Ok(sub_entries) = std::fs::read_dir(&sub) {
                        for se in sub_entries.flatten() {
                            if se.path().extension().and_then(|e| e.to_str()) == Some("svg") {
                                return true;
                            }
                        }
                    }
                }
            }
        }
    }

    // Check size dirs (16x16, 24x24, etc.)
    for size in &["16x16", "24x24", "32x32", "48x48", "64x64", "128x128", "256x256"] {
        let size_dir = path.join(size);
        if size_dir.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&size_dir) {
                for entry in entries.flatten() {
                    if let Some(ext) = entry.path().extension().and_then(|e| e.to_str()) {
                        if ext == "svg" || ext == "png" || ext == "jpg" {
                            return true;
                        }
                    }
                }
            }
        }
    }

    false
}

/// Find the currently active system icon theme via gsettings.
fn get_active_icon_theme() -> Option<String> {
    let output = Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "icon-theme"])
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Some(stdout.trim_matches('\'').to_string())
}

/// Pick the best available icon theme to use as the source for recoloring.
/// Noctalia is preferred if installed; otherwise falls back to the
/// currently-active gsettings theme, then Adwaita.
/// Skips any recolored output themes (those in ~/.local/share/icons that
/// we may have created previously).
pub(crate) fn find_best_base_theme() -> String {
    // Check system Adwaita first — it has the full scalable/ icons
    if find_icon_theme_root_at("/usr/share/icons/Adwaita").is_some() {
        return "Adwaita".to_string();
    }
    if let Some(active) = get_active_icon_theme() {
        // Skip if the active theme IS a recolored output (not a real source)
        if !is_recolored_output(&active)
            && find_icon_theme_root(&active).is_some()
        {
            return active;
        }
    }
    if find_icon_theme_root("Noctalia").is_some() {
        return "Noctalia".to_string();
    }
    // Always fall back to Adwaita (covers Adwaita-dark as a variant)
    "Adwaita".to_string()
}

/// Check if a specific path has a valid icon theme root (not just by name).
fn find_icon_theme_root_at(path: &str) -> Option<PathBuf> {
    let p = PathBuf::from(path);
    if p.is_dir()
        && (p.join("index.theme").exists()
            || p.join("16x16").is_dir()
            || p.join("scalable").is_dir())
    {
        Some(p)
    } else {
        None
    }
}

/// Check if a theme name looks like a recolored output (e.g. Purple_Haze,
/// Eldritch, custom theme names — not system themes like Adwaita).
fn is_recolored_output(name: &str) -> bool {
    let skip = [
        "Adwaita", "Adwaita-dark", "Adwaita-light",
        "Noctalia", "hicolor", "Humanity", "gnome", "oxygen",
        "Papirus", "Papirus-Dark",
    ];
    !skip.contains(&name)
        && (name.contains('_')
            || name == "Custom"
            || name == "Eldritch"
            || name == "Purple Haze"
            || name == "Lilac AMOLED"
            || name == "Murasaki"
            || name == "Oxocarbon")
}

// --------------------------------------------------------------------------
// Output directory
// --------------------------------------------------------------------------

/// Return the output icon theme directory for a given theme name.
fn icon_theme_dir_for(name: &str) -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("/home"))
        .join("icons")
        .join(name)
}

// --------------------------------------------------------------------------
// Hashing
// --------------------------------------------------------------------------

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

// --------------------------------------------------------------------------
// Reuse wallpaper recolor
// --------------------------------------------------------------------------

pub fn recolor_image(
    input: &image::RgbImage,
    theme_data: &NoctaliaTheme,
    verbose: bool,
) -> image::RgbImage {
    crate::utils::recolor::recolor_wallpaper(input, theme_data, verbose)
}

// --------------------------------------------------------------------------
// Icon recoloring
// --------------------------------------------------------------------------

/// Recolor all icons from the base theme and write to the per-theme output dir.
pub fn recolor_icons(theme_name: &str, verbose: bool) -> Result<String, String> {
    let (_, theme_content) = theme::read_theme()?;
    let theme_data = parse_theme(&theme_content)
        .ok_or_else(|| "failed to parse theme from colors.json".to_string())?;
    let hash = theme_hash_for_icons(&theme_data);
    let base_theme = find_best_base_theme();

    let base_root = find_icon_theme_root(&base_theme)
        .ok_or_else(|| format!("icon theme '{}' not found", base_theme))?;

    let output_dir = icon_theme_dir_for(theme_name);
    std::fs::create_dir_all(&output_dir)
        .map_err(|e| format!("failed to create icon output dir: {}", e))?;

    if verbose {
        eprintln!("Base theme: {} ({})", base_theme, base_root.display());
        eprintln!("Output theme: {}", theme_name);
        eprintln!("Output dir: {}", output_dir.display());
        eprintln!("Theme hash: {}", hash);
        eprintln!("Theme palette:");
        eprintln!("  primary:      #{:02x}{:02x}{:02x}", theme_data.primary[0], theme_data.primary[1], theme_data.primary[2]);
        eprintln!("  on_primary:  #{:02x}{:02x}{:02x}", theme_data.on_primary[0], theme_data.on_primary[1], theme_data.on_primary[2]);
        eprintln!("  surface:      #{:02x}{:02x}{:02x}", theme_data.surface[0], theme_data.surface[1], theme_data.surface[2]);
        eprintln!("  on_surface:  #{:02x}{:02x}{:02x}", theme_data.on_surface[0], theme_data.on_surface[1], theme_data.on_surface[2]);
        eprintln!("  surface_var: #{:02x}{:02x}{:02x}", theme_data.surface_variant[0], theme_data.surface_variant[1], theme_data.surface_variant[2]);
        eprintln!("  on_surface_v:#{:02x}{:02x}{:02x}", theme_data.on_surface_variant[0], theme_data.on_surface_variant[1], theme_data.on_surface_variant[2]);
        eprintln!("  error:       #{:02x}{:02x}{:02x}", theme_data.error[0], theme_data.error[1], theme_data.error[2]);
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
                recolor_svg_icon(&path, &theme_data, &output_dir, verbose).map(|_| ())
            } else if matches!(ext, "png" | "jpg" | "jpeg" | "webp" | "bmp") {
                recolor_raster_icon(&path, &theme_data, &output_dir).map(|_| ())
            } else {
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
                match recolor_svg_icon(&path, &theme_data, &output_dir, verbose) {
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

    generate_index_theme(&output_dir, theme_name, &base_theme)?;

    if verbose {
        eprintln!("Recolored {} icons ({} errors)", total, errors);
    }

    Ok(hash)
}

// ------------------------------------------------------------------------------------------------------------------------------------------
// Generate index.theme
// ------------------------------------------------------------------------------------------------------------------------------------------

fn generate_index_theme(output_dir: &Path, theme_name: &str, base_name: &str) -> Result<(), String> {
    let index_content = format!(
        "[Icon Theme]\n\
         Name={}\n\
         Comment=Recolored by AKSprayPaint from {}\n\
         DisplayName={}\n\
         Inherits=hicolor\n\
         Example=folder\n\
         FollowsNav=True\n\
         \n\
         [Directories]\n\
         scalable/actions=svg\n\
         scalable/apps=svg\n\
         scalable/categories=svg\n\
         scalable/devices=svg\n\
         scalable/emblems=svg\n\
         scalable/mimetypes=svg\n\
         scalable/places=svg\n\
         scalable/status=svg\n",
        theme_name,
        base_name,
        theme_name.replace('_', " ")
    );

    std::fs::write(output_dir.join("index.theme"), index_content)
        .map_err(|e| format!("failed to write index.theme: {}", e))?;
    Ok(())
}

// ------------------------------------------------------------------------------------------------------------------------------------------
// SVG color extraction & recoloring
// ------------------------------------------------------------------------------------------------------------------------------------------

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
                            if !s.is_empty() && s != "none" && s != "transparent" && s != "currentColor" {
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

// ------------------------------------------------------------------------------------------------------------------------------------------
// OKLCH color math
// ------------------------------------------------------------------------------------------------------------------------------------------

use palette::{FromColor, IntoColor, OklabHue, Oklch, Srgb};

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

// ------------------------------------------------------------------------------------------------------------------------------------------
// SVG recoloring
// ------------------------------------------------------------------------------------------------------------------------------------------

fn recolor_svg_icon(
    src: &Path,
    theme_data: &NoctaliaTheme,
    output_dir: &Path,
    verbose: bool,
) -> Result<PathBuf, String> {
    let svg_bytes = std::fs::read(src).map_err(|e| format!("failed to read SVG: {}", e))?;

    let palette = extract_svg_palette(&svg_bytes)?;
    if palette.is_empty() {
        return copy_icon_as_is(src, output_dir);
    }

    let mappings = build_svg_anchor_mappings(&palette, theme_data);
    if mappings.is_empty() {
        return copy_icon_as_is(src, output_dir);
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
    let out_dir = output_dir.join(size_dir);
    std::fs::create_dir_all(&out_dir)
        .map_err(|e| format!("failed to create dir: {}", e))?;
    let out_path = out_dir.join(name);
    std::fs::write(&out_path, recolored)
        .map_err(|e| format!("failed to write: {}", e))?;

    Ok(out_path)
}

fn copy_icon_as_is(src: &Path, output_dir: &Path) -> Result<PathBuf, String> {
    let name = src.file_name().unwrap();
    let size_dir = src
        .parent()
        .and_then(|p| p.file_name())
        .unwrap_or(".".as_ref());
    let out_dir = output_dir.join(size_dir);
    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
    let out_path = out_dir.join(name);
    std::fs::copy(src, &out_path).map_err(|e| e.to_string())?;
    Ok(out_path)
}

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
                writer
                    .write_event(Event::Eof)
                    .map_err(|e| format!("SVG write error: {}", e))?;
                break;
            }
            Ok(e) => {
                writer.write_event(e).map_err(|e| format!("SVG write error: {}", e))?;
            }
            Err(e) => return Err(format!("SVG read error: {}", e)),
        }
    }

    Ok(writer.into_inner().into_inner())
}

fn rewrite_element_attrs(
    elem: BytesStart<'_>,
    mappings: &std::collections::HashMap<String, String>,
) -> Result<BytesStart<'static>, String> {
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

    let elem_name_str = String::from_utf8_lossy(&elem_name_bytes);
    let mut result = BytesStart::new(elem_name_str.into_owned());
    for (key, value) in new_attrs {
        result.push_attribute((key.as_str(), value.as_str()));
    }

    Ok(result)
}

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

// ------------------------------------------------------------------------------------------------------------------------------------------
// Raster icon recoloring
// ------------------------------------------------------------------------------------------------------------------------------------------

fn recolor_raster_icon(
    src: &Path,
    theme_data: &NoctaliaTheme,
    output_dir: &Path,
) -> Result<PathBuf, String> {
    let img = image::open(src).map_err(|e| format!("failed to open image: {}", e))?;
    let rgb_img = img.to_rgb8();
    let recolored = recolor_image(&rgb_img, theme_data, false);

    let name = src.file_name().unwrap();
    let size_dir = src
        .parent()
        .and_then(|p| p.file_name())
        .unwrap_or(".".as_ref());
    let out_path = output_dir.join(size_dir).join(name);

    recolored
        .save(&out_path)
        .map_err(|e| format!("failed to save icon: {}", e))?;
    Ok(out_path)
}

// ------------------------------------------------------------------------------------------------------------------------------------------
// Apply theme
// ------------------------------------------------------------------------------------------------------------------------------------------

/// Apply the named icon theme via gsettings, then rebuild the GTK icon cache.
pub fn apply_icon_theme(theme_name: &str) -> Result<(), String> {
    let output = Command::new("gsettings")
        .args(["set", "org.gnome.desktop.interface", "icon-theme", theme_name])
        .output()
        .map_err(|e| format!("failed to run gsettings: {}", e))?;
    if !output.status.success() {
        return Err(format!(
            "gsettings failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let output_dir = icon_theme_dir_for(theme_name);
    let cache_output = Command::new("gtk-update-icon-cache")
        .args(["--force", &output_dir.to_string_lossy()])
        .output();

    if let Err(e) = cache_output {
        eprintln!(
            "warning: gtk-update-icon-cache failed: {} (non-fatal)",
            e
        );
    } else if !cache_output.unwrap().status.success() {
        eprintln!(
            "warning: gtk-update-icon-cache returned non-zero (non-fatal)"
        );
    }

    // Restart Nemo to force icon refresh (Nemo caches icons in-memory)
    let nemo_quit = Command::new("nemo")
        .args(["--quit"])
        .output();
    if nemo_quit.as_ref().map(|o| o.status.success()).unwrap_or(false) {
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_millis(300));
            let _ = Command::new("nemo").spawn();
        });
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

    #[test]
    fn test_extract_toml_string() {
        let content = "builtin = \"Eldritch\"
custom_palette = \"Purple Haze\"
source = \"custom\"";
        assert_eq!(extract_toml_string(content, "builtin"), Some("Eldritch".to_string()));
        assert_eq!(extract_toml_string(content, "custom_palette"), Some("Purple Haze".to_string()));
        assert_eq!(extract_toml_string(content, "source"), Some("custom".to_string()));
    }
}
