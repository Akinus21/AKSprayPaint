use image::{Rgb, RgbImage};
use palette::{FromColor, IntoColor, Oklch, Srgb};

pub fn recolor_wallpaper(
    input: &RgbImage,
    target_palette: &[[u8; 3]],
) -> RgbImage {
    let (width, height) = input.dimensions();
    let total = (width as usize) * (height as usize);
    let max_samples = 200_000usize;
    let stride = if total > max_samples {
        (total / max_samples).max(1)
    } else {
        1
    };

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

    let target_oklch: Vec<Oklch<f32>> = target_palette
        .iter()
        .map(|c| {
            let rgb = Rgb(*c);
            rgb_to_oklch(&rgb)
        })
        .collect();

    let k = target_oklch.len();
    let mut means = pick_spread_means(&samples, k);
    let assignments = kmeans_luminance(&samples, &mut means, 20);

    let mut cluster_map: Vec<usize> = vec![0; k];
    for ci in 0..k {
        let closest = target_oklch
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                oklch_distance(&means[ci], a)
                    .partial_cmp(&oklch_distance(&means[ci], b))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(idx, _)| idx)
            .unwrap_or(0);
        cluster_map[ci] = closest;
    }

    let mut output = RgbImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let idx = (y as usize) * (width as usize) + (x as usize);
            let sample_idx = idx / stride;
            let cluster = if sample_idx < assignments.len() {
                assignments[sample_idx]
            } else {
                0
            };
            let target = &target_oklch[cluster_map[cluster]];

            let p = input.get_pixel(x, y);
            let c = rgb_to_oklch(p);

            let base_lum = means[cluster].l;
            let lum_ratio = if base_lum > 0.001 {
                (c.l / base_lum).clamp(0.3, 2.5)
            } else {
                1.0
            };

            let new = Oklch {
                l: (target.l * lum_ratio).clamp(0.0, 1.0),
                chroma: target.chroma * 0.8 + c.chroma * 0.2,
                hue: target.hue,
            };
            output.put_pixel(x, y, oklch_to_rgb(&new));
        }
    }
    output
}

fn rgb_to_oklch(p: &Rgb<u8>) -> Oklch<f32> {
    let linear: Srgb<f32> = Srgb::new(
        p[0] as f32 / 255.0,
        p[1] as f32 / 255.0,
        p[2] as f32 / 255.0,
    )
    .into_linear();
    Oklch::from_color(linear)
}

fn oklch_to_rgb(c: &Oklch<f32>) -> Rgb<u8> {
    let linear: Srgb<f32> = Srgb::from_color(*c);
    let encoded = linear.into_encoding();
    Rgb([
        (encoded.red * 255.0).round().clamp(0.0, 255.0) as u8,
        (encoded.green * 255.0).round().clamp(0.0, 255.0) as u8,
        (encoded.blue * 255.0).round().clamp(0.0, 255.0) as u8,
    ])
}

fn oklch_distance(a: &Oklch<f32>, b: &Oklch<f32>) -> f32 {
    let dl = a.l - b.l;
    let da = a.chroma * a.hue.to_radians().cos() - b.chroma * b.hue.to_radians().cos();
    let db = a.chroma * a.hue.to_radians().sin() - b.chroma * b.hue.to_radians().sin();
    dl * dl + da * da + db * db
}

fn pick_spread_means(samples: &[Oklch<f32>], k: usize) -> Vec<Oklch<f32>> {
    let mut means = vec![samples[0]];
    for _ in 1..k {
        let mut best = samples[0];
        let mut best_dist = 0.0f32;
        for &s in samples {
            let min_d = means.iter().map(|m| oklch_distance(&s, m)).fold(f32::MAX, f32::min);
            if min_d > best_dist {
                best_dist = min_d;
                best = s;
            }
        }
        means.push(best);
    }
    means
}

fn kmeans_luminance(
    samples: &[Oklch<f32>],
    means: &mut [Oklch<f32>],
    iters: usize,
) -> Vec<usize> {
    let n = samples.len();
    let k = means.len();
    let mut assignments = vec![0usize; n];
    for _ in 0..iters {
        for (i, &s) in samples.iter().enumerate() {
            let mut best = 0;
            let mut best_d = f32::MAX;
            for (j, m) in means.iter().enumerate() {
                let d = oklch_distance(&s, m);
                if d < best_d {
                    best_d = d;
                    best = j;
                }
            }
            assignments[i] = best;
        }
        let mut counts = vec![0usize; k];
        let mut sums = vec![(0.0f32, 0.0f32, 0.0f32); k];
        for (i, &s) in samples.iter().enumerate() {
            let c = assignments[i];
            counts[c] += 1;
            sums[c].0 += s.l;
            sums[c].1 += s.chroma;
            sums[c].2 += s.hue.into_positive_degrees();
        }
        for j in 0..k {
            if counts[j] > 0 {
                let nf = counts[j] as f32;
                let avg_hue = sums[j].2 / nf;
                means[j] = Oklch {
                    l: (sums[j].0 / nf).clamp(0.0, 1.0),
                    chroma: (sums[j].1 / nf).clamp(0.0, 0.5),
                    hue: palette::Hue::from_degrees(avg_hue),
                };
            }
        }
    }
    assignments
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
