use sha2::{Digest, Sha256};
use std::path::PathBuf;

use akspraypaint::{parse_theme, NoctaliaTheme};

fn v4_theme_path() -> Option<PathBuf> {
    let config = dirs::config_dir()?;
    let path = config.join("noctalia").join("colors.json");
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

pub fn v5_state_dir() -> Option<PathBuf> {
    let state_dir = std::env::var("NOCTALIA_STATE_HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(|| {
            // dirs::state_dir() resolves $XDG_STATE_HOME (~/.local/state
            // on Linux) — NOT dirs::data_local_dir(), which resolves
            // $XDG_DATA_HOME (~/.local/share) instead. Using the wrong
            // one here silently pointed at the wrong directory whenever
            // NOCTALIA_STATE_HOME wasn't explicitly set, which broke
            // both the file-watch (never added, since the wrong path
            // failed the is_dir() check below) and the theme-file reads.
            let state = dirs::state_dir()?;
            Some(state.join("noctalia"))
        })?;
    if state_dir.is_dir() {
        Some(state_dir)
    } else {
        None
    }
}

/// Asks the running Noctalia shell what's actually active right now,
/// via its own IPC — ground truth, straight from the source, rather
/// than independently re-parsing settings.toml and risking drift from
/// whatever Noctalia itself considers current.
///
/// `noctalia msg color-scheme-get` (no arguments) prints `<source>
/// <name>` for the currently active scheme, e.g. `custom Purple Haze`.
/// Names can contain spaces, so only the first token is the source —
/// everything after it, trimmed, is the name.
fn active_color_scheme() -> Option<(String, String)> {
    let output = std::process::Command::new("noctalia")
        .args(["msg", "color-scheme-get"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let text = text.trim();
    let mut parts = text.splitn(2, ' ');
    let source = parts.next()?.trim().to_string();
    let name = parts.next()?.trim().to_string();
    if source.is_empty() || name.is_empty() {
        return None;
    }
    Some((source, name))
}

fn v5_theme_path() -> Option<PathBuf> {
    let (source, name) = active_color_scheme()?;

    match source.as_str() {
        "custom" => {
            let config = dirs::config_dir()?;
            let path = config.join("noctalia").join("palettes").join(format!("{}.json", name));
            if path.exists() {
                Some(path)
            } else {
                None
            }
        }
        "community" => {
            // Observed filenames use literal %20 for spaces (e.g.
            // "Ayu%20Green.json"), not a real space character — matches
            // how the shell saves palettes downloaded by URL-encoded
            // name. Only spaces are handled here since that's the only
            // special character actually observed; if a palette name
            // ever contains something else unusual, this will need
            // broader encoding.
            let encoded_name = name.replace(' ', "%20");
            let path = v5_state_dir()?
                .join("community-palettes")
                .join(format!("{}.json", encoded_name));
            if path.exists() {
                Some(path)
            } else {
                None
            }
        }
        // "builtin" — no reliable source for these colors has been
        // found. The obvious candidates were checked and ruled out:
        // api.noctalia.dev/palette/<name> only serves community
        // palettes (confirmed: returns "Palette not found" for a
        // builtin name), and the old v4 Quickshell build's bundled
        // JSON assets live under an OSTree deployment-hash-specific
        // path that isn't guaranteed to stick around. Deliberately
        // unimplemented rather than pointing at something fragile or
        // wrong — see the source-aware error in read_theme() below.
        //
        // "wallpaper" (M3 extraction from the current wallpaper) also
        // has no static palette file to read — it's generated, not
        // stored — so it falls into the same catch-all.
        _ => None,
    }
}

pub fn theme_config_path() -> Option<PathBuf> {
    v5_theme_path().or_else(v4_theme_path)
}

pub fn read_theme() -> Result<(NoctaliaTheme, String), String> {
    let path = theme_config_path().ok_or_else(|| {
        match active_color_scheme() {
            Some((source, name)) if source == "builtin" => format!(
                "builtin color scheme '{}' is not yet supported — no reliable local source for \
                 builtin palette colors has been found (the community API only serves community \
                 palettes, and the old v4 bundled assets aren't a stable path to rely on). Switch \
                 to a custom or community palette for now, or track down where builtin colors \
                 actually live and this can be added.",
                name
            ),
            Some((source, name)) => format!(
                "could not find a palette file for source '{}', name '{}' — check it exists in \
                 the expected location for that source",
                source, name
            ),
            None => "noctalia colors.json not found at ~/.config/noctalia/colors.json, and \
                     `noctalia msg color-scheme-get` did not return a usable result"
                .to_string(),
        }
    })?;
    let content =
        std::fs::read_to_string(&path).map_err(|e| format!("failed to read theme: {}", e))?;
    let theme = parse_theme(&content)
        .ok_or_else(|| "failed to parse theme".to_string())?;
    Ok((theme, content))
}

pub fn theme_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hex::encode(&hasher.finalize()[..4])
}

pub fn noctalia_dir() -> Option<PathBuf> {
    let config = dirs::config_dir()?;
    let dir = config.join("noctalia");
    if dir.is_dir() {
        Some(dir)
    } else {
        None
    }
}
