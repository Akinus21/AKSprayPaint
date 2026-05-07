use image::{Rgb, RgbImage};
use palette::{FromColor, Oklch, OklabHue, Srgb};

pub fn recolor_wallpaper(
    input: &RgbImage,
    target_palette: &[[u8; 3]],
) -> RgbImage {
    let (width, height) = input.dimensions();

    let mut palette_lch: Vec<(f32, Oklch<f32>)> = target_palette
        .iter()
        .map(|c| {
            let lch = rgb_to_oklch(&Rgb(*c));
            (lch.l, lch)
        })
        .collect();
    palette_lch.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

    let n = palette_lch.len();
    if n < 2 {
        let mut output = RgbImage::new(width, height);
        for y in 0..height {
            for x in 0..width {
                output.put_pixel(x, y, *input.get_pixel(x, y));
            }
        }
        return output;
    }

    let step = (n - 1).max(1);
    let indices: Vec<usize> = (0..n).step_by(step).collect();
    let gradient: Vec<(f32, Oklch<f32>)> = indices
        .iter()
        .copied()
        .map(|i| (palette_lch[i].0, palette_lch[i].1))
        .collect();

    let mut output = RgbImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let pixel = input.get_pixel(x, y);
            let lch = rgb_to_oklch(pixel);
            let luminance = (lch.l + 0.05).clamp(0.0, 1.0);
            let theme = interpolate_oklch(&gradient, luminance);
            output.put_pixel(x, y, oklch_to_rgb(&theme));
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

fn interpolate_oklch(gradient: &[(f32, Oklch<f32>)], luminance: f32) -> Oklch<f32> {
    if gradient.is_empty() {
        return Oklch::new(0.5, 0.0, OklabHue::from_degrees(0.0));
    }
    if gradient.len() == 1 {
        return gradient[0].1;
    }

    if luminance <= gradient[0].0 {
        return gradient[0].1;
    }
    if luminance >= gradient[gradient.len() - 1].0 {
        return gradient[gradient.len() - 1].1;
    }

    for i in 0..gradient.len() - 1 {
        let (l1, c1) = gradient[i];
        let (l2, c2) = gradient[i + 1];
        if luminance >= l1 && luminance <= l2 {
            let t = if (l2 - l1).abs() > 0.0001 {
                (luminance - l1) / (l2 - l1)
            } else {
                0.0
            };
            let t = t.clamp(0.0, 1.0);
            return Oklch {
                l: c1.l + (c2.l - c1.l) * t,
                chroma: c1.chroma + (c2.chroma - c1.chroma) * t,
                hue: OklabHue::from_degrees(
                    c1.hue.into_positive_degrees()
                        + (c2.hue.into_positive_degrees() - c1.hue.into_positive_degrees()) * t,
                ),
            };
        }
    }

    gradient[gradient.len() - 1].1
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
        let palette = vec![[200, 50, 50], [50, 50, 200], [50, 200, 50]];
        let result = recolor_wallpaper(&img, &palette);
        assert_eq!(result.dimensions(), (32, 32));
    }
}
