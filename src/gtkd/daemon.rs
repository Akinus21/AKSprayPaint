//! Daemon main loop.

use std::process::Command;
use std::time::{Duration, Instant};

use crate::gtkd::config::GtkBridgeConfig;
use crate::gtkd::inject::{build_settings, inject_async};
use crate::gtkd::theme::{has_changed, ThemeState};
use crate::gtkd::watch::{scan_running, WatchedApps};

/// Restart a process by name (e.g. "nemo") after injection.
fn restart_app(name: &str) {
    eprintln!("[gtkd] restarting {}...", name);
    if let Err(e) = Command::new(name).arg("--quit").spawn() {
        eprintln!("[gtkd] warning: failed to quit {}: {}", name, e);
    }
    std::thread::sleep(Duration::from_millis(200));
    if let Err(e) = Command::new(name).arg("--browser").spawn() {
        eprintln!("[gtkd] warning: failed to restart {}: {}", name, e);
    }
}

/// Main daemon run loop.
pub fn run(config: GtkBridgeConfig) -> Result<(), String> {
    let poll_interval = Duration::from_millis(config.poll.interval_ms);
    let grace_period = Duration::from_millis(config.poll.launch_grace_ms);
    // Global pause after any restart — prevents the inject→restart→detect→re-inject loop.
    let inject_pause = Duration::from_secs(5);

    let mut theme_state = ThemeState::detect();
    let mut injected_pids: std::collections::HashSet<u32> = std::collections::HashSet::new();
    let mut watched_apps = WatchedApps::default();
    // Moment when the global injection pause ends. None = not paused.
    let mut inject_pause_until: Option<Instant> = None;

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

        // Check if global injection pause is active
        let paused = inject_pause_until
            .map(|until| now < until)
            .unwrap_or(false);

        if paused {
            eprintln!(
                "[gtkd] injection paused (waiting for restart to settle)"
            );
        }

        // Scan for currently-running watched apps
        let running = scan_running(&config.watch.apps);

        // Always process theme changes even if paused
        let new_theme = ThemeState::detect();
        if has_changed(&new_theme, &theme_state) {
            eprintln!(
                "[gtkd] theme changed: {:?} -> {:?}",
                theme_state.icon_theme, new_theme.icon_theme
            );
            theme_state = new_theme.clone();

            for (pid, comm) in &running {
                let settings = build_settings(&config, &theme_state);
                eprintln!("[gtkd] re-injecting into {} (PID {})", comm, pid);
                inject_async(*pid, settings);
                if config.watch.restart_after_inject && !paused {
                    restart_app(comm);
                    inject_pause_until = Some(now + inject_pause);
                }
            }
            continue;
        }

        // Scan for brand-new watched app launches (only if not paused)
        if !paused {
            for (pid, comm) in &running {
                if !injected_pids.contains(pid) {
                    let pid_val = *pid;
                    let comm_val = comm.clone();
                    let settings = build_settings(&config, &theme_state);
                    let grace = grace_period;
                    let restart = config.watch.restart_after_inject;
                    let comm_for_restart = if restart {
                        Some(comm.clone())
                    } else {
                        None
                    };

                    eprintln!(
                        "[gtkd] detected new {} (PID {}), scheduling injection in {}ms...",
                        comm_val,
                        pid_val,
                        grace.as_millis()
                    );

                    std::thread::spawn(move || {
                        std::thread::sleep(grace);
                        eprintln!(
                            "[gtkd] injecting into {} (PID {})",
                            comm_val, pid_val
                        );
                        inject_async(pid_val, settings);
                    });

                    injected_pids.insert(pid_val);
                    watched_apps.mark_injected(pid_val);

                    if restart {
                        if let Some(ref name) = comm_for_restart {
                            restart_app(name);
                        }
                        inject_pause_until = Some(now + inject_pause);
                    }
                }
            }
        }

        // Clear pause if it has expired
        if let Some(until) = inject_pause_until {
            if now >= until {
                inject_pause_until = None;
            }
        }

        // Prune dead PIDs from injected set
        watched_apps.prune_dead();
        injected_pids
            .retain(|&pid| std::path::Path::new(&format!("/proc/{}", pid)).exists());
    }
}
