use image::{Rgb, RgbImage};
use palette::{FromColor, IntoColor, Oklch, OklabHue, Srgb};

use akspraypaint::NoctaliaTheme;

/// Recolor a wallpaper to match a noctalia theme.
///
/// Strategy: for each pixel, find the "closest" theme hue bucket, then
/// rotate the pixel's hue to match while preserving its lightness and a
/// blended chroma. Achromatic/near-grey pixels are instead mapped to the
/// theme's surface colors by lightness alone, which keeps gradients smooth.
pub fn recolor_wallpaper(input: &RgbImage, theme: &NoctaliaTheme) -> RgbImage {
    // Build palette entries as Oklch
    let palette: Vec<Oklch<f32>> = theme
        .palette()
        .into_iter()
        .map(|c| rgb_arr_to_oklch(c))
        .collect();

    let (width, height) = input.dimensions();
    let mut output = RgbImage::new(width, height);

    for (x, y, pixel) in input.enumerate_pixels() {
        let orig = rgb_to_oklch(pixel);
        output.put_pixel(x, y, map_pixel(orig, &palette));
    }

    output
}

/// Map a single pixel's Oklch value to the theme palette.
fn map_pixel(orig: Oklch<f32>, palette: &[Oklch<f32>]) -> Rgb<u8> {
    const CHROMA_THRESHOLD: f32 = 0.04; // below this = "grey/achromatic"

    if orig.chroma < CHROMA_THRESHOLD {
        // --- Achromatic path: map by lightness to the nearest surface tone ---
        // Find the palette entry whose lightness is closest.
        let target = palette
            .iter()
            .min_by(|a, b| {
                let da = (a.l - orig.l).abs();
                let db = (b.l - orig.l).abs();
                da.partial_cmp(&db).unwrap()
            })
            .unwrap();

        // Preserve original lightness, adopt target hue & small chroma so
        // the background doesn't become a flat slab of colour.
        let out = Oklch {
            l: orig.l,
            chroma: (target.chroma * 0.15).min(0.04),
            hue: target.hue,
        };
        return oklch_to_rgb(&out);
    }

    // --- Chromatic path: rotate hue, preserve lightness, blend chroma ---
    // Find the palette entry whose hue is closest (circular distance).
    let target = palette
        .iter()
        .min_by(|a, b| {
            let da = hue_dist(orig.hue, a.hue);
            let db = hue_dist(orig.hue, b.hue);
            da.partial_cmp(&db).unwrap()
        })
        .unwrap();

    let out = Oklch {
        l: orig.l,
        // Blend chroma: keep most of original so contrast/saturation is preserved,
        // nudge toward theme chroma so very different saturations converge a bit.
        chroma: (orig.chroma * 0.7 + target.chroma * 0.3).clamp(0.0, 0.5),
        hue: target.hue,
    };
    oklch_to_rgb(&out)
}

/// Circular distance between two hues (0..=180 range result).
fn hue_dist(a: OklabHue<f32>, b: OklabHue<f32>) -> f32 {
    let diff = (a.into_degrees() - b.into_degrees()).abs() % 360.0;
    if diff > 180.0 { 360.0 - diff } else { diff }
}

fn rgb_arr_to_oklch(arr: [u8; 3]) -> Oklch<f32> {
    rgb_to_oklch(&Rgb(arr))
}

fn rgb_to_oklch(p: &Rgb<u8>) -> Oklch<f32> {
    let srgb = Srgb::new(
        p[0] as f32 / 255.0,
        p[1] as f32 / 255.0,
        p[2] as f32 / 255.0,
    );
    let linear = srgb.into_linear();
    Oklch::from_color(linear)
}

fn oklch_to_rgb(c: &Oklch<f32>) -> Rgb<u8> {
    let linear: palette::LinSrgb<f32> = (*c).into_color();
    let srgb: Srgb<f32> = linear.into_encoding();
    Rgb([
        (srgb.red * 255.0).round().clamp(0.0, 255.0) as u8,
        (srgb.green * 255.0).round().clamp(0.0, 255.0) as u8,
        (srgb.blue * 255.0).round().clamp(0.0, 255.0) as u8,
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
        let theme = akspraypaint::NoctaliaTheme {
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

    #[test]
    fn test_achromatic_stays_dark() {
        // A near-black pixel should stay dark after recoloring
        let mut img = RgbImage::new(1, 1);
        img.put_pixel(0, 0, Rgb([10, 10, 10]));
        let theme = akspraypaint::NoctaliaTheme {
            primary: [180, 50, 220],
            on_primary: [255, 255, 255],
            surface: [30, 20, 40],
            on_surface: [220, 210, 230],
            surface_variant: [60, 50, 80],
            on_surface_variant: [200, 190, 210],
            error: [220, 50, 50],
        };
        let result = recolor_wallpaper(&img, &theme);
        let p = result.get_pixel(0, 0);
        // Should stay very dark (L preserved)
        let out = rgb_to_oklch(p);
        assert!(out.l < 0.15, "dark pixel should remain dark, got L={}", out.l);
    }
}
