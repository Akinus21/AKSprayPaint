use image::{Rgb, RgbImage};
use palette::{FromColor, IntoColor, Oklch, OklabHue, Srgb};

use akspraypaint::NoctaliaTheme;

// ─────────────────────────────────────────────────────────────────────────────
// CONSTANTS
// ─────────────────────────────────────────────────────────────────────────────

/// Number of dominant color clusters to extract from the source image.
const K: usize = 7;

/// K-means iterations.
const KMEANS_ITER: usize = 12;

/// Max pixels to sample for clustering.
const MAX_SAMPLES: usize = 40_000;

/// Controls sharpness of region boundaries in the transfer.
/// Higher → harder edges, lower → softer blending. Range 4–16.
const SHARPNESS: f32 = 8.0;

/// Avoid division by zero.
const EPSILON: f32 = 1e-6;

/// Base achromatic threshold. The actual per-pixel threshold scales UP as
/// lightness decreases — dark pixels have noisier chroma readings and need
/// a wider band before we trust their hue. See `is_achromatic()`.
const ACHROMATIC_BASE: f32 = 0.06;

/// How much the achromatic threshold widens per unit of darkness (1.0 - L).
/// At L=0.0 (pure black) the threshold is ACHROMATIC_BASE + ACHROMATIC_DARK_BOOST.
/// At L=1.0 (pure white) the threshold is just ACHROMATIC_BASE.
const ACHROMATIC_DARK_BOOST: f32 = 0.12;

/// Hard ceiling on the adaptive threshold so very saturated darks
/// (e.g. a vivid dark-teal colour) are still classified as chromatic.
const ACHROMATIC_MAX: f32 = 0.18;

// ─────────────────────────────────────────────────────────────────────────────
// PUBLIC API
// ─────────────────────────────────────────────────────────────────────────────

/// Recolor `input` so its colors match the noctalia `theme`.
///
/// Works on **any** image — no hardcoded source colors.
///
/// # Algorithm (three stages)
///
/// ## Stage 1 — Cluster
/// Sample up to MAX_SAMPLES pixels and run k-means in Oklch space to find K
/// dominant colors. Oklch is perceptually uniform: cluster boundaries align
/// with what the eye sees as visually distinct color regions.
///
/// ## Stage 2 — Match (lightness-adaptive hue-family matching)
/// Partition clusters and theme colors into two groups using a
/// **lightness-adaptive achromatic threshold**:
///
///   threshold(L) = clamp(ACHROMATIC_BASE + (1 - L) * ACHROMATIC_DARK_BOOST,
///                        ACHROMATIC_BASE, ACHROMATIC_MAX)
///
/// Dark pixels get a wider achromatic band because low-L chroma is dominated
/// by quantization noise — a near-black pixel with chroma=0.07 has no
/// meaningful hue and must not be sent to the chromatic (hue-matching) path.
/// Bright pixels get a tighter band so genuinely colourful mid-tones are
/// still classified chromatic and hue-matched correctly.
///
///   **Achromatic clusters** → matched to achromatic theme colors by lightness
///   rank (darkest → darkest). These are backgrounds, shadows, outlines.
///
///   **Chromatic clusters** → matched to chromatic theme colors by nearest
///   circular hue angle. Yellow moon (~100°) always wins the lime green slot
///   (~135°); purple owl (~290°) always wins the purple slot. The swap that
///   plagued earlier versions cannot happen.
///
/// ## Stage 3 — Transfer
/// Inverse-power-distance weighted blend across all cluster→target pairs.
/// Lightness is ratio-preserved (orig_L / src_L × tgt_L) so gradients remain
/// smooth. No hard chroma clamp — the adaptive matching in Stage 2 is the
/// correct place to prevent colour bleed, not a blanket Stage 3 suppression
/// that would kill the purple tint in the dark background.
pub fn recolor_wallpaper(input: &RgbImage, theme: &NoctaliaTheme) -> RgbImage {
    // ── Stage 1: k-means clustering ───────────────────────────────────────
    let samples = sample_pixels(input, MAX_SAMPLES);
    let clusters = kmeans(&samples, K, KMEANS_ITER);

    // ── Stage 2: hue-family matching ──────────────────────────────────────
    let theme_colors: Vec<Oklch<f32>> = theme
        .palette()
        .into_iter()
        .map(rgb_arr_to_oklch)
        .collect();

    let mappings = match_clusters_to_theme(&clusters, &theme_colors);

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
// STAGE 2 — LIGHTNESS-ADAPTIVE HUE-FAMILY MATCHING
// ─────────────────────────────────────────────────────────────────────────────

/// Lightness-adaptive achromatic test.
///
/// The threshold widens as L decreases because near-black chroma readings are
/// noise-dominated. A cluster at L=0.05 with chroma=0.09 is almost certainly
/// just a dark background — its hue cannot be trusted. A cluster at L=0.7
/// with chroma=0.09 is genuinely slightly-coloured and should be hue-matched.
fn is_achromatic(c: &Oklch<f32>) -> bool {
    let threshold = (ACHROMATIC_BASE + (1.0 - c.l) * ACHROMATIC_DARK_BOOST)
        .min(ACHROMATIC_MAX);
    c.chroma < threshold
}

/// Pair each source cluster with the most appropriate theme color.
///
/// - Achromatic clusters → achromatic theme colors, by lightness rank.
/// - Chromatic clusters  → chromatic theme colors, by nearest hue angle.
///
/// Falls back gracefully when one side is empty (greyscale image, or a theme
/// with no achromatic slots).
fn match_clusters_to_theme(
    clusters: &[Oklch<f32>],
    theme_colors: &[Oklch<f32>],
) -> Vec<(Oklch<f32>, Oklch<f32>)> {
    let mut achromatic_clusters: Vec<Oklch<f32>> = clusters
        .iter()
        .filter(|c| is_achromatic(c))
        .copied()
        .collect();
    let chromatic_clusters: Vec<Oklch<f32>> = clusters
        .iter()
        .filter(|c| !is_achromatic(c))
        .copied()
        .collect();

    let mut achromatic_theme: Vec<Oklch<f32>> = theme_colors
        .iter()
        .filter(|c| is_achromatic(c))
        .copied()
        .collect();
    let chromatic_theme: Vec<Oklch<f32>> = theme_colors
        .iter()
        .filter(|c| !is_achromatic(c))
        .copied()
        .collect();

    // Sort achromatic by lightness for rank-pairing
    achromatic_clusters.sort_by(|a, b| a.l.partial_cmp(&b.l).unwrap());
    achromatic_theme.sort_by(|a, b| a.l.partial_cmp(&b.l).unwrap());

    let mut mappings: Vec<(Oklch<f32>, Oklch<f32>)> = Vec::with_capacity(clusters.len());

    // ── Achromatic: pair by lightness rank ───────────────────────────────
    let ath_len = achromatic_theme.len();
    for (i, &src) in achromatic_clusters.iter().enumerate() {
        let target = if ath_len == 0 {
            *theme_colors
                .iter()
                .min_by(|a, b| a.l.partial_cmp(&b.l).unwrap())
                .unwrap_or(&theme_colors[0])
        } else {
            let t = if achromatic_clusters.len() == 1 {
                ath_len / 2
            } else {
                (i * (ath_len - 1)) / (achromatic_clusters.len() - 1)
            };
            achromatic_theme[t.min(ath_len - 1)]
        };
        mappings.push((src, target));
    }

    // ── Chromatic: pair by nearest hue ───────────────────────────────────
    // Many-to-one is fine: two source hues both near "green" both map to the
    // green theme slot. Each theme color can receive multiple clusters.
    for src in chromatic_clusters {
        let target = if chromatic_theme.is_empty() {
            *theme_colors
                .iter()
                .min_by(|a, b| {
                    (a.l - src.l).abs().partial_cmp(&(b.l - src.l).abs()).unwrap()
                })
                .unwrap_or(&theme_colors[0])
        } else {
            *chromatic_theme
                .iter()
                .min_by(|a, b| {
                    hue_dist(src.hue, a.hue)
                        .partial_cmp(&hue_dist(src.hue, b.hue))
                        .unwrap()
                })
                .unwrap()
        };
        mappings.push((src, target));
    }

    mappings
}

/// Circular distance between two hue angles. Result ∈ [0, 180] degrees.
fn hue_dist(a: OklabHue<f32>, b: OklabHue<f32>) -> f32 {
    let diff = (a.into_degrees() - b.into_degrees()).abs() % 360.0;
    if diff > 180.0 { 360.0 - diff } else { diff }
}

// ─────────────────────────────────────────────────────────────────────────────
// STAGE 1 — SAMPLING
// ─────────────────────────────────────────────────────────────────────────────

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
// STAGE 1 — K-MEANS IN OKLCH SPACE
// ─────────────────────────────────────────────────────────────────────────────

/// Lloyd's k-means. Centroids computed in cartesian Oklab (a, b) to avoid
/// hue-angle averaging artifacts near the 0°/360° wrap boundary.
fn kmeans(points: &[Oklch<f32>], k: usize, iters: usize) -> Vec<Oklch<f32>> {
    if points.is_empty() || k == 0 {
        return vec![];
    }

    let mut centroids = kmeans_plus_plus_init(points, k);

    for _ in 0..iters {
        let assignments: Vec<usize> = points
            .iter()
            .map(|p| nearest_centroid(p, &centroids))
            .collect();

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

/// KMeans++ init — deterministic, no RNG.
/// First centroid at 25th-percentile index; each next one is the point
/// farthest from all existing centroids.
fn kmeans_plus_plus_init(points: &[Oklch<f32>], k: usize) -> Vec<Oklch<f32>> {
    let mut centroids = Vec::with_capacity(k);
    centroids.push(points[points.len() / 4]);

    for _ in 1..k {
        let distances: Vec<f32> = points
            .iter()
            .map(|p| {
                centroids
                    .iter()
                    .map(|c| oklch_distance(p, c))
                    .fold(f32::MAX, f32::min)
            })
            .collect();

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
// STAGE 3 — PER-PIXEL TRANSFER
// ─────────────────────────────────────────────────────────────────────────────

/// Inverse-power-distance weighted blend across all cluster→target mappings.
///
/// weight_i = 1 / dist(pixel, source_cluster_i)^SHARPNESS
///
/// Lightness: ratio-preserved (orig_L / src_L × tgt_L) so gradients stay
/// smooth — no banding, no flat slabs.
///
/// Hue + chroma: blended in cartesian (a, b) Oklab space to avoid wrap
/// artifacts near 0°/360°.
///
/// No hard dark-chroma clamp here — colour bleed prevention lives in Stage 2
/// (adaptive achromatic classification) where it belongs. A Stage 3 clamp
/// would strip the legitimate purple tint from the dark background.
fn transfer_pixel(orig: Oklch<f32>, mappings: &[(Oklch<f32>, Oklch<f32>)]) -> Rgb<u8> {
    let mut total_w = 0.0f32;
    let mut out_l   = 0.0f32;
    let mut out_a   = 0.0f32;
    let mut out_b   = 0.0f32;

    for (src, tgt) in mappings {
        let dist = oklch_distance(&orig, src).max(EPSILON);
        let w = 1.0 / dist.powf(SHARPNESS);

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
    let a = out_a / total_w;
    let b = out_b / total_w;
    let final_chroma = (a * a + b * b).sqrt().clamp(0.0, 0.5);
    let final_hue = OklabHue::from_degrees(b.atan2(a).to_degrees());

    oklch_to_rgb(&Oklch {
        l: final_l,
        chroma: final_chroma,
        hue: final_hue,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// HELPERS
// ─────────────────────────────────────────────────────────────────────────────

/// Perceptually uniform distance via cartesian Oklab conversion.
fn oklch_distance(a: &Oklch<f32>, b: &Oklch<f32>) -> f32 {
    let (aa, ab) = hue_to_ab(a.chroma, a.hue);
    let (ba, bb) = hue_to_ab(b.chroma, b.hue);
    let dl = a.l - b.l;
    let da = aa - ba;
    let db = ab - bb;
    (dl * dl + da * da + db * db).sqrt()
}

/// Polar → cartesian Oklab. Required for correct centroid averaging and
/// colour blending — averaging hue angles directly breaks near 0°/360°.
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
    fn test_recolor_runs() {
        let mut img = RgbImage::new(64, 64);
        for p in img.pixels_mut() { *p = Rgb([128, 64, 192]); }
        assert_eq!(recolor_wallpaper(&img, &test_theme()).dimensions(), (64, 64));
    }

    #[test]
    fn test_gradient_stays_monotone() {
        let mut img = RgbImage::new(256, 1);
        for x in 0..256u32 {
            img.put_pixel(x, 0, Rgb([x as u8, x as u8, x as u8]));
        }
        let result = recolor_wallpaper(&img, &test_theme());
        let left  = rgb_to_oklch(result.get_pixel(10, 0));
        let right = rgb_to_oklch(result.get_pixel(245, 0));
        assert!(left.l < right.l,
            "gradient must stay monotone: L_left={} L_right={}", left.l, right.l);
    }

    #[test]
    fn test_kmeans_count() {
        let samples: Vec<Oklch<f32>> = (0..200)
            .map(|i| Oklch {
                l: i as f32 / 200.0,
                chroma: 0.1,
                hue: OklabHue::from_degrees(120.0),
            })
            .collect();
        assert_eq!(kmeans(&samples, K, KMEANS_ITER).len(), K);
    }

    #[test]
    fn test_dark_stays_dark() {
        let mut img = RgbImage::new(1, 1);
        img.put_pixel(0, 0, Rgb([5, 5, 5]));
        let out = rgb_to_oklch(recolor_wallpaper(&img, &test_theme()).get_pixel(0, 0));
        assert!(out.l < 0.25, "near-black should stay dark, got L={}", out.l);
    }

    #[test]
    fn test_light_stays_light() {
        let mut img = RgbImage::new(1, 1);
        img.put_pixel(0, 0, Rgb([250, 250, 250]));
        let out = rgb_to_oklch(recolor_wallpaper(&img, &test_theme()).get_pixel(0, 0));
        assert!(out.l > 0.7, "near-white should stay light, got L={}", out.l);
    }

    #[test]
    fn test_adaptive_threshold_classifies_dark_as_achromatic() {
        // A dark pixel with moderate chroma should be achromatic — its hue
        // is noise. A bright pixel with the same chroma should be chromatic.
        let dark   = Oklch { l: 0.08, chroma: 0.09, hue: OklabHue::from_degrees(120.0) };
        let bright = Oklch { l: 0.70, chroma: 0.09, hue: OklabHue::from_degrees(120.0) };
        assert!(is_achromatic(&dark),   "dark pixel with chroma=0.09 should be achromatic");
        assert!(!is_achromatic(&bright), "bright pixel with chroma=0.09 should be chromatic");
    }

    #[test]
    fn test_vivid_dark_stays_chromatic() {
        // A vivid dark-teal (high chroma even at low L) should still be chromatic.
        let vivid_dark = Oklch { l: 0.15, chroma: 0.22, hue: OklabHue::from_degrees(200.0) };
        assert!(!is_achromatic(&vivid_dark),
            "vivid dark colour should remain chromatic, chroma={}", vivid_dark.chroma);
    }

    #[test]
    fn test_yellow_and_purple_map_to_different_hues() {
        let yellow = Oklch { l: 0.88, chroma: 0.18, hue: OklabHue::from_degrees(100.0) };
        let purple = Oklch { l: 0.55, chroma: 0.15, hue: OklabHue::from_degrees(290.0) };
        let theme = vec![
            Oklch { l: 0.12, chroma: 0.02, hue: OklabHue::from_degrees(290.0) },
            Oklch { l: 0.25, chroma: 0.02, hue: OklabHue::from_degrees(290.0) },
            Oklch { l: 0.55, chroma: 0.17, hue: OklabHue::from_degrees(290.0) },
            Oklch { l: 0.78, chroma: 0.22, hue: OklabHue::from_degrees(135.0) },
            Oklch { l: 0.85, chroma: 0.04, hue: OklabHue::from_degrees(290.0) },
            Oklch { l: 0.92, chroma: 0.02, hue: OklabHue::from_degrees(290.0) },
            Oklch { l: 0.50, chroma: 0.20, hue: OklabHue::from_degrees(25.0)  },
        ];
        let mappings = match_clusters_to_theme(&[yellow, purple], &theme);
        let tgt_y = mappings.iter()
            .find(|(s, _)| (s.hue.into_degrees() - 100.0).abs() < 5.0).unwrap().1;
        let tgt_p = mappings.iter()
            .find(|(s, _)| (s.hue.into_degrees() - 290.0).abs() < 5.0).unwrap().1;
        let gap = hue_dist(tgt_y.hue, tgt_p.hue);
        assert!(gap > 30.0,
            "yellow and purple should hit different theme hues, gap={}", gap);
    }
}
