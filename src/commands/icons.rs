use crate::utils::icons::find_best_base_theme;
use crate::utils::{icons, theme};
use inotify::{EventMask, Inotify, WatchMask};
use std::time::{Duration, Instant};

const DEBOUNCE_MS: u64 = 500;

pub fn recolor(verbose: bool) -> Result<(), String> {
    let theme_name = icons::get_current_theme_name()?;
    eprintln!("Icon theme: {}", theme_name);

    let hash = icons::recolor_icons(&theme_name, verbose)?;
    if let Err(e) = icons::apply_icon_theme(&theme_name) {
        eprintln!("warning: icon theme apply failed (non-fatal): {}", e);
    }

    eprintln!(
        "Icon theme '{}' applied ({} icons cached)",
        theme_name, hash
    );
    Ok(())
}

/// Returns true if the file name indicates a theme-name or colors change that
/// should trigger an icon recolor.
///
/// We watch two directories:
///   - ~/.config/noctalia/  (noctalia_dir) → colors.json
///   - ~/.local/state/noctalia/ (noctalia_state_dir) → settings.toml
///
/// Both must trigger a recolor because:
///   - colors.json changes on any theme color tweak (but doesn't carry the theme name)
///   - settings.toml changes when the user switches between community themes
///     (e.g. Catppuccin → Dracula) even if the actual color values haven't changed
fn should_recolor_file(name: &str) -> bool {
    name == "colors.json" || name == "settings.toml"
}

pub fn watch() -> Result<(), String> {
    crate::utils::daemon::write_pid()?;

    let noctalia_dir =
        theme::noctalia_dir().ok_or_else(|| "noctalia config directory not found".to_string())?;

    // Also watch ~/.local/state/noctalia/ for settings.toml changes.
    // This is where Noctalia writes the active theme name (source + theme key)
    // on community theme switches — colors.json may not change at all when
    // switching between themes that share the same palette colors.
    let noctalia_state_dir = theme::noctalia_state_dir();

    let base_theme = find_best_base_theme();

    eprintln!("Icon theme watch started");
    eprintln!("Base theme: {}", base_theme);
    eprintln!("Watching: {}", noctalia_dir.display());
    if let Some(ref state_dir) = noctalia_state_dir {
        eprintln!("Watching state: {}", state_dir.display());
    }

    let mut inotify = Inotify::init().map_err(|e| format!("failed to init inotify: {}", e))?;

    inotify
        .watches()
        .add(
            &noctalia_dir,
            WatchMask::CLOSE_WRITE | WatchMask::MOVED_TO | WatchMask::DELETE,
        )
        .map_err(|e| format!("failed to watch directory: {}", e))?;

    // Watch the state directory for settings.toml changes (community theme switches)
    if let Some(ref state_dir) = noctalia_state_dir {
        inotify
            .watches()
            .add(
                state_dir,
                WatchMask::CLOSE_WRITE | WatchMask::MOVED_TO | WatchMask::DELETE,
            )
            .map_err(|e| format!("failed to watch state directory: {}", e))?;
    }

    let mut buffer = [0u8; 4096];
    let mut last_event = Instant::now().checked_sub(Duration::from_secs(60)).unwrap();

    loop {
        let events = inotify
            .read_events_blocking(&mut buffer)
            .map_err(|e| format!("inotify read error: {}", e))?;

        let mut recolor_triggered = false;
        for event in events {
            if event.mask.contains(EventMask::ISDIR) {
                continue;
            }
            if let Some(name) = event.name {
                let name_str = name.to_string_lossy();
                if should_recolor_file(&name_str) {
                    recolor_triggered = true;
                }
            }
        }

        if recolor_triggered {
            let now = Instant::now();
            if now.duration_since(last_event).as_millis() < DEBOUNCE_MS as u128 {
                continue;
            }
            last_event = now;

            std::thread::sleep(Duration::from_millis(DEBOUNCE_MS));
            eprintln!("Theme or config change detected, recoloring icons...");

            match icons::get_current_theme_name() {
                Ok(theme_name) => match icons::recolor_icons(&theme_name, false) {
                    Ok(hash) => {
                        if let Err(e) = icons::apply_icon_theme(&theme_name) {
                            eprintln!("warning: icon theme apply failed: {}", e);
                        } else {
                            eprintln!("Icon theme '{}' applied ({})", theme_name, hash);
                        }
                    }
                    Err(e) => eprintln!("Error recoloring icons: {}", e),
                },
                Err(e) => eprintln!("Error getting theme name: {}", e),
            }
        }
    }
}

pub fn clean() -> Result<(), String> {
    let icons_base = dirs::data_dir()
        .ok_or_else(|| "data directory not found".to_string())?
        .join("icons");

    let mut removed = 0usize;
    if !icons_base.is_dir() {
        eprintln!("No icon theme directory found");
        return Ok(());
    }

    let entries = std::fs::read_dir(&icons_base).map_err(|e| e.to_string())?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        // Remove any theme directory that isn't a standard system theme
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
                eprintln!("Removed: {}", path.display());
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
