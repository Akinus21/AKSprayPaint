//! Daemon main loop.

use std::collections::HashMap;
use std::process::Command;
use std::time::{Duration, Instant};

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
    // Small delay to let quit take effect before relaunching
    std::thread::sleep(Duration::from_millis(200));
    if let Err(e) = Command::new(name).arg("--browser").spawn() {
        eprintln!("[gtkd] warning: failed to restart {}: {}", name, e);
    }
}

/// Main daemon run loop.
pub fn run(config: GtkBridgeConfig) -> Result<(), String> {
    let poll_interval = Duration::from_millis(config.poll.interval_ms);
    let grace_period = Duration::from_millis(config.poll.launch_grace_ms);
    // How long to wait after restarting before re-injecting into the same PID.
    // Prevents infinite inject→restart→detect→re-inject loops.
    let restart_cooldown = Duration::from_secs(10);

    let mut theme_state = ThemeState::detect();
    let mut injected_pids: std::collections::HashSet<u32> = std::collections::HashSet::new();
    // PIDs that were recently restarted, along with when the cooldown expires.
    let mut recently_restarted: HashMap<u32, Instant> = HashMap::new();
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
        let now = Instant::now();

        // Prune expired cooldown entries
        recently_restarted.retain(|_pid, expires_at| now < *expires_at);

        // Skip PIDs in cooldown — they were recently restarted and we're
        // waiting for them to re-launch and finish initializing.
        let running: Vec<(u32, String)> = scan_running(&config.watch.apps)
            .into_iter()
            .filter(|(pid, _comm)| !recently_restarted.contains_key(pid))
            .collect();

        // Detect theme changes
        let new_theme = ThemeState::detect();
        if has_changed(&new_theme, &theme_state) {
            eprintln!(
                "[gtkd] theme changed: {:?} -> {:?}",
                theme_state.icon_theme, new_theme.icon_theme
            );
            theme_state = new_theme.clone();

            // Re-apply to all currently-running watched apps
            for (pid, comm) in &running {
                let settings = build_settings(&config, &theme_state);
                eprintln!("[gtkd] re-injecting into {} (PID {})", comm, pid);
                inject_async(*pid, settings);
                if config.watch.restart_after_inject {
                    restart_app(comm);
                    // Mark as recently restarted to avoid re-triggering
                    recently_restarted.insert(*pid, now + restart_cooldown);
                }
            }
            // Also mark any pre-existing injected PIDs that weren't in `running`
            // (they may still be restarting in the background)
            if config.watch.restart_after_inject {
                for pid in &injected_pids {
                    recently_restarted.insert(*pid, now + restart_cooldown);
                }
            }
            continue;
        }

        // Scan for brand-new watched app launches (not recently restarted)
        for (pid, comm) in &running {
            if !injected_pids.contains(pid) {
                let pid = *pid;
                let comm = comm.clone();
                let settings = build_settings(&config, &theme_state);
                let grace = grace_period;
                let restart = config.watch.restart_after_inject;

                eprintln!(
                    "[gtkd] detected new {} (PID {}), scheduling injection in {}ms...",
                    comm,
                    pid,
                    grace.as_millis()
                );

                // Inject after grace period, then optionally restart
                std::thread::spawn(move || {
                    std::thread::sleep(grace);
                    eprintln!("[gtkd] injecting into {} (PID {})", comm, pid);
                    inject_async(pid, settings);
                    if restart {
                        restart_app(&comm);
                    }
                });

                injected_pids.insert(pid);
                watched_apps.mark_injected(pid);
                if restart {
                    recently_restarted.insert(pid, now + restart_cooldown);
                }
            }
        }

        // Prune dead PIDs from injected set
        watched_apps.prune_dead();
        injected_pids.retain(|&pid| std::path::Path::new(&format!("/proc/{}", pid)).exists());
    }
}
