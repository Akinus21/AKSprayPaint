use std::path::{Path, PathBuf};
use std::process::Command;

pub fn detect_wallpaper() -> Option<PathBuf> {
    if let Some(path) = try_swww() {
        return Some(path);
    }
    if let Some(path) = try_hyprpaper() {
        return Some(path);
    }
    if let Some(path) = try_swaybg() {
        return Some(path);
    }
    if let Some(path) = try_gsettings() {
        return Some(path);
    }
    if let Some(path) = try_noctalia_cache() {
        return Some(path);
    }
    None
}

fn try_hyprpaper() -> Option<PathBuf> {
    let output = Command::new("hyprctl")
        .args(["hyprpaper", "listactive"])
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if line.contains(", ") {
            let parts: Vec<&str> = line.split(", ").collect();
            if parts.len() >= 2 {
                let path = PathBuf::from(parts[1].trim());
                if path.is_file() {
                    return Some(path);
                }
            }
        }
    }
    None
}

fn try_noctalia_cache() -> Option<PathBuf> {
    // Try XDG config directory for noctalia config files
    if let Some(config_dir) = dirs::config_dir() {
        // Check for wallpaper path in noctalia config
        let noctalia_config = config_dir.join("noctalia").join("config.toml");
        if let Ok(content) = std::fs::read_to_string(&noctalia_config) {
            for line in content.lines() {
                if line.starts_with("wallpaper") {
                    if let Some(eq_pos) = line.find('=') {
                        let path_str = line[eq_pos + 1..].trim().trim_matches('"').trim_matches('\'');
                        let path = PathBuf::from(path_str);
                        if path.is_file() {
                            return Some(path);
                        }
                    }
                }
            }
        }
    }
    
    // Try XDG data directories
    if let Some(data_dir) = dirs::data_dir() {
        let paths = [
            data_dir.join("noctalia").join("wallpaper"),
            data_dir.join("noctalia-shell").join("wallpaper"),
            data_dir.join("niri").join("wallpaper"),
        ];
        for path in paths {
            if path.is_file() {
                return Some(path);
            }
        }
    }
    
    // Try common noctalia cache locations
    if let Some(cache_dir) = dirs::cache_dir() {
        let paths = [
            cache_dir.join("noctalia").join("wallpaper"),
            cache_dir.join("noctalia-shell").join("current_wallpaper"),
        ];
        for path in paths {
            if path.is_file() {
                return Some(path);
            }
        }
    }
    
    // Try XDG state directory
    if let Some(state_dir) = dirs::state_dir() {
        let paths = [
            state_dir.join("noctalia").join("wallpaper"),
            state_dir.join("noctalia-shell").join("wallpaper"),
        ];
        for path in paths {
            if path.is_file() {
                return Some(path);
            }
        }
    }
    
    None
}

fn try_swaybg() -> Option<PathBuf> {
    let output = Command::new("pgrep")
        .args(["-a", "swaybg"])
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let args: Vec<&str> = line.split_whitespace().collect();
        for i in 0..args.len() {
            if args[i] == "-i" {
                if let Some(path) = args.get(i + 1) {
                    let p = PathBuf::from(path);
                    if p.exists() {
                        return Some(p);
                    }
                }
            }
        }
    }
    None
}

fn try_swww() -> Option<PathBuf> {
    let output = Command::new("swww").arg("query").output().ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines().rev() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 {
            let path = PathBuf::from(parts.last()?);
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

fn try_gsettings() -> Option<PathBuf> {
    let output = Command::new("gsettings")
        .args([
            "get",
            "org.gnome.desktop.background",
            "picture-uri-dark",
        ])
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if let Some(path) = stdout.strip_prefix("file://") {
        let decoded = urlencoding(path);
        let p = PathBuf::from(&decoded);
        if p.exists() {
            return Some(p);
        }
    }

    let output = Command::new("gsettings")
        .args(["get", "org.gnome.desktop.background", "picture-uri"])
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if let Some(path) = stdout.strip_prefix("file://") {
        let decoded = urlencoding(path);
        let p = PathBuf::from(&decoded);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

fn urlencoding(raw: &str) -> String {
    let without_quotes = raw.trim_matches('\'');
    let mut result = String::new();
    let chars: Vec<char> = without_quotes.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '%' && i + 2 < chars.len() {
            if let Ok(byte) =
                u8::from_str_radix(&format!("{}{}", chars[i + 1], chars[i + 2]), 16)
            {
                result.push(byte as char);
                i += 3;
                continue;
            }
        }
        result.push(chars[i]);
        i += 1;
    }
    result
}

pub fn set_wallpaper(path: &Path) -> Result<(), String> {
    if try_set_swww(path) {
        return Ok(());
    }
    if try_set_swaybg(path) {
        return Ok(());
    }
    try_set_feh(path)
}

fn try_set_swww(path: &Path) -> bool {
    Command::new("swww")
        .args([
            "img",
            &path.to_string_lossy(),
            "--transition-type",
            "any",
            "--transition-duration",
            "1",
        ])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn try_set_swaybg(path: &Path) -> bool {
    Command::new("killall")
        .args(["-q", "swaybg"])
        .output()
        .ok();
    std::thread::sleep(std::time::Duration::from_millis(100));
    let output = Command::new("swaybg")
        .args(["-i", &path.to_string_lossy(), "-m", "fill"])
        .spawn()
        .is_ok();
    output
}

fn try_set_feh(path: &Path) -> Result<(), String> {
    let status = Command::new("feh")
        .args(["--bg-fill", &path.to_string_lossy()])
        .status()
        .map_err(|e| format!("failed to run feh: {}", e))?;
    if status.success() {
        Ok(())
    } else {
        Err("feh returned non-zero exit code".into())
    }
}
