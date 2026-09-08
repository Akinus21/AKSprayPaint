mod commands;
mod gtkd;
mod utils;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "akspraypaint",
    about = "Recolors wallpaper and icons to match the noctalia theme"
)]
struct Cli {
    #[arg(long, help = "Kill the running watch daemon")]
    disable: bool,
    #[arg(long, help = "Verbose output")]
    verbose: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Recolor the current wallpaper to match the noctalia theme (one-shot)
    Run {
        /// Path to wallpaper (auto-detect if not provided)
        #[arg(long)]
        wallpaper: Option<String>,
        /// Verbose output
        #[arg(long)]
        verbose: bool,
        /// Skip cache and force recolor
        #[arg(long)]
        no_cache: bool,
    },
    /// Watch for noctalia theme changes and automatically recolor wallpaper
    Watch {
        /// Path to wallpaper (auto-detect if not provided)
        #[arg(long)]
        wallpaper: Option<String>,
    },
    /// Recolor GTK icons to match the active theme
    Icons {
        #[command(subcommand)]
        sub: IconsSubcommand,
    },
    /// Recolor a specific image and set it as wallpaper
    Set {
        /// Path to the image file
        path: String,
    },
    /// Show current status (wallpaper, theme, cache)
    Status,
    /// Remove cached recolored images and/or recolored icon themes
    Clean {
        /// What to clean: "icons", "wallpapers", or both (default)
        #[arg(default_value = "all", hide_default_value = true)]
        target: String,
    },
    /// Upgrade akspraypaint via Homebrew
    Upgrade,
    /// Open the GTK bridge daemon config in $EDITOR
    Config,
    /// Run the GTK bridge daemon
    Gtkd {
        #[clap(skip)]
        _unit: (),
    },
}

#[derive(Subcommand)]
enum IconsSubcommand {
    /// Recolor icons now (one-shot)
    Recolor {
        /// Verbose output
        #[arg(long)]
        verbose: bool,
        /// Override the theme name (for daemon use)
        #[arg(long)]
        theme: Option<String>,
    },
    /// Watch for theme changes and automatically recolor icons
    Watch,
}

fn main() -> Result<(), String> {
    let cli = Cli::parse();

    if cli.disable {
        if let Err(e) = utils::kill_watch_daemon() {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
        return Ok(());
    }

    let command = match cli.command {
        Some(c) => c,
        None => {
            eprintln!("Error: no command specified. Use --help for usage.");
            std::process::exit(1);
        }
    };

    let result = match command {
        Command::Run {
            wallpaper,
            verbose,
            no_cache,
        } => commands::run::run(wallpaper.as_deref(), verbose, no_cache),
        Command::Watch { wallpaper } => commands::watch::watch(wallpaper.as_deref()),
        Command::Icons { sub } => match sub {
            IconsSubcommand::Recolor { verbose, ref theme } => {
                commands::icons::recolor(theme.as_deref(), verbose)
            }
            IconsSubcommand::Watch => commands::icons::watch(),
        },
        Command::Set { path } => commands::set::set(&path),
        Command::Status => commands::set::status(),
        Command::Clean { target } => {
            let t = match target.as_str() {
                "all" => None,
                "icons" => Some("icons"),
                "wallpapers" => Some("wallpapers"),
                _ => {
                    return Err(format!(
                        "unknown clean target '{}': valid values are 'icons', 'wallpapers', or 'all'",
                        target
                    ));
                }
            };
            commands::clean::clean(t)
        }
        Command::Upgrade => {
            commands::upgrade::upgrade();
            Ok(())
        }
        Command::Config => {
            let path = gtkd::config::default_path();
            let _ = gtkd::config::GtkBridgeConfig::load_or_create(&path);
            let editor = std::env::var("EDITOR").unwrap_or_else(|_| "nano".to_string());
            let result = std::process::Command::new(&editor).arg(&path).status();
            if let Err(e) = result {
                eprintln!("failed to open editor: {}", e);
            }
            Ok(())
        }
        Command::Gtkd { .. } => {
            let config = match gtkd::config::GtkBridgeConfig::load_or_create(
                &gtkd::config::default_path(),
            ) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("[gtkd] config error: {}", e);
                    return Ok(());
                }
            };
            gtkd::daemon::run(config)
        }
    };

    result
}
