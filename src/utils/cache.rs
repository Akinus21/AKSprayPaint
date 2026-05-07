use std::path::{Path, PathBuf};

fn cache_root() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("akspraypaint")
}

pub fn cache_dir_for_hash(theme_hash: &str) -> PathBuf {
    cache_root().join(theme_hash)
}

pub fn cached_path(theme_hash: &str, wallpaper_path: &Path) -> PathBuf {
    let name = wallpaper_path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy();
    let ext = wallpaper_path
        .extension()
        .unwrap_or_default()
        .to_string_lossy();
    cache_dir_for_hash(theme_hash).join(format!("{}.{}", name, ext))
}

pub fn find_cached(theme_hash: &str, wallpaper_path: &Path) -> Option<PathBuf> {
    let path = cached_path(theme_hash, wallpaper_path);
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

pub fn save_to_cache(
    data: &[u8],
    theme_hash: &str,
    wallpaper_path: &Path,
) -> Result<PathBuf, String> {
    let dir = cache_dir_for_hash(theme_hash);
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("failed to create cache dir: {}", e))?;
    let dest = cached_path(theme_hash, wallpaper_path);
    std::fs::write(&dest, data).map_err(|e| format!("failed to write cache: {}", e))?;
    Ok(dest)
}

pub fn clean_cache() -> Result<usize, String> {
    let root = cache_root();
    if !root.exists() {
        return Ok(0);
    }
    let mut count = 0;
    let entries = std::fs::read_dir(&root)
        .map_err(|e| format!("failed to read cache dir: {}", e))?;
    for entry in entries {
        if let Ok(entry) = entry {
            if entry.path().is_dir() {
                if std::fs::remove_dir_all(entry.path()).is_ok() {
                    count += 1;
                }
            }
        }
    }
    Ok(count)
}
