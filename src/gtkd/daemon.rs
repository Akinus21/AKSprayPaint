//! Daemon main loop.

use std::process::Command;
use std::time::Duration;

use crate::gtkd::config::GtkBridgeConfig;
use crate::gtkd::inject::{build_settings, inject_async};
use crate::gtkd::theme::{has_changed, ThemeState};
use crate::gtkd::watch::{scan_running, WatchedApps};

/// Restart a process by name (e.g. "nemo") after injection.
/// This ensures the app picks up new GTK settings at startup.
fn restart_app(name: &str) {
    eprintln!("[gtkd] restarting {}...", name);
    if let Err(e) = Command::new(name).arg("--quit").spawn() {
        eprintln!("[gtkd] warning: failed to quit {}: {}", name, e);
    }
    // Small delay to let quit take effect
    std::thread::sleep(Duration::from_millis(200));
    if let Err(e) = Command::new(name).arg("--browser").spawn() {
        eprintln!("[gtkd] warning: failed to restart {}: {}", name, e);
    }
}



/// Main daemon run loop.
pub fn run(config: GtkBridgeConfig) -> Result<(), String> {
    let poll_interval = Duration::from_millis(config.poll.interval_ms);
    let grace_period = Duration::from_millis(config.poll.launch_grace_ms);

    let mut theme_state = ThemeState::detect();
    let mut injected_pids: std::collections::HashSet<u32> = std::collections::HashSet::new();
    let mut watched_apps = WatchedApps::default();

    eprintln!(
        "[gtkd] started, watching for: {:?}",
        config.watch.apps
    );
    eprintln!("[gtkd] initial theme: {:?}", theme_state.icon_theme);

    // Inject into any already-running watched apps
    let running = scan_running(&config.watch.apps);
    for (pid, comm) in &running {
        eprintln!("[gtkd] found already-running {} (PID {}), injecting...", comm, pid);
        let settings = build_settings(&config, &theme_state);
        inject_async(*pid, settings);
        injected_pids.insert(*pid);
        watched_apps.mark_injected(*pid);
    }

    loop {
        std::thread::sleep(poll_interval);

        // Detect theme changes
        let new_theme = ThemeState::detect();
        if has_changed(&new_theme, &theme_state) {
            eprintln!("[gtkd] theme changed: {:?} -> {:?}", theme_state.icon_theme, new_theme.icon_theme);
            theme_state = new_theme.clone();

            // Re-apply to all currently-running watched apps
            let running = scan_running(&config.watch.apps);
            for (pid, comm) in &running {
                let settings = build_settings(&config, &theme_state);
                eprintln!("[gtkd] re-injecting into {} (PID {})", comm, pid);
                inject_async(*pid, settings);
                if config.watch.restart_after_inject {
                    restart_app(comm);
                }
            }
            continue;
        }

        // Scan for new watched app launches
        let running = scan_running(&config.watch.apps);
        for (pid, comm) in &running {
            if !injected_pids.contains(pid) {
                let pid = *pid;
                let comm = comm.clone();
                let settings = build_settings(&config, &theme_state);
                let grace = grace_period;

                eprintln!(
                    "[gtkd] detected new {} (PID {}), scheduling injection in {}ms...",
                    comm, pid, grace.as_millis()
                );

                // Inject after grace period, then restart so it picks up new settings
                std::thread::spawn(move || {
                    std::thread::sleep(grace);
                    eprintln!("[gtkd] injecting into {} (PID {})", comm, pid);
                    inject_async(pid, settings);
                    if config.watch.restart_after_inject {
                        restart_app(&comm);
                    }
                });

                injected_pids.insert(pid);
                watched_apps.mark_injected(pid);
            }
        }

        // Prune dead PIDs
        watched_apps.prune_dead();
        injected_pids.retain(|&pid| std::path::Path::new(&format!("/proc/{}", pid)).exists());
    }
}
