use image::{Rgb, RgbImage};
use palette::{FromColor, Oklch, OklabHue, Srgb};

use akspraypaint::NoctaliaTheme;

pub fn recolor_wallpaper(
    input: &RgbImage,
    theme: &NoctaliaTheme,
) -> RgbImage {
    let (width, height) = input.dimensions();
    let total = (width as usize) * (height as usize);
    let max_samples = 200_000usize;
    let stride = if total > max_samples {
        (total / max_samples).max(1)
    } else {
        1
    };

    const SHADOW_L: f32 = 0.05;
    const SHADOW_C: f32 = 0.05;

    let bright = rgb_to_oklch(&Rgb(theme.bright_color()));
    let light_surface = rgb_to_oklch(&Rgb(theme.light_surface_color()));
    let dark_surface = rgb_to_oklch(&Rgb(theme.dark_surface_color()));
    let background = rgb_to_oklch(&Rgb(theme.background_color()));

    let target_colors = [bright, light_surface, dark_surface, background];

    let mut samples: Vec<Oklch<f32>> = Vec::with_capacity(total / stride);
    let mut flat_idx: Vec<usize> = Vec::with_capacity(total / stride);
    for y in 0..height {
        for x in 0..width {
            let idx = (y as usize) * (width as usize) + (x as usize);
            if idx % stride == 0 {
                flat_idx.push(idx);
                let p = input.get_pixel(x, y);
                let c: Oklch<f32> = rgb_to_oklch(p);
                samples.push(c);
            }
        }
    }

    let mut assignments = vec![0usize; samples.len()];
    for (i, s) in samples.iter().enumerate() {
        let mut best = 0;
        let mut best_d = f32::MAX;
        for (j, tc) in target_colors.iter().enumerate() {
            let d = oklch_distance(s, tc);
            if d < best_d {
                best_d = d;
                best = j;
            }
        }
        assignments[i] = best;
    }

    let mut output = RgbImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let idx = (y as usize) * (width as usize) + (x as usize);
            let sample_idx = idx / stride;

            let pixel = input.get_pixel(x, y);
            let original = rgb_to_oklch(pixel);
            let is_shadow = original.l < SHADOW_L && original.chroma < SHADOW_C;

            let cluster = if sample_idx < assignments.len() {
                assignments[sample_idx]
            } else {
                0
            };
            let target = &target_colors[cluster];

            let new_l = if is_shadow {
                original.l
            } else {
                (original.l * 0.4 + target.l * 0.6).clamp(0.0, 1.0)
            };

            let new_chroma = if is_shadow {
                target.chroma * 0.5 + original.chroma * 0.5
            } else {
                target.chroma * 0.8 + original.chroma * 0.2
            };

            let new = Oklch {
                l: new_l,
                chroma: new_chroma.clamp(0.0, 0.5),
                hue: target.hue,
            };

            output.put_pixel(x, y, oklch_to_rgb(&new));
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

fn oklch_distance(a: &Oklch<f32>, b: &Oklch<f32>) -> f32 {
    let dl = a.l - b.l;
    let da = a.chroma * a.hue.into_radians().cos() - b.chroma * b.hue.into_radians().cos();
    let db = a.chroma * a.hue.into_radians().sin() - b.chroma * b.hue.into_radians().sin();
    dl * dl + da * da + db * db
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
