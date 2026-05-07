use image::{Rgb, RgbImage};
use palette::{FromColor, IntoColor, Oklch, OklabHue, Srgb};

use akspraypaint::NoctaliaTheme;

// ─────────────────────────────────────────────────────────────────────────────
// CONSTANTS
// ─────────────────────────────────────────────────────────────────────────────

const K: usize = 7;
const KMEANS_ITER: usize = 12;
const MAX_SAMPLES: usize = 40_000;
const SHARPNESS: f32 = 8.0;
const EPSILON: f32 = 1e-6;

// ─────────────────────────────────────────────────────────────────────────────
// PUBLIC API
// ─────────────────────────────────────────────────────────────────────────────

/// Recolor `input` so its colors match the noctalia `theme`.
///
/// Works on **any** image — no hardcoded source colors.
///
/// # Algorithm
///
/// ## Stage 1 — Cluster
/// K-means in Oklch space finds K dominant colors in the source image.
///
/// ## Stage 2 — Match by chroma rank, then hue
/// This is the key insight: **lightness is never used for matching**.
///
/// Sort both clusters and theme colors by chroma (low → high).
/// Pair them by chroma rank: the least-chromatic cluster maps to the
/// least-chromatic theme color, the most-chromatic to the most-chromatic.
///
/// Within each chroma rank position, if there are ties or ambiguity, hue
/// is used to break them — the cluster whose hue is closest to a theme
/// color's hue wins that slot.
///
/// Why chroma-first?
///   - The dark background (chroma ≈ 0.00) and the lime green accent
///     (chroma ≈ 0.22) are maximally separated in chroma space.
///     They can never compete for the same theme slot.
///   - Lightness is unreliable for matching: a dark purple and a dark
///     background both have low L, but one is chromatic and one is not.
///   - Chroma is the correct axis: it directly encodes "how colourful is
///     this region" which is exactly what determines which theme slot it
///     belongs in.
///
/// ## Stage 3 — Transfer
/// Inverse-power-distance weighted blend. Lightness ratio-preserved.
pub fn recolor_wallpaper(input: &RgbImage, theme: &NoctaliaTheme) -> RgbImage {
    let samples = sample_pixels(input, MAX_SAMPLES);
    let clusters = kmeans(&samples, K, KMEANS_ITER);

    let theme_colors: Vec<Oklch<f32>> = theme
        .palette()
        .into_iter()
        .map(rgb_arr_to_oklch)
        .collect();

    let mappings = match_by_chroma_then_hue(&clusters, &theme_colors);

    let (width, height) = input.dimensions();
    let mut output = RgbImage::new(width, height);

    for (x, y, pixel) in input.enumerate_pixels() {
        let orig = rgb_to_oklch(pixel);
        output.put_pixel(x, y, transfer_pixel(orig, &mappings));
    }

    output
}

// ─────────────────────────────────────────────────────────────────────────────
// STAGE 2 — MATCH BY CHROMA RANK THEN HUE
// ─────────────────────────────────────────────────────────────────────────────

/// Match source clusters to theme colors using chroma as the primary axis
/// and hue as a tiebreaker.
///
/// The matching works in two passes:
///
/// **Pass 1 — Chroma bucketing**
/// Divide the chroma range [0, max_chroma] into equal buckets, one per
/// theme color (sorted by chroma). Each cluster is assigned to the bucket
/// matching its chroma rank. This guarantees near-zero-chroma clusters
/// (backgrounds) never compete with high-chroma clusters (accents) for
/// the same theme slot.
///
/// **Pass 2 — Hue refinement within bucket**
/// When multiple clusters land in the same chroma bucket, assign each to
/// the theme color in that bucket whose hue is closest. This handles the
/// case where e.g. both the owl and a mid-tone outline land in the "medium
/// chroma" bucket — they get sorted to their nearest hue within that group.
fn match_by_chroma_then_hue(
    clusters: &[Oklch<f32>],
    theme_colors: &[Oklch<f32>],
) -> Vec<(Oklch<f32>, Oklch<f32>)> {
    // Sort both by chroma ascending
    let mut sorted_clusters = clusters.to_vec();
    let mut sorted_theme = theme_colors.to_vec();
    sorted_clusters.sort_by(|a, b| a.chroma.partial_cmp(&b.chroma).unwrap());
    sorted_theme.sort_by(|a, b| a.chroma.partial_cmp(&b.chroma).unwrap());

    let nc = sorted_clusters.len();
    let nt = sorted_theme.len();

    // For each cluster, find its chroma-rank position in the theme palette.
    // Use fractional indexing so clusters spread evenly across theme slots
    // even when K ≠ palette length.
    let mut mappings: Vec<(Oklch<f32>, Oklch<f32>)> = Vec::with_capacity(nc);

    for (ci, &src) in sorted_clusters.iter().enumerate() {
        // Map cluster index → theme index by chroma rank
        let base_ti = if nc == 1 {
            nt / 2
        } else {
            (ci * (nt - 1)) / (nc - 1)
        };
        let base_ti = base_ti.min(nt - 1);

        // Among the theme colors at adjacent chroma ranks (base_ti ± 1),
        // pick the one whose hue is closest to src.hue.
        // This is the hue refinement step — it prevents yellow and blue-purple
        // from collapsing to the same slot when they share a chroma rank.
        let lo = base_ti.saturating_sub(1);
        let hi = (base_ti + 1).min(nt - 1);

        let best_theme = sorted_theme[lo..=hi]
            .iter()
            .min_by(|a, b| {
                // For achromatic sources (chroma ≈ 0), hue is meaningless —
                // just pick the closest chroma (i.e. stay at base_ti).
                // For chromatic sources, use hue distance.
                if src.chroma < 0.05 {
                    a.chroma.partial_cmp(&b.chroma).unwrap()
                } else {
                    hue_dist(src.hue, a.hue)
                        .partial_cmp(&hue_dist(src.hue, b.hue))
                        .unwrap()
                }
            })
            .copied()
            .unwrap_or(sorted_theme[base_ti]);

        mappings.push((src, best_theme));
    }

    mappings
}

/// Circular hue distance in degrees. Result ∈ [0, 180].
fn hue_dist(a: OklabHue<f32>, b: OklabHue<f32>) -> f32 {
    let diff = (a.into_degrees() - b.into_degrees()).abs() % 360.0;
    if diff > 180.0 { 360.0 - diff } else { diff }
}

// ─────────────────────────────────────────────────────────────────────────────
// STAGE 1 — SAMPLING + K-MEANS
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

fn kmeans(points: &[Oklch<f32>], k: usize, iters: usize) -> Vec<Oklch<f32>> {
    if points.is_empty() || k == 0 { return vec![]; }
    let mut centroids = kmeans_plus_plus_init(points, k);

    for _ in 0..iters {
        let assignments: Vec<usize> = points
            .iter()
            .map(|p| nearest_centroid(p, &centroids))
            .collect();

        let mut sums = vec![[0.0f32; 3]; k];
        let mut counts = vec![0usize; k];

        for (p, &c) in points.iter().zip(assignments.iter()) {
            let (a, b) = hue_to_ab(p.chroma, p.hue);
            sums[c][0] += p.l;
            sums[c][1] += a;
            sums[c][2] += b;
            counts[c] += 1;
        }

        for (i, s) in sums.iter().enumerate() {
            if counts[i] == 0 { continue; }
            let n = counts[i] as f32;
            let l = s[0] / n;
            let a = s[1] / n;
            let b = s[2] / n;
            let chroma = (a * a + b * b).sqrt();
            let hue = OklabHue::from_degrees(b.atan2(a).to_degrees());
            centroids[i] = Oklch { l, chroma, hue };
        }
    }
    centroids
}

fn kmeans_plus_plus_init(points: &[Oklch<f32>], k: usize) -> Vec<Oklch<f32>> {
    let mut centroids = Vec::with_capacity(k);
    centroids.push(points[points.len() / 4]);

    for _ in 1..k {
        let distances: Vec<f32> = points
            .iter()
            .map(|p| centroids.iter().map(|c| oklch_distance(p, c)).fold(f32::MAX, f32::min))
            .collect();
        let next = distances
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
            .map(|(i, _)| i)
            .unwrap_or(0);
        centroids.push(points[next]);
    }
    centroids
}

fn nearest_centroid(p: &Oklch<f32>, centroids: &[Oklch<f32>]) -> usize {
    centroids
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| oklch_distance(p, a).partial_cmp(&oklch_distance(p, b)).unwrap())
        .map(|(i, _)| i)
        .unwrap_or(0)
}

// ─────────────────────────────────────────────────────────────────────────────
// STAGE 3 — PER-PIXEL TRANSFER
// ─────────────────────────────────────────────────────────────────────────────

/// Inverse-power-distance weighted blend across all cluster→target mappings.
/// Lightness is ratio-preserved; hue+chroma blended in cartesian (a,b) space.
fn transfer_pixel(orig: Oklch<f32>, mappings: &[(Oklch<f32>, Oklch<f32>)]) -> Rgb<u8> {
    let mut total_w = 0.0f32;
    let mut out_l   = 0.0f32;
    let mut out_a   = 0.0f32;
    let mut out_b   = 0.0f32;

    for (src, tgt) in mappings {
        let dist = oklch_distance(&orig, src).max(EPSILON);
        let w = 1.0 / dist.powf(SHARPNESS);

        let l_ratio = if src.l > 0.01 { (orig.l / src.l).clamp(0.5, 2.0) } else { 1.0 };
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

    oklch_to_rgb(&Oklch { l: final_l, chroma: final_chroma, hue: final_hue })
}

// ─────────────────────────────────────────────────────────────────────────────
// HELPERS
// ─────────────────────────────────────────────────────────────────────────────

fn oklch_distance(a: &Oklch<f32>, b: &Oklch<f32>) -> f32 {
    let (aa, ab) = hue_to_ab(a.chroma, a.hue);
    let (ba, bb) = hue_to_ab(b.chroma, b.hue);
    let dl = a.l - b.l;
    let da = aa - ba;
    let db = ab - bb;
    (dl * dl + da * da + db * db).sqrt()
}

fn hue_to_ab(chroma: f32, hue: OklabHue<f32>) -> (f32, f32) {
    let rad = hue.into_radians();
    (chroma * rad.cos(), chroma * rad.sin())
}

fn rgb_arr_to_oklch(arr: [u8; 3]) -> Oklch<f32> { rgb_to_oklch(&Rgb(arr)) }

fn rgb_to_oklch(p: &Rgb<u8>) -> Oklch<f32> {
    let srgb = Srgb::new(p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0);
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
    fn test_achromatic_cluster_never_maps_to_high_chroma_theme() {
        // A near-grey cluster (chroma ≈ 0) must map to a low-chroma theme slot,
        // never to the vivid green accent.
        let near_grey  = Oklch { l: 0.10, chroma: 0.01, hue: OklabHue::from_degrees(0.0) };
        let vivid_green = Oklch { l: 0.78, chroma: 0.22, hue: OklabHue::from_degrees(135.0) };
        let theme = vec![
            Oklch { l: 0.10, chroma: 0.01, hue: OklabHue::from_degrees(290.0) }, // dark bg
            Oklch { l: 0.25, chroma: 0.03, hue: OklabHue::from_degrees(290.0) }, // dark surface
            Oklch { l: 0.40, chroma: 0.08, hue: OklabHue::from_degrees(290.0) }, // mid purple
            Oklch { l: 0.55, chroma: 0.17, hue: OklabHue::from_degrees(290.0) }, // purple
            Oklch { l: 0.78, chroma: 0.22, hue: OklabHue::from_degrees(135.0) }, // green
            Oklch { l: 0.85, chroma: 0.05, hue: OklabHue::from_degrees(290.0) }, // light
            Oklch { l: 0.92, chroma: 0.02, hue: OklabHue::from_degrees(290.0) }, // near-white
        ];
        let mappings = match_by_chroma_then_hue(&[near_grey, vivid_green], &theme);
        let (_, grey_target) = mappings.iter()
            .find(|(s, _)| s.chroma < 0.05).unwrap();
        assert!(
            grey_target.chroma < 0.10,
            "achromatic cluster must not map to high-chroma theme color, got chroma={}",
            grey_target.chroma
        );
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
        let mappings = match_by_chroma_then_hue(&[yellow, purple], &theme);
        let tgt_y = mappings.iter()
            .find(|(s, _)| (s.hue.into_degrees() - 100.0).abs() < 5.0).unwrap().1;
        let tgt_p = mappings.iter()
            .find(|(s, _)| (s.hue.into_degrees() - 290.0).abs() < 5.0).unwrap().1;
        assert!(
            hue_dist(tgt_y.hue, tgt_p.hue) > 30.0,
            "yellow and purple should hit different theme hues"
        );
    }

    #[test]
    fn test_kmeans_count() {
        let samples: Vec<Oklch<f32>> = (0..200)
            .map(|i| Oklch { l: i as f32 / 200.0, chroma: 0.1, hue: OklabHue::from_degrees(120.0) })
            .collect();
        assert_eq!(kmeans(&samples, K, KMEANS_ITER).len(), K);
    }
}
