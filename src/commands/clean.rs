use crate::utils::cache;

/// Unified clean command.
/// target: None = all, Some("icons") = icons only, Some("wallpapers") = wallpapers only
pub fn clean(target: Option<&str>) -> Result<(), String> {
    match target {
        None | Some("all") => {
            clean_icons()?;
            clean_wallpapers()?;
        }
        Some("icons") => {
            clean_icons()?;
        }
        Some("wallpapers") => {
            clean_wallpapers()?;
        }
        Some(t) => {
            return Err(format!(
                "unknown clean target '{}': valid values are 'icons', 'wallpapers', or 'all'",
                t
            ));
        }
    }
    Ok(())
}

fn clean_icons() -> Result<(), String> {
    let icons_base = dirs::data_dir()
        .ok_or_else(|| "data directory not found".to_string())?
        .join("icons");

    if !icons_base.is_dir() {
        eprintln!("No icon theme directory found");
        return Ok(());
    }

    let entries = std::fs::read_dir(&icons_base).map_err(|e| e.to_string())?;
    let mut removed = 0usize;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        // Don't remove standard system themes
        if ![
            "Adwaita",
            "Adwaita-dark",
            "Noctalia",
            "hicolor",
            "Humanity",
            "gnome",
            "oxygen",
        ]
        .contains(&name)
        {
            if let Err(e) = std::fs::remove_dir_all(&path) {
                eprintln!("Failed to remove {}: {}", path.display(), e);
            } else {
                removed += 1;
                eprintln!("Removed icon theme: {}", path.display());
            }
        }
    }

    if removed == 0 {
        eprintln!("No recolored icon themes found");
    } else {
        eprintln!("Removed {} recolored icon theme(s)", removed);
    }
    Ok(())
}

fn clean_wallpapers() -> Result<(), String> {
    let count = cache::clean_cache()?;
    println!("Removed {} cached theme directories", count);
    Ok(())
}
