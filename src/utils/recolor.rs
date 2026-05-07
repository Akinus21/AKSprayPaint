use image::{Rgb, RgbImage};
use palette::{FromColor, Oklch, OklabHue, Srgb};

use akspraypaint::NoctaliaTheme;

pub fn recolor_wallpaper(
    input: &RgbImage,
    theme: &NoctaliaTheme,
) -> RgbImage {
    let (width, height) = input.dimensions();

    let bright = rgb_to_oklch(&Rgb(theme.bright_color()));
    let light_surface = rgb_to_oklch(&Rgb(theme.light_surface_color()));
    let dark_surface = rgb_to_oklch(&Rgb(theme.dark_surface_color()));
    let background = rgb_to_oklch(&Rgb(theme.background_color()));

    let mut output = RgbImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let pixel = input.get_pixel(x, y);
            let lch = rgb_to_oklch(pixel);
            let luminance = lch.l;

            let new_color = if luminance > 0.7 {
                bright
            } else if luminance > 0.5 {
                light_surface
            } else if luminance > 0.3 {
                dark_surface
            } else {
                background
            };

            output.put_pixel(x, y, oklch_to_rgb(&new_color));
        }
    }
    output
}

fn rgb_to_oklch(p: &Rgb<u8>) -> Oklch<f32> {
    let rgb = Srgb::new(
        p[0] as f32 / 255.0,
        p[1] as f32 / 255.0,
        p[2] as f32 / 255.0,
    );
    let linear = rgb.into_linear();
    Oklch::from_color(linear)
}

fn oklch_to_rgb(c: &Oklch<f32>) -> Rgb<u8> {
    let linear: Srgb<f32> = Srgb::from_color(*c);
    let gamma: Srgb<f32> = linear.into_linear().into_encoding();
    Rgb([
        (gamma.red * 255.0).round().clamp(0.0, 255.0) as u8,
        (gamma.green * 255.0).round().clamp(0.0, 255.0) as u8,
        (gamma.blue * 255.0).round().clamp(0.0, 255.0) as u8,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recolor_runs() {
        let mut img = RgbImage::new(32, 32);
        for pixel in img.pixels_mut() {
            *pixel = Rgb([128, 64, 192]);
        }
        let theme = NoctaliaTheme {
            primary: [200, 50, 50],
            on_primary: [255, 255, 255],
            surface: [50, 50, 200],
            on_surface: [255, 255, 255],
            surface_variant: [30, 30, 30],
            on_surface_variant: [240, 240, 240],
            error: [200, 50, 50],
        };
        let result = recolor_wallpaper(&img, &theme);
        assert_eq!(result.dimensions(), (32, 32));
    }
}
