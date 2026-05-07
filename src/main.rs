mod commands;
mod utils;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "akspraypaint", about = "Recolors wallpaper to match noctalia theme")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Recolor the current wallpaper to match the noctalia theme (one-shot)
    Run,
    /// Watch for noctalia theme changes and automatically recolor
    Watch,
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

    let result = match cli.command {
        Command::Run => commands::run::run(),
        Command::Watch => commands::watch::watch(),
        Command::Set { path } => commands::set::set(&path),
        Command::Status => commands::set::status(),
        Command::Clean => commands::set::clean(),
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
