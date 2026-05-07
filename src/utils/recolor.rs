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

/// How much to weight lightness vs chroma+hue in the matching cost.
/// 0.5 = equal weight. Higher = lightness dominates matching.
const L_WEIGHT: f32 = 0.6;
const CH_WEIGHT: f32 = 0.4;

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
/// ## Stage 2 — Hungarian optimal matching
/// Build a K×N cost matrix where cost[i][j] is a weighted distance between
/// cluster i and theme color j, combining:
///   - Lightness distance (L axis): preserves dark→dark, light→light
///   - Chroma+hue distance (a,b axes): preserves colour family
///
/// Run the Hungarian algorithm to find the globally optimal one-to-one
/// assignment that minimises total cost. This is the key fix over all
/// previous approaches: no thresholds, no axis prioritization, no
/// heuristics — just the mathematically optimal pairing.
///
/// When K > palette size, extra clusters are assigned to the nearest
/// already-assigned theme color (many-to-one allowed after optimal pairing).
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

    let mappings = hungarian_match(&clusters, &theme_colors);

    let (width, height) = input.dimensions();
    let mut output = RgbImage::new(width, height);
    for (x, y, pixel) in input.enumerate_pixels() {
        let orig = rgb_to_oklch(pixel);
        output.put_pixel(x, y, transfer_pixel(orig, &mappings));
    }
    output
}

// ─────────────────────────────────────────────────────────────────────────────
// STAGE 2 — HUNGARIAN OPTIMAL MATCHING
// ─────────────────────────────────────────────────────────────────────────────

/// Weighted matching cost between a source cluster and a theme color.
///
/// Combines lightness distance and chroma+hue (a,b) distance with separate
/// weights so both axes contribute. This prevents:
///   - Dark background matching to near-white (L distance too high)
///   - Dark background matching to lime green (CH distance too high — green
///     has high chroma, background has near-zero chroma)
///   - Yellow moon matching to purple owl slot (hue distance too high)
fn matching_cost(src: &Oklch<f32>, tgt: &Oklch<f32>) -> f32 {
    let l_dist = (src.l - tgt.l).abs();
    let (sa, sb) = hue_to_ab(src.chroma, src.hue);
    let (ta, tb) = hue_to_ab(tgt.chroma, tgt.hue);
    let ch_dist = ((sa - ta).powi(2) + (sb - tb).powi(2)).sqrt();
    L_WEIGHT * l_dist + CH_WEIGHT * ch_dist
}

/// Run the Hungarian algorithm to find the optimal one-to-one assignment
/// of clusters to theme colors, minimizing total weighted matching cost.
///
/// Implementation: Munkres / Hungarian algorithm on an n×m cost matrix
/// where n = clusters.len(), m = theme_colors.len().
/// We pad to square if needed, solve, then discard dummy assignments.
///
/// After optimal matching, any clusters that didn't get a unique theme slot
/// (when K > palette size) are assigned to the nearest theme color by cost.
fn hungarian_match(
    clusters: &[Oklch<f32>],
    theme_colors: &[Oklch<f32>],
) -> Vec<(Oklch<f32>, Oklch<f32>)> {
    let nc = clusters.len();
    let nt = theme_colors.len();
    let n = nc.max(nt); // square dimension for the algorithm

    // Build cost matrix, padded with zeros for dummy rows/cols
    let mut cost = vec![vec![0.0f32; n]; n];
    for i in 0..nc {
        for j in 0..nt {
            cost[i][j] = matching_cost(&clusters[i], &theme_colors[j]);
        }
    }

    // Run Hungarian algorithm → assignment[i] = j means cluster i → theme j
    let assignment = munkres(&cost, n);

    // Build mappings. For clusters mapped to dummy cols (j >= nt), fall back
    // to nearest-cost theme color.
    let mut mappings = Vec::with_capacity(nc);
    for i in 0..nc {
        let j = assignment[i];
        let theme_color = if j < nt {
            theme_colors[j]
        } else {
            // Dummy assignment — find nearest real theme color by cost
            *theme_colors
                .iter()
                .min_by(|a, b| {
                    matching_cost(&clusters[i], a)
                        .partial_cmp(&matching_cost(&clusters[i], b))
                        .unwrap()
                })
                .unwrap()
        };
        mappings.push((clusters[i], theme_color));
    }

    mappings
}

/// Munkres (Hungarian) algorithm on an n×n cost matrix.
/// Returns assignment[i] = j such that total cost is minimized.
///
/// Classic O(n³) implementation:
/// 1. Row reduction: subtract row minimum from each row
/// 2. Col reduction: subtract col minimum from each col
/// 3. Cover zeros with minimum lines; if n lines needed → done
/// 4. Otherwise find minimum uncovered value, subtract from uncovered,
///    add to doubly-covered, repeat from step 3
fn munkres(cost: &[Vec<f32>], n: usize) -> Vec<usize> {
    let mut matrix: Vec<Vec<f32>> = cost.to_vec();

    // Step 1: row reduction
    for row in matrix.iter_mut() {
        let min = row.iter().cloned().fold(f32::MAX, f32::min);
        for v in row.iter_mut() { *v -= min; }
    }

    // Step 2: col reduction
    for j in 0..n {
        let min = (0..n).map(|i| matrix[i][j]).fold(f32::MAX, f32::min);
        for i in 0..n { matrix[i][j] -= min; }
    }

    let mut row_covered = vec![false; n];
    let mut col_covered = vec![false; n];
    // starred[i][j] = true if (i,j) is a starred zero
    let mut starred = vec![vec![false; n]; n];
    // primed[i][j] = true if (i,j) is a primed zero
    let mut primed = vec![vec![false; n]; n];

    // Step 3: star zeros
    for i in 0..n {
        for j in 0..n {
            if matrix[i][j].abs() < 1e-9 && !row_covered[i] && !col_covered[j] {
                starred[i][j] = true;
                row_covered[i] = true;
                col_covered[j] = true;
            }
        }
    }
    row_covered = vec![false; n];
    col_covered = vec![false; n];

    loop {
        // Cover columns with starred zeros
        for j in 0..n {
            if (0..n).any(|i| starred[i][j]) {
                col_covered[j] = true;
            }
        }

        // If all cols covered → we have a complete assignment
        if col_covered.iter().filter(|&&c| c).count() == n {
            break;
        }

        // Find an uncovered zero and prime it
        'outer: loop {
            let mut found_zero = None;
            'find: for i in 0..n {
                for j in 0..n {
                    if matrix[i][j].abs() < 1e-9 && !row_covered[i] && !col_covered[j] {
                        found_zero = Some((i, j));
                        break 'find;
                    }
                }
            }

            match found_zero {
                None => {
                    // No uncovered zero — adjust matrix
                    let min_uncovered = (0..n)
                        .flat_map(|i| (0..n).map(move |j| (i, j)))
                        .filter(|&(i, j)| !row_covered[i] && !col_covered[j])
                        .map(|(i, j)| matrix[i][j])
                        .fold(f32::MAX, f32::min);

                    for i in 0..n {
                        for j in 0..n {
                            if row_covered[i] { matrix[i][j] += min_uncovered; }
                            if !col_covered[j] { matrix[i][j] -= min_uncovered; }
                        }
                    }
                    // (continue loop to find uncovered zero again)
                }
                Some((pi, pj)) => {
                    primed[pi][pj] = true;

                    // Is there a starred zero in this row?
                    let star_col = (0..n).find(|&j| starred[pi][j]);
                    match star_col {
                        Some(sj) => {
                            // Cover this row, uncover the starred column
                            row_covered[pi] = true;
                            col_covered[sj] = false;
                            // continue looking for uncovered zeros
                        }
                        None => {
                            // Augment path starting at (pi, pj)
                            let mut path = vec![(pi, pj)];
                            loop {
                                let (_, last_j) = *path.last().unwrap();
                                // Find starred zero in this column
                                let star_row = (0..n).find(|&i| starred[i][last_j]);
                                match star_row {
                                    None => break,
                                    Some(sr) => {
                                        path.push((sr, last_j));
                                        // Find primed zero in this row
                                        let prime_col = (0..n).find(|&j| primed[sr][j]).unwrap();
                                        path.push((sr, prime_col));
                                    }
                                }
                            }
                            // Flip stars along path
                            for &(i, j) in &path {
                                starred[i][j] = !starred[i][j];
                            }
                            // Clear primes and covers
                            primed = vec![vec![false; n]; n];
                            row_covered = vec![false; n];
                            col_covered = vec![false; n];
                            break 'outer;
                        }
                    }
                }
            }
        }
    }

    // Extract assignment from starred zeros
    let mut assignment = vec![0usize; n];
    for i in 0..n {
        for j in 0..n {
            if starred[i][j] {
                assignment[i] = j;
            }
        }
    }
    assignment
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
    fn test_munkres_identity() {
        // On a diagonal cost matrix, Hungarian should assign i→i
        let n = 4;
        let mut cost = vec![vec![1.0f32; n]; n];
        for i in 0..n { cost[i][i] = 0.0; }
        let assignment = munkres(&cost, n);
        for i in 0..n {
            assert_eq!(assignment[i], i, "identity matrix: cluster {} should map to theme {}", i, i);
        }
    }

    #[test]
    fn test_dark_bg_maps_to_dark_theme() {
        // Dark background cluster must map to dark theme color, not bright green
        let dark_bg    = Oklch { l: 0.08, chroma: 0.01, hue: OklabHue::from_degrees(280.0) };
        let lime_green = Oklch { l: 0.80, chroma: 0.22, hue: OklabHue::from_degrees(135.0) };
        let theme = vec![
            Oklch { l: 0.08, chroma: 0.02, hue: OklabHue::from_degrees(280.0) }, // dark bg
            Oklch { l: 0.25, chroma: 0.05, hue: OklabHue::from_degrees(280.0) }, // dark surface
            Oklch { l: 0.45, chroma: 0.10, hue: OklabHue::from_degrees(280.0) }, // mid purple
            Oklch { l: 0.60, chroma: 0.18, hue: OklabHue::from_degrees(280.0) }, // purple
            Oklch { l: 0.80, chroma: 0.22, hue: OklabHue::from_degrees(135.0) }, // green
            Oklch { l: 0.88, chroma: 0.04, hue: OklabHue::from_degrees(280.0) }, // light
            Oklch { l: 0.95, chroma: 0.01, hue: OklabHue::from_degrees(280.0) }, // near-white
        ];
        let mappings = hungarian_match(&[dark_bg, lime_green], &theme);
        let (_, bg_target) = mappings.iter().find(|(s, _)| s.l < 0.15).unwrap();
        assert!(bg_target.l < 0.4,
            "dark background should map to a dark theme color, got L={}", bg_target.l);
        assert!(bg_target.chroma < 0.10,
            "dark background should map to low-chroma theme color, got chroma={}", bg_target.chroma);
    }

    #[test]
    fn test_yellow_and_purple_separate() {
        let yellow = Oklch { l: 0.88, chroma: 0.18, hue: OklabHue::from_degrees(100.0) };
        let purple = Oklch { l: 0.55, chroma: 0.15, hue: OklabHue::from_degrees(290.0) };
        let theme = vec![
            Oklch { l: 0.10, chroma: 0.02, hue: OklabHue::from_degrees(290.0) },
            Oklch { l: 0.25, chroma: 0.02, hue: OklabHue::from_degrees(290.0) },
            Oklch { l: 0.55, chroma: 0.17, hue: OklabHue::from_degrees(290.0) },
            Oklch { l: 0.78, chroma: 0.22, hue: OklabHue::from_degrees(135.0) },
            Oklch { l: 0.85, chroma: 0.04, hue: OklabHue::from_degrees(290.0) },
            Oklch { l: 0.92, chroma: 0.02, hue: OklabHue::from_degrees(290.0) },
            Oklch { l: 0.50, chroma: 0.20, hue: OklabHue::from_degrees(25.0)  },
        ];
        let mappings = hungarian_match(&[yellow, purple], &theme);
        let tgt_y = mappings.iter().find(|(s, _)| (s.hue.into_degrees() - 100.0).abs() < 5.0).unwrap().1;
        let tgt_p = mappings.iter().find(|(s, _)| (s.hue.into_degrees() - 290.0).abs() < 5.0).unwrap().1;
        let gap = {
            let diff = (tgt_y.hue.into_degrees() - tgt_p.hue.into_degrees()).abs() % 360.0;
            if diff > 180.0 { 360.0 - diff } else { diff }
        };
        assert!(gap > 30.0, "yellow and purple should hit different theme hues, gap={}", gap);
    }

    #[test]
    fn test_kmeans_count() {
        let samples: Vec<Oklch<f32>> = (0..200)
            .map(|i| Oklch { l: i as f32 / 200.0, chroma: 0.1, hue: OklabHue::from_degrees(120.0) })
            .collect();
        assert_eq!(kmeans(&samples, K, KMEANS_ITER).len(), K);
    }
}
