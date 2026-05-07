use crate::utils::{cache, recolor, theme, wallpaper};

pub fn run() -> Result<(), String> {
    let wp_path = wallpaper::detect_wallpaper()
        .ok_or_else(|| "could not detect current wallpaper".to_string())?;
    eprintln!("Detected wallpaper: {}", wp_path.display());

    let (_, theme_content) = theme::read_theme()?;
    let hash = theme::theme_hash(&theme_content);

    if let Some(cached_path) = cache::find_cached(&hash, &wp_path) {
        eprintln!("Using cached recolored wallpaper: {}", cached_path.display());
        wallpaper::set_wallpaper(&cached_path)
    } else {
        eprintln!("Recoloring wallpaper to match theme ({})...", hash);
        apply_recolor(&wp_path, &hash)?;
        Ok(())
    }
}

pub fn apply_recolor(wp_path: &std::path::Path, hash: &str) -> Result<(), String> {
    let img = image::open(wp_path)
        .map_err(|e| format!("failed to load image: {}", e))?;
    let rgb_img = img.to_rgb8();

    let (theme_data, _) = theme::read_theme()?;
    let palette = theme_data.palette();

    let recolored = recolor::recolor_wallpaper(&rgb_img, &palette);

    let mut buf = std::io::Cursor::new(Vec::new());
    let ext = wp_path.extension().unwrap_or_default().to_string_lossy();
    match ext.as_ref() {
        "png" => recolored
            .write_to(&mut buf, image::ImageFormat::Png)
            .map_err(|e| format!("failed to encode png: {}", e))?,
        "jpg" | "jpeg" => recolored
            .write_to(&mut buf, image::ImageFormat::Jpeg)
            .map_err(|e| format!("failed to encode jpeg: {}", e))?,
        "webp" => recolored
            .write_to(&mut buf, image::ImageFormat::WebP)
            .map_err(|e| format!("failed to encode webp: {}", e))?,
        _ => recolored
            .write_to(&mut buf, image::ImageFormat::Png)
            .map_err(|e| format!("failed to encode image: {}", e))?,
    }

    let dest = cache::save_to_cache(buf.get_ref(), hash, wp_path)?;
    eprintln!("Saved recolored wallpaper: {}", dest.display());
    wallpaper::set_wallpaper(&dest)
}
