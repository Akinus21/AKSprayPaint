use crate::utils::{cache, recolor, theme, wallpaper};

pub fn set(path: &str) -> Result<(), String> {
    let wp_path = std::path::Path::new(path);
    if !wp_path.is_file() {
        return Err(format!("file not found: {}", path));
    }

    let (_, theme_content) = theme::read_theme()?;
    let hash = theme::theme_hash(&theme_content);

    if let Some(cached) = cache::find_cached(&hash, wp_path) {
        eprintln!("Using cached recolored version: {}", cached.display());
        return wallpaper::set_wallpaper(&cached);
    }

    eprintln!("Recoloring image to match theme ({})...", hash);

    let img = image::open(wp_path).map_err(|e| format!("failed to load image: {}", e))?;
    let rgb_img = img.to_rgb8();

    let (theme_data, _) = theme::read_theme()?;
    let palette: Vec<[u8; 3]> = theme_data.palette();
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

    let dest = cache::save_to_cache(buf.get_ref(), &hash, wp_path)?;
    eprintln!("Saved recolored wallpaper: {}", dest.display());
    wallpaper::set_wallpaper(&dest)?;

    Ok(())
}

pub fn status() -> Result<(), String> {
    let wp = wallpaper::detect_wallpaper();
    let noctalia_cfg = theme::theme_config_path();
    let has_theme = theme::read_theme();

    println!("AKSprayPaint Status");
    println!("===================");
    match wp {
        Some(ref p) => println!("Current wallpaper: {}", p.display()),
        None => println!("Current wallpaper: unknown"),
    }
    match noctalia_cfg {
        Some(ref p) => println!("Noctalia config:    {}", p.display()),
        None => println!("Noctalia config:    not found"),
    }
    if let Ok((_, content)) = has_theme {
        let hash = theme::theme_hash(&content);
        println!("Theme hash:         {}", hash);
    }

    Ok(())
}

pub fn clean() -> Result<(), String> {
    let count = cache::clean_cache()?;
    println!("Removed {} cached theme directories", count);
    Ok(())
}
