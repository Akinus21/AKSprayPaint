mod commands;
mod utils;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "akspraypaint", about = "Recolors wallpaper and icons to match the noctalia theme")]
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
    /// Remove all cached recolored images
    Clean,
    /// Upgrade akspraypaint via Homebrew
    Upgrade,
}

#[derive(Subcommand)]
enum IconsSubcommand {
    /// Recolor icons now (one-shot)
    Recolor {
        /// Verbose output
        #[arg(long)]
        verbose: bool,
    },
    /// Watch for theme changes and automatically recolor icons
    Watch,
    /// Remove the PurpleHaze icon theme
    Clean,
}

fn main() {
    let cli = Cli::parse();

    if cli.disable {
        if let Err(e) = utils::kill_watch_daemon() {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
        return;
    }

    let command = match cli.command {
        Some(c) => c,
        None => {
            eprintln!("Error: no command specified. Use --help for usage.");
            std::process::exit(1);
        }
    };

    let result = match command {
        Command::Run { wallpaper, verbose, no_cache } => {
            commands::run::run(wallpaper.as_deref(), verbose, no_cache)
        }
        Command::Watch { wallpaper } => commands::watch::watch(wallpaper.as_deref()),
        Command::Icons { sub } => match sub {
            IconsSubcommand::Recolor { verbose } => commands::icons::recolor(verbose),
            IconsSubcommand::Watch => commands::icons::watch(),
            IconsSubcommand::Clean => commands::icons::clean(),
        },
        Command::Set { path } => commands::set::set(&path),
        Command::Status => commands::set::status(),
        Command::Clean => commands::set::clean(),
        Command::Upgrade => commands::upgrade::upgrade(),
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
