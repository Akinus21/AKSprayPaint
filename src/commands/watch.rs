use crate::utils::{daemon, theme, wallpaper};
use inotify::{EventMask, Inotify, WatchMask};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const DEBOUNCE_MS: u64 = 500;

pub fn watch() -> Result<(), String> {
    daemon::write_pid()?;

    let noctalia_dir = theme::noctalia_dir()
        .ok_or_else(|| "noctalia config directory not found".to_string())?;

    let wp_path = wallpaper::detect_wallpaper()
        .ok_or_else(|| "could not detect current wallpaper".to_string())?;
    eprintln!("Watching theme: {}", noctalia_dir.display());
    eprintln!("Current wallpaper: {}", wp_path.display());
    eprintln!("Daemon started with PID {}", std::process::id());

    let mut inotify =
        Inotify::init().map_err(|e| format!("failed to init inotify: {}", e))?;
    inotify
        .watches()
        .add(
            &noctalia_dir,
            WatchMask::CLOSE_WRITE
                | WatchMask::MOVED_TO
                | WatchMask::MOVED_FROM
                | WatchMask::DELETE,
        )
        .map_err(|e| format!("failed to watch directory: {}", e))?;

    let mut buffer = [0u8; 4096];
    let mut last_event = Instant::now()
        .checked_sub(Duration::from_secs(60))
        .unwrap();

    loop {
        let events = inotify
            .read_events_blocking(&mut buffer)
            .map_err(|e| format!("inotify read error: {}", e))?;

        let mut colors_changed = false;
        for event in events {
            if event.mask.contains(EventMask::ISDIR) {
                continue;
            }
            if let Some(name) = event.name {
                if name == "colors.json" {
                    colors_changed = true;
                }
            }
        }

        if colors_changed {
            let now = Instant::now();
            if now.duration_since(last_event).as_millis() < DEBOUNCE_MS as u128 {
                continue;
            }
            last_event = now;

            std::thread::sleep(Duration::from_millis(DEBOUNCE_MS));
            eprintln!("Theme change detected, applying...");
            if let Err(e) = apply_theme(&wp_path) {
                eprintln!("Error applying theme: {}", e);
            }
        }
    }
}

fn apply_theme(wp_path: &PathBuf) -> Result<(), String> {
    let (_, theme_content) = theme::read_theme()?;
    let hash = theme::theme_hash(&theme_content);

    if let Some(cached) = crate::utils::cache::find_cached(&hash, wp_path) {
        eprintln!("Using cached version: {}", cached.display());
        wallpaper::set_wallpaper(&cached)
    } else {
        eprintln!("Recoloring wallpaper to match theme ({})...", hash);
        crate::commands::run::apply_recolor(wp_path, &hash)
    }
}
