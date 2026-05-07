# AKSprayPaint

Recolors the current wallpaper to match the [noctalia](https://github.com/Akinus21/noctalia-shell) theme.

## Installation

```bash
cargo install --git https://github.com/Akinus21/AKSprayPaint
```

Or via Homebrew:

```bash
brew install Akinus21/tap/akspraypaint
```

## Usage

```bash
# Recolor the current wallpaper (one-shot)
akspraypaint run

# Recolor with a specific wallpaper path
akspraypaint run --wallpaper ~/Pictures/wallpaper.png

# Watch for noctalia theme changes and automatically recolor
akspraypaint watch

# Recolor a specific image and set it as wallpaper
akspraypaint set ~/Pictures/cat.png

# Show current status
akspraypaint status

# Clear cached recolored images
akspraypaint clean
```

**Note:** Wallpaper auto-detection supports swww, hyprpaper, swaybg, GNOME, and Noctalia Shell. If detection fails, use `--wallpaper` to specify the path manually.

## How It Works

AKSprayPaint reads the active noctalia theme from `~/.config/noctalia/colors.json`, extracts the theme's color palette, and applies a full color replacement to the current wallpaper. Recolored images are cached in `~/.cache/akspraypaint/` keyed by a hash of the theme content.

In `watch` mode, the process monitors `~/.config/noctalia/` for changes to `colors.json` and automatically reapplies the theme.
