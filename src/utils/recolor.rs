use image::{Rgb, RgbImage};
use palette::{FromColor, IntoColor, Oklch, OklabHue, Srgb};

use akspraypaint::NoctaliaTheme;

// ─────────────────────────────────────────────────────────────────────────────
// CONSTANTS  (tune these to taste)
// ─────────────────────────────────────────────────────────────────────────────

/// Number of dominant color clusters to extract from the source image.
/// 6–8 is usually enough to capture backgrounds, outlines, fills, highlights.
const K: usize = 7;

/// K-means iterations. More = more accurate clusters, more CPU time.
const KMEANS_ITER: usize = 12;

/// How many pixels to sample for clustering. Sampling avoids O(N²) cost on
/// huge wallpapers while still capturing the color distribution accurately.
const MAX_SAMPLES: usize = 40_000;

/// Controls how "sharp" the region boundaries are during transfer.
/// Higher → harder edges, lower → softer cross-region blending.
/// Range 4–16; 8 is a good default.
const SHARPNESS: f32 = 8.0;

/// Tiny constant to avoid division by zero in inverse-distance weighting.
const EPSILON: f32 = 1e-6;

// ─────────────────────────────────────────────────────────────────────────────
// PUBLIC API
// ─────────────────────────────────────────────────────────────────────────────

/// Recolor `input` so its colors match the noctalia `theme`.
///
/// Works on **any** image — no hardcoded source colors.
///
/// # Algorithm (three stages)
///
/// 1. **Cluster** — Run k-means on a sample of the image's pixels in Oklch
///    space to find `K` dominant colors.  Oklch is used because its Euclidean
///    distance is perceptually uniform: clusters that look visually distinct
///    are numerically distinct, and vice-versa.
///
/// 2. **Match** — Sort both the extracted clusters AND the theme palette by
///    Oklch lightness (L), then zip them in order: darkest cluster → darkest
///    theme color, lightest cluster → lightest theme color.  This works for
///    arbitrary images because luminance order is semantically stable: dark
///    areas (shadows, backgrounds) should stay dark in the output; light
///    highlights should stay light.
///
/// 3. **Transfer** — For every pixel, compute an inverse-power-distance
///    weighted blend across all (source_cluster → target_color) pairs.
///    Pixels close to one cluster are almost entirely mapped to that cluster's
///    target; pixels that sit between two clusters get a smooth interpolation.
///    Lightness is preserved as a *ratio* (orig_L / cluster_L × target_L) so
///    gradients remain smooth instead of being collapsed to flat regions.
pub fn recolor_wallpaper(input: &RgbImage, theme: &NoctaliaTheme) -> RgbImage {
    // ── Stage 1: extract dominant colors via k-means ──────────────────────
    let samples = sample_pixels(input, MAX_SAMPLES);
    let mut clusters = kmeans(&samples, K, KMEANS_ITER);

    // Sort clusters darkest → lightest
    clusters.sort_by(|a, b| a.l.partial_cmp(&b.l).unwrap());

    // ── Stage 2: build theme palette, sort darkest → lightest ────────────
    let mut theme_colors: Vec<Oklch<f32>> = theme
        .palette()
        .into_iter()
        .map(|c| rgb_arr_to_oklch(c))
        .collect();
    theme_colors.sort_by(|a, b| a.l.partial_cmp(&b.l).unwrap());

    // If K ≠ theme palette length, map clusters to nearest theme color by
    // fractional position in the sorted list so every cluster gets a target.
    let mappings: Vec<(Oklch<f32>, Oklch<f32>)> = clusters
        .iter()
        .enumerate()
        .map(|(i, &src)| {
            let t = (i * (theme_colors.len() - 1)) / (K - 1).max(1);
            let t = t.min(theme_colors.len() - 1);
            (src, theme_colors[t])
        })
        .collect();

    // ── Stage 3: per-pixel transfer ───────────────────────────────────────
    let (width, height) = input.dimensions();
    let mut output = RgbImage::new(width, height);

    for (x, y, pixel) in input.enumerate_pixels() {
        let orig = rgb_to_oklch(pixel);
        output.put_pixel(x, y, transfer_pixel(orig, &mappings));
    }

    output
}

// ─────────────────────────────────────────────────────────────────────────────
// STAGE 1 – SAMPLING
// ─────────────────────────────────────────────────────────────────────────────

/// Collect up to `max_samples` pixels from `img`, evenly strided so that the
/// full image area is represented rather than just the top-left corner.
fn sample_pixels(img: &RgbImage, max_samples: usize) -> Vec<Oklch<f32>> {
    let (w, h) = img.dimensions();
    let total = (w * h) as usize;
    let stride = (total / max_samples).max(1);

    img.pixels()
        .enumerate()
        .filter(|(i, _)| i % stride == 0)
        .map(|(_, p)| rgb_to_oklch(p))
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// STAGE 1 – K-MEANS IN OKLCH SPACE
// ─────────────────────────────────────────────────────────────────────────────

/// Run Lloyd's k-means algorithm on `points` in Oklch space.
/// Returns the `k` centroid colors after `iters` iterations.
///
/// We treat Oklch as *cartesian Oklab* for centroid arithmetic — converting
/// (chroma, hue) → (a, b) so that averaging is geometrically correct.
/// A naive average of hue angles breaks near 0°/360°; the (a,b) approach
/// does not.
fn kmeans(points: &[Oklch<f32>], k: usize, iters: usize) -> Vec<Oklch<f32>> {
    if points.is_empty() || k == 0 {
        return vec![];
    }

    // Initialise centroids via KMeans++ for better spread.
    let mut centroids = kmeans_plus_plus_init(points, k);

    for _ in 0..iters {
        // Assignment step: map each point to its nearest centroid index.
        let assignments: Vec<usize> = points
            .iter()
            .map(|p| nearest_centroid(p, &centroids))
            .collect();

        // Update step: recompute each centroid as the mean of its members.
        let mut new_centroids = vec![[0.0f32; 3]; k]; // [L, a, b]
        let mut counts = vec![0usize; k];

        for (p, &c) in points.iter().zip(assignments.iter()) {
            let (a, b) = hue_to_ab(p.chroma, p.hue);
            new_centroids[c][0] += p.l;
            new_centroids[c][1] += a;
            new_centroids[c][2] += b;
            counts[c] += 1;
        }

        for (i, nc) in new_centroids.iter().enumerate() {
            if counts[i] == 0 {
                // Empty cluster: keep old centroid to avoid collapse.
                continue;
            }
            let n = counts[i] as f32;
            let l = nc[0] / n;
            let a = nc[1] / n;
            let b = nc[2] / n;
            let chroma = (a * a + b * b).sqrt();
            let hue = OklabHue::from_degrees(b.atan2(a).to_degrees());
            centroids[i] = Oklch { l, chroma, hue };
        }
    }

    centroids
}

/// KMeans++ initialisation: first centroid is random (well-spread via stride),
/// each subsequent one is chosen with probability proportional to squared
/// distance from the nearest already-chosen centroid.
/// This dramatically reduces bad initialisations vs uniform random.
fn kmeans_plus_plus_init(points: &[Oklch<f32>], k: usize) -> Vec<Oklch<f32>> {
    let mut centroids = Vec::with_capacity(k);

    // Pick first centroid deterministically: the point at the 25th-percentile
    // index (avoids edge-case outlier colours that live at position 0).
    let first_idx = points.len() / 4;
    centroids.push(points[first_idx]);

    for _ in 1..k {
        // For each point, find its distance to the nearest existing centroid.
        let distances: Vec<f32> = points
            .iter()
            .map(|p| {
                centroids
                    .iter()
                    .map(|c| oklch_distance(p, c))
                    .fold(f32::MAX, f32::min)
            })
            .collect();

        // Choose the next centroid as the point with the maximum distance
        // (deterministic analogue of the probabilistic D² sampling).
        let next_idx = distances
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
            .map(|(i, _)| i)
            .unwrap_or(0);

        centroids.push(points[next_idx]);
    }

    centroids
}

fn nearest_centroid(p: &Oklch<f32>, centroids: &[Oklch<f32>]) -> usize {
    centroids
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            oklch_distance(p, a)
                .partial_cmp(&oklch_distance(p, b))
                .unwrap()
        })
        .map(|(i, _)| i)
        .unwrap_or(0)
}

// ─────────────────────────────────────────────────────────────────────────────
// STAGE 3 – PER-PIXEL COLOUR TRANSFER
// ─────────────────────────────────────────────────────────────────────────────

/// Map a single pixel from source space to theme space.
///
/// For each (source_cluster, target_color) pair, compute a weight
///   w_i = 1 / (dist(pixel, source_i)^SHARPNESS + EPSILON)
///
/// The output color is the weighted average of all target colors in Oklab
/// cartesian space, with lightness scaled by the ratio orig_L / source_L
/// so that gradients within a region are preserved proportionally.
fn transfer_pixel(orig: Oklch<f32>, mappings: &[(Oklch<f32>, Oklch<f32>)]) -> Rgb<u8> {
    let mut total_w = 0.0f32;
    let mut out_l = 0.0f32;
    let mut out_a = 0.0f32;
    let mut out_b = 0.0f32;

    for (src, tgt) in mappings {
        let dist = oklch_distance(&orig, src).max(EPSILON);
        let w = 1.0 / dist.powf(SHARPNESS);

        // Ratio-preserve lightness so a pixel that is 80% as bright as its
        // cluster centroid comes out 80% as bright as the target centroid.
        // Clamp ratio to [0.5, 2.0] to prevent runaway values at extremes.
        let l_ratio = if src.l > 0.01 {
            (orig.l / src.l).clamp(0.5, 2.0)
        } else {
            1.0
        };
        let mapped_l = (tgt.l * l_ratio).clamp(0.0, 1.0);

        let (ta, tb) = hue_to_ab(tgt.chroma, tgt.hue);

        out_l += w * mapped_l;
        out_a += w * ta;
        out_b += w * tb;
        total_w += w;
    }

    let final_l = (out_l / total_w).clamp(0.0, 1.0);
    let final_chroma = {
        let a = out_a / total_w;
        let b = out_b / total_w;
        (a * a + b * b).sqrt().clamp(0.0, 0.5)
    };
    let final_hue = OklabHue::from_degrees(
        (out_b / total_w).atan2(out_a / total_w).to_degrees(),
    );

    oklch_to_rgb(&Oklch {
        l: final_l,
        chroma: final_chroma,
        hue: final_hue,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// HELPERS
// ─────────────────────────────────────────────────────────────────────────────

/// Perceptually uniform distance in Oklch (via cartesian Oklab conversion).
fn oklch_distance(a: &Oklch<f32>, b: &Oklch<f32>) -> f32 {
    let (aa, ab) = hue_to_ab(a.chroma, a.hue);
    let (ba, bb) = hue_to_ab(b.chroma, b.hue);
    let dl = a.l - b.l;
    let da = aa - ba;
    let db = ab - bb;
    (dl * dl + da * da + db * db).sqrt()
}

/// Convert polar (chroma, hue) to cartesian (a, b) Oklab coordinates.
/// This is required for numerically correct centroid averaging and blending:
/// averaging hue angles directly wraps incorrectly near 0°/360°.
fn hue_to_ab(chroma: f32, hue: OklabHue<f32>) -> (f32, f32) {
    let rad = hue.into_radians();
    (chroma * rad.cos(), chroma * rad.sin())
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
    Oklch::from_color(srgb.into_linear())
}

fn oklch_to_rgb(c: &Oklch<f32>) -> Rgb<u8> {
    let linear: palette::LinSrgb<f32> = (*c).into_color();
    let srgb: Srgb<f32> = linear.into_encoding();
    Rgb([
        (srgb.red   * 255.0).round().clamp(0.0, 255.0) as u8,
        (srgb.green * 255.0).round().clamp(0.0, 255.0) as u8,
        (srgb.blue  * 255.0).round().clamp(0.0, 255.0) as u8,
    ])
}

// ─────────────────────────────────────────────────────────────────────────────
// TESTS
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use akspraypaint::NoctaliaTheme;

    fn test_theme() -> NoctaliaTheme {
        NoctaliaTheme {
            primary:            [180, 50,  220],
            on_primary:         [255, 255, 255],
            surface:            [30,  20,  40 ],
            on_surface:         [220, 210, 230],
            surface_variant:    [60,  50,  80 ],
            on_surface_variant: [200, 190, 210],
            error:              [220, 50,  50 ],
        }
    }

    #[test]
    fn test_recolor_solid_image() {
        let mut img = RgbImage::new(64, 64);
        for p in img.pixels_mut() { *p = Rgb([128, 64, 192]); }
        let result = recolor_wallpaper(&img, &test_theme());
        assert_eq!(result.dimensions(), (64, 64));
    }

    #[test]
    fn test_recolor_gradient_image() {
        // Gradient from black to white — all achromatic
        let mut img = RgbImage::new(256, 1);
        for x in 0..256u32 {
            img.put_pixel(x, 0, Rgb([x as u8, x as u8, x as u8]));
        }
        let result = recolor_wallpaper(&img, &test_theme());
        // Verify monotonicity: left pixels should be darker than right pixels
        let left  = rgb_to_oklch(result.get_pixel(0, 0));
        let right = rgb_to_oklch(result.get_pixel(255, 0));
        assert!(left.l < right.l, "gradient should remain monotone after recolor");
    }

    #[test]
    fn test_kmeans_returns_k_clusters() {
        let samples: Vec<Oklch<f32>> = (0..100)
            .map(|i| Oklch {
                l: i as f32 / 100.0,
                chroma: 0.1,
                hue: OklabHue::from_degrees(120.0),
            })
            .collect();
        let clusters = kmeans(&samples, K, KMEANS_ITER);
        assert_eq!(clusters.len(), K);
    }

    #[test]
    fn test_dark_stays_dark() {
        let mut img = RgbImage::new(1, 1);
        img.put_pixel(0, 0, Rgb([5, 5, 5]));
        let result = recolor_wallpaper(&img, &test_theme());
        let out = rgb_to_oklch(result.get_pixel(0, 0));
        assert!(out.l < 0.25, "near-black input should stay dark, got L={}", out.l);
    }

    #[test]
    fn test_light_stays_light() {
        let mut img = RgbImage::new(1, 1);
        img.put_pixel(0, 0, Rgb([250, 250, 250]));
        let result = recolor_wallpaper(&img, &test_theme());
        let out = rgb_to_oklch(result.get_pixel(0, 0));
        assert!(out.l > 0.7, "near-white input should stay light, got L={}", out.l);
    }
}
