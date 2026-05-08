mod commands;
mod utils;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "akspraypaint", about = "Recolors wallpaper to match noctalia theme")]
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
    },
    /// Watch for noctalia theme changes and automatically recolor
    Watch {
        /// Path to wallpaper (auto-detect if not provided)
        #[arg(long)]
        wallpaper: Option<String>,
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
        Command::Run { wallpaper, verbose } => commands::run::run(wallpaper.as_deref(), verbose),
        Command::Watch { wallpaper } => commands::watch::watch(wallpaper.as_deref()),
        Command::Set { path } => commands::set::set(&path),
        Command::Status => commands::set::status(),
        Command::Clean => commands::set::clean(),
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
