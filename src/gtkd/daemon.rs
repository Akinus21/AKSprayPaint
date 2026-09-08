//! Daemon main loop.

use std::collections::HashSet;
use std::time::Duration;

use crate::gtkd::config::GtkBridgeConfig;
use crate::gtkd::inject::{build_settings, inject_async};
use crate::gtkd::theme::{has_changed, ThemeState};
use crate::gtkd::watch::{scan_running, WatchedApps};

/// Main daemon run loop.
pub fn run(config: GtkBridgeConfig) -> Result<(), String> {
    let poll_interval = Duration::from_millis(config.poll.interval_ms);
    let grace_period = Duration::from_millis(config.poll.launch_grace_ms);

    let mut theme_state = ThemeState::detect();
    let mut injected_pids: HashSet<u32> = HashSet::new();
    let mut watched_apps = WatchedApps::default();

    eprintln!(
        "[gtkd] started, watching for: {:?}",
        config.watch.apps
    );
    eprintln!("[gtkd] initial theme: {:?}", theme_state.icon_theme);

    // Inject into any already-running watched apps
    let running = scan_running(&config.watch.apps);
    for (pid, comm) in &running {
        eprintln!(
            "[gtkd] found already-running {} (PID {}), injecting...",
            comm, pid
        );
        let settings = build_settings(&config, &theme_state);
        inject_async(*pid, settings);
        injected_pids.insert(*pid);
        watched_apps.mark_injected(*pid);
    }

    loop {
        std::thread::sleep(poll_interval);

        let running = scan_running(&config.watch.apps);

        // Theme change: re-inject into all running instances
        let new_theme = ThemeState::detect();
        if has_changed(&new_theme, &theme_state) {
            eprintln!(
                "[gtkd] theme changed: {:?} -> {:?}",
                theme_state.icon_theme, new_theme.icon_theme
            );

            // Apply niri border colors
            if let Some(ref theme) = new_theme.noctalia_theme {
                if let Err(e) = crate::gtkd::niri::write_border_config(theme) {
                    eprintln!("[gtkd] niri border config failed: {}", e);
                } else {
                    crate::gtkd::niri::reload_niri_config();
                }
            }

            // Recolor icons if enabled — do this BEFORE updating theme_state
            // so we use the NEW theme name for the folder, not the old one
            if config.settings.icons {
                eprintln!("[gtkd] recoloring icons for new theme...");
                let recolor_out = std::process::Command::new("akspraypaint")
                    .args(["icons", "recolor"])
                    .output();
                match recolor_out {
                    Ok(out) if out.status.success() => {
                        eprintln!("[gtkd] icon recolor done");
                    }
                    Ok(out) => {
                        eprintln!(
                            "[gtkd] icon recolor failed ({}): {}",
                            out.status,
                            String::from_utf8_lossy(&out.stderr)
                        );
                    }
                    Err(e) => {
                        eprintln!("[gtkd] icon recolor error: {}", e);
                    }
                }
            }

            // Re-detect theme AFTER recolor so icon_theme matches the folder we just created
            theme_state = ThemeState::detect();

            for (pid, comm) in &running {
                let settings = build_settings(&config, &theme_state);
                eprintln!("[gtkd] re-injecting into {} (PID {})", comm, pid);
                inject_async(*pid, settings);
            }
            continue;
        }

        // New nemo launch
        for (pid, comm) in &running {
            if injected_pids.contains(pid) {
                continue;
            }

            // Mark injected BEFORE spawning so we don't double-schedule the same PID
            let pid_val = *pid;
            injected_pids.insert(pid_val);
            let comm_val = comm.clone();
            let settings = build_settings(&config, &theme_state);
            let grace = grace_period;

            eprintln!(
                "[gtkd] detected new {} (PID {}), scheduling injection in {}ms...",
                comm_val,
                pid_val,
                grace.as_millis()
            );

            std::thread::spawn(move || {
                std::thread::sleep(grace);
                eprintln!("[gtkd] injecting into {} (PID {})", comm_val, pid_val);
                inject_async(pid_val, settings);
            });

            watched_apps.mark_injected(pid_val);
        }

        // Prune dead PIDs
        watched_apps.prune_dead();
        injected_pids.retain(|&pid| std::path::Path::new(&format!("/proc/{}", pid)).exists());
    }
}
