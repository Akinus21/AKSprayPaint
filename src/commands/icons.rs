use crate::utils::icons::find_best_base_theme;
use crate::utils::{icons, theme};
use inotify::{EventMask, Inotify, WatchMask};
use std::time::{Duration, Instant};

const DEBOUNCE_MS: u64 = 500;

pub fn recolor(verbose: bool) -> Result<(), String> {
    let base_theme = find_best_base_theme();
    eprintln!("Detected icon theme: {}", base_theme);

    let hash = icons::recolor_icons(&base_theme, verbose)?;

    icons::apply_icon_theme()?;
    eprintln!(
        "Icon theme '{}' applied ({} icons cached)",
        icons::ICON_THEME_NAME,
        hash
    );
    Ok(())
}

pub fn watch() -> Result<(), String> {
    crate::utils::daemon::write_pid()?;

    let noctalia_dir = theme::noctalia_dir()
        .ok_or_else(|| "noctalia config directory not found".to_string())?;

    let base_theme = icons::find_best_base_theme();

    eprintln!("Icon theme watch started");
    eprintln!("Base theme: {}", base_theme);
    eprintln!("Watching: {}", noctalia_dir.display());

    let mut inotify =
        Inotify::init().map_err(|e| format!("failed to init inotify: {}", e))?;

    inotify
        .watches()
        .add(
            &noctalia_dir,
            WatchMask::CLOSE_WRITE | WatchMask::MOVED_TO | WatchMask::DELETE,
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
                let name_str = name.to_string_lossy();
                if name_str == "colors.json" || name_str.ends_with(".json") {
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
            eprintln!("Theme change detected, recoloring icons...");
            match icons::recolor_icons(&base_theme, false) {
                Ok(hash) => {
                    if let Err(e) = icons::apply_icon_theme() {
                        eprintln!("Error applying icon theme: {}", e);
                    } else {
                        eprintln!("Icon theme '{}' applied ({})", icons::ICON_THEME_NAME, hash);
                    }
                }
                Err(e) => eprintln!("Error recoloring icons: {}", e),
            }
        }
    }
}

pub fn clean() -> Result<(), String> {
    let dir = icons::icon_theme_dir();
    if dir.exists() {
        std::fs::remove_dir_all(&dir)
            .map_err(|e| format!("failed to remove icon theme dir: {}", e))?;
        eprintln!("Removed: {}", dir.display());
    } else {
        eprintln!("No icon theme directory found");
    }
    Ok(())
}
