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

/// Sobel gradient magnitude above this → edge pixel.
const EDGE_THRESHOLD: f32 = 0.12;

/// Saliency above this → subject pixel. Below → background.
/// Soft transition between SALIENCY_BG_MAX and SALIENCY_FG_MIN.
const SALIENCY_BG_MAX: f32 = 0.38;
const SALIENCY_FG_MIN: f32 = 0.55;

/// Box blur radius for smoothing saliency before segmentation.
const BLUR_RADIUS: u32 = 12;

/// Center-bias weight in saliency. Subjects tend to be centered.
const CENTER_WEIGHT: f32 = 0.55;

// ─────────────────────────────────────────────────────────────────────────────
// SEGMENT
// ─────────────────────────────────────────────────────────────────────────────

/// Coarse 3-way segment for a pixel.
/// Used to gate which theme color pool each pixel can draw from.
#[derive(Clone, Copy, Debug)]
enum Segment {
    Background, // → only dark/achromatic theme colors
    Edge,       // → on_surface theme color
    Subject,    // → chromatic theme colors
}

// ─────────────────────────────────────────────────────────────────────────────
// PUBLIC API
// ─────────────────────────────────────────────────────────────────────────────

/// Recolor `input` so its colors match the noctalia `theme`.
///
/// Works on any image — no hardcoded source colors.
///
/// # Algorithm — three passes
///
/// ## Pass 1 — CV segmentation
/// Classify every pixel as Background, Edge, or Subject using:
///   - Sobel edge detection on perceptual lightness (Oklch L)
///   - Frequency-tuned saliency (contrast × center-proximity)
///   - Box blur to smooth the saliency map
///
/// This produces a soft 3-way mask. Crucially, dark vignette corners are
/// always Background regardless of their slight colour tint — the CV pass
/// gates them out before any colour matching happens.
///
/// ## Pass 2 — Hue-family matching within each segment
/// Each segment only draws from a restricted pool of theme colors:
///   Background → surface + surface_variant (dark/achromatic only)
///   Edge       → on_surface
///   Subject    → primary + on_primary + on_surface_variant (chromatic)
///
/// Within each pool, k-means clusters from the source image are matched
/// to theme colors by hue proximity. Because the pools are gated by the
/// CV pass, green accent can never bleed into background pixels —
/// they're in separate pools entirely.
///
/// ## Pass 3 — Smooth transfer
/// Inverse-power-distance weighted blend using per-pixel segment weights.
/// Lightness ratio-preserved so gradients remain smooth.
pub fn recolor_wallpaper(input: &RgbImage, theme: &NoctaliaTheme) -> RgbImage {
    let (width, height) = input.dimensions();
    let n = (width * height) as usize;

    // ── Pass 1: CV segmentation ───────────────────────────────────────────
    let lightness   = compute_lightness_map(input);
    let edges       = compute_edge_map(&lightness, width, height);
    let saliency    = compute_saliency_map(&lightness, width, height);
    let saliency_sm = box_blur(&saliency, width, height, BLUR_RADIUS);

    // Per-pixel soft weights for each segment: [bg, edge, subject]
    let seg_weights = compute_segment_weights(&edges, &saliency_sm, n);

    // ── Pass 2: hue-family matching per segment pool ──────────────────────
    let samples  = sample_pixels(input, MAX_SAMPLES);
    let clusters = kmeans(&samples, K, KMEANS_ITER);

    // Background pool: dark/achromatic theme colors
    // Subject pool:    chromatic theme colors
    // Edge:            on_surface (fixed, no matching needed)
    let bg_pool      = background_pool(theme);
    let subject_pool = subject_pool(theme);
    let edge_color   = rgb_arr_to_oklch(theme.on_surface);

    // Match clusters to each pool by hue proximity
    let bg_mappings      = match_by_hue(&clusters, &bg_pool);
    let subject_mappings = match_by_hue(&clusters, &subject_pool);

    // ── Pass 3: per-pixel transfer ────────────────────────────────────────
    let mut output = RgbImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let idx = (y * width + x) as usize;
            let orig = rgb_to_oklch(input.get_pixel(x, y));
            let [w_bg, w_edge, w_subj] = seg_weights[idx];
            let pixel = blend_segments(
                orig,
                w_bg, w_edge, w_subj,
                &bg_mappings,
                &subject_mappings,
                edge_color,
            );
            output.put_pixel(x, y, pixel);
        }
    }
    output
}

// ─────────────────────────────────────────────────────────────────────────────
// THEME COLOR POOLS
// ─────────────────────────────────────────────────────────────────────────────

/// Background pool: the dark/base theme colors.
/// These are the only colors background pixels can ever map to.
/// Green accent is NOT in this list — that's how we prevent bleed.
fn background_pool(theme: &NoctaliaTheme) -> Vec<Oklch<f32>> {
    vec![
        rgb_arr_to_oklch(theme.surface),
        rgb_arr_to_oklch(theme.surface_variant),
    ]
}

/// Subject pool: the foreground/accent theme colors.
/// These are what the owl, moon, and other subject elements map to.
fn subject_pool(theme: &NoctaliaTheme) -> Vec<Oklch<f32>> {
    vec![
        rgb_arr_to_oklch(theme.primary),
        rgb_arr_to_oklch(theme.on_primary),
        rgb_arr_to_oklch(theme.on_surface_variant),
    ]
}

// ─────────────────────────────────────────────────────────────────────────────
// PASS 2 — HUE-FAMILY MATCHING
// ─────────────────────────────────────────────────────────────────────────────

/// For each cluster, find the pool color whose hue is nearest.
/// For achromatic clusters (low chroma), fall back to nearest-L in pool.
/// Returns (cluster, target) pairs for use in transfer_pixel.
fn match_by_hue(
    clusters: &[Oklch<f32>],
    pool: &[Oklch<f32>],
) -> Vec<(Oklch<f32>, Oklch<f32>)> {
    if pool.is_empty() { return vec![]; }

    clusters.iter().map(|&src| {
        let target = if src.chroma < 0.05 {
            // Achromatic: match by lightness
            *pool.iter()
                .min_by(|a, b| {
                    (a.l - src.l).abs()
                        .partial_cmp(&(b.l - src.l).abs())
                        .unwrap()
                })
                .unwrap()
        } else {
            // Chromatic: match by nearest hue
            *pool.iter()
                .min_by(|a, b| {
                    hue_dist(src.hue, a.hue)
                        .partial_cmp(&hue_dist(src.hue, b.hue))
                        .unwrap()
                })
                .unwrap()
        };
        (src, target)
    }).collect()
}

fn hue_dist(a: OklabHue<f32>, b: OklabHue<f32>) -> f32 {
    let diff = (a.into_degrees() - b.into_degrees()).abs() % 360.0;
    if diff > 180.0 { 360.0 - diff } else { diff }
}

// ─────────────────────────────────────────────────────────────────────────────
// PASS 1 — CV SEGMENTATION
// ─────────────────────────────────────────────────────────────────────────────

fn compute_lightness_map(img: &RgbImage) -> Vec<f32> {
    img.pixels().map(|p| rgb_to_oklch(p).l).collect()
}

/// Sobel edge detection on Oklch lightness. Returns normalised [0,1].
fn compute_edge_map(lightness: &[f32], width: u32, height: u32) -> Vec<f32> {
    let w = width as i32;
    let h = height as i32;
    let n = (width * height) as usize;
    let mut edges = vec![0.0f32; n];

    for y in 1..h-1 {
        for x in 1..w-1 {
            let px = |dy: i32, dx: i32| lightness[((y+dy)*w+(x+dx)) as usize];
            let gx = -px(-1,-1) - 2.0*px(0,-1) - px(1,-1)
                     +px(-1, 1) + 2.0*px(0, 1) + px(1, 1);
            let gy = -px(-1,-1) - 2.0*px(-1,0) - px(-1,1)
                     +px( 1,-1) + 2.0*px( 1,0) + px( 1,1);
            edges[(y*w+x) as usize] = (gx*gx + gy*gy).sqrt();
        }
    }
    let max_e = edges.iter().cloned().fold(0.0f32, f32::max).max(1e-6);
    edges.iter_mut().for_each(|e| *e /= max_e);
    edges
}

/// Frequency-tuned saliency: local contrast × center-proximity.
fn compute_saliency_map(lightness: &[f32], width: u32, height: u32) -> Vec<f32> {
    let n = (width * height) as usize;
    let mean_l = lightness.iter().sum::<f32>() / n as f32;
    let cx = width  as f32 / 2.0;
    let cy = height as f32 / 2.0;
    let max_dist = (cx*cx + cy*cy).sqrt();

    let mut sal = vec![0.0f32; n];
    for y in 0..height {
        for x in 0..width {
            let idx = (y*width+x) as usize;
            let contrast = (lightness[idx] - mean_l).abs();
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let center = 1.0 - ((dx*dx+dy*dy).sqrt() / max_dist).clamp(0.0, 1.0);
            sal[idx] = (1.0 - CENTER_WEIGHT) * contrast + CENTER_WEIGHT * center;
        }
    }
    let max_s = sal.iter().cloned().fold(0.0f32, f32::max).max(1e-6);
    sal.iter_mut().for_each(|s| *s /= max_s);
    sal
}

/// Compute per-pixel soft segment weights [bg, edge, subject].
/// Sum of weights = 1.0 per pixel.
fn compute_segment_weights(
    edges: &[f32],
    saliency: &[f32],
    n: usize,
) -> Vec<[f32; 3]> {
    let mut weights = vec![[0.0f32; 3]; n];
    for i in 0..n {
        let e = edges[i];
        let s = saliency[i];

        // Edge membership: soft threshold
        let edge_w = smoothstep(EDGE_THRESHOLD * 0.5, EDGE_THRESHOLD * 1.5, e);
        let non_edge = 1.0 - edge_w;

        // Subject vs background from saliency, on the non-edge portion
        let subj_w = smoothstep(SALIENCY_BG_MAX, SALIENCY_FG_MIN, s) * non_edge;
        let bg_w   = (1.0 - smoothstep(SALIENCY_BG_MAX, SALIENCY_FG_MIN, s)) * non_edge;

        weights[i] = [bg_w, edge_w, subj_w];
    }
    weights
}

fn smoothstep(lo: f32, hi: f32, t: f32) -> f32 {
    let x = ((t - lo) / (hi - lo)).clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

fn box_blur(input: &[f32], width: u32, height: u32, radius: u32) -> Vec<f32> {
    let w = width as usize;
    let h = height as usize;
    let r = radius as usize;
    let mut tmp = vec![0.0f32; w * h];
    let mut out = vec![0.0f32; w * h];

    // Horizontal
    for y in 0..h {
        for x in 0..w {
            let lo = x.saturating_sub(r);
            let hi = (x + r).min(w - 1);
            let n = (hi - lo + 1) as f32;
            tmp[y*w+x] = (lo..=hi).map(|xx| input[y*w+xx]).sum::<f32>() / n;
        }
    }
    // Vertical
    for y in 0..h {
        for x in 0..w {
            let lo = y.saturating_sub(r);
            let hi = (y + r).min(h - 1);
            let n = (hi - lo + 1) as f32;
            out[y*w+x] = (lo..=hi).map(|yy| tmp[yy*w+x]).sum::<f32>() / n;
        }
    }
    out
}

// ─────────────────────────────────────────────────────────────────────────────
// PASS 1 — K-MEANS
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
    let mut centroids = kmeans_init(points, k);

    for _ in 0..iters {
        let assignments: Vec<usize> = points.iter()
            .map(|p| nearest(p, &centroids))
            .collect();

        let mut sums = vec![[0.0f32; 3]; k];
        let mut counts = vec![0usize; k];
        for (p, &c) in points.iter().zip(&assignments) {
            let (a, b) = hue_to_ab(p.chroma, p.hue);
            sums[c][0] += p.l; sums[c][1] += a; sums[c][2] += b;
            counts[c] += 1;
        }
        for i in 0..k {
            if counts[i] == 0 { continue; }
            let n = counts[i] as f32;
            let l = sums[i][0]/n;
            let a = sums[i][1]/n;
            let b = sums[i][2]/n;
            centroids[i] = Oklch {
                l,
                chroma: (a*a+b*b).sqrt(),
                hue: OklabHue::from_degrees(b.atan2(a).to_degrees()),
            };
        }
    }
    centroids
}

fn kmeans_init(points: &[Oklch<f32>], k: usize) -> Vec<Oklch<f32>> {
    let mut c = vec![points[points.len()/4]];
    for _ in 1..k {
        let dists: Vec<f32> = points.iter()
            .map(|p| c.iter().map(|q| oklch_dist(p, q)).fold(f32::MAX, f32::min))
            .collect();
        let next = dists.iter().enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
            .map(|(i, _)| i).unwrap_or(0);
        c.push(points[next]);
    }
    c
}

fn nearest(p: &Oklch<f32>, cs: &[Oklch<f32>]) -> usize {
    cs.iter().enumerate()
        .min_by(|(_, a), (_, b)| oklch_dist(p, a).partial_cmp(&oklch_dist(p, b)).unwrap())
        .map(|(i, _)| i).unwrap_or(0)
}

// ─────────────────────────────────────────────────────────────────────────────
// PASS 3 — PIXEL TRANSFER
// ─────────────────────────────────────────────────────────────────────────────

/// Blend a pixel using its segment weights.
///
/// For each segment (bg, edge, subject):
///   1. Find the best-matching cluster→target mapping for this pixel
///      within that segment's pool (inverse-distance weighted)
///   2. Weight the result by the segment's membership for this pixel
///
/// Lightness is ratio-preserved within each segment's transfer.
/// Final blend is in cartesian (a,b) Oklab space — no hue wrap artifacts.
fn blend_segments(
    orig: Oklch<f32>,
    w_bg: f32,
    w_edge: f32,
    w_subj: f32,
    bg_mappings: &[(Oklch<f32>, Oklch<f32>)],
    subj_mappings: &[(Oklch<f32>, Oklch<f32>)],
    edge_color: Oklch<f32>,
) -> Rgb<u8> {
    let total_seg = w_bg + w_edge + w_subj;
    if total_seg < EPSILON {
        return oklch_to_rgb(&edge_color);
    }

    let mut out_l = 0.0f32;
    let mut out_a = 0.0f32;
    let mut out_b = 0.0f32;

    // Background contribution
    if w_bg > 1e-3 {
        let (tl, ta, tb) = transfer_from_mappings(orig, bg_mappings);
        let wb = w_bg / total_seg;
        out_l += wb * tl;
        out_a += wb * ta;
        out_b += wb * tb;
    }

    // Edge contribution (fixed color, lightness ratio-preserved)
    if w_edge > 1e-3 {
        let we = w_edge / total_seg;
        let l_ratio = if orig.l > 0.01 { (orig.l / edge_color.l.max(0.01)).clamp(0.5, 2.0) } else { 1.0 };
        let mapped_l = (edge_color.l * l_ratio).clamp(0.0, 1.0);
        let (ta, tb) = hue_to_ab(edge_color.chroma, edge_color.hue);
        out_l += we * mapped_l;
        out_a += we * ta;
        out_b += we * tb;
    }

    // Subject contribution
    if w_subj > 1e-3 {
        let (tl, ta, tb) = transfer_from_mappings(orig, subj_mappings);
        let ws = w_subj / total_seg;
        out_l += ws * tl;
        out_a += ws * ta;
        out_b += ws * tb;
    }

    let final_l = out_l.clamp(0.0, 1.0);
    let final_chroma = (out_a*out_a + out_b*out_b).sqrt().clamp(0.0, 0.5);
    let final_hue = OklabHue::from_degrees(out_b.atan2(out_a).to_degrees());
    oklch_to_rgb(&Oklch { l: final_l, chroma: final_chroma, hue: final_hue })
}

/// Inverse-power-distance weighted transfer within one segment's mappings.
/// Returns (mapped_L, a, b) in Oklab cartesian space.
fn transfer_from_mappings(
    orig: Oklch<f32>,
    mappings: &[(Oklch<f32>, Oklch<f32>)],
) -> (f32, f32, f32) {
    if mappings.is_empty() {
        return (orig.l, 0.0, 0.0);
    }

    let mut total_w = 0.0f32;
    let mut out_l = 0.0f32;
    let mut out_a = 0.0f32;
    let mut out_b = 0.0f32;

    for (src, tgt) in mappings {
        let dist = oklch_dist(&orig, src).max(EPSILON);
        let w = 1.0 / dist.powf(SHARPNESS);
        let l_ratio = if src.l > 0.01 { (orig.l / src.l).clamp(0.5, 2.0) } else { 1.0 };
        let mapped_l = (tgt.l * l_ratio).clamp(0.0, 1.0);
        let (ta, tb) = hue_to_ab(tgt.chroma, tgt.hue);
        out_l += w * mapped_l;
        out_a += w * ta;
        out_b += w * tb;
        total_w += w;
    }

    (out_l / total_w, out_a / total_w, out_b / total_w)
}

// ─────────────────────────────────────────────────────────────────────────────
// HELPERS
// ─────────────────────────────────────────────────────────────────────────────

fn oklch_dist(a: &Oklch<f32>, b: &Oklch<f32>) -> f32 {
    let (aa, ab) = hue_to_ab(a.chroma, a.hue);
    let (ba, bb) = hue_to_ab(b.chroma, b.hue);
    let dl = a.l - b.l;
    let da = aa - ba;
    let db = ab - bb;
    (dl*dl + da*da + db*db).sqrt()
}

fn hue_to_ab(chroma: f32, hue: OklabHue<f32>) -> (f32, f32) {
    let r = hue.into_radians();
    (chroma * r.cos(), chroma * r.sin())
}

fn rgb_arr_to_oklch(arr: [u8; 3]) -> Oklch<f32> { rgb_to_oklch(&Rgb(arr)) }

fn rgb_to_oklch(p: &Rgb<u8>) -> Oklch<f32> {
    let s = Srgb::new(p[0] as f32/255.0, p[1] as f32/255.0, p[2] as f32/255.0);
    Oklch::from_color(s.into_linear())
}

fn oklch_to_rgb(c: &Oklch<f32>) -> Rgb<u8> {
    let linear: palette::LinSrgb<f32> = (*c).into_color();
    let s: Srgb<f32> = linear.into_encoding();
    Rgb([
        (s.red   * 255.0).round().clamp(0.0, 255.0) as u8,
        (s.green * 255.0).round().clamp(0.0, 255.0) as u8,
        (s.blue  * 255.0).round().clamp(0.0, 255.0) as u8,
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
            primary:            [100, 50,  200],
            on_primary:         [240, 230, 255],
            surface:            [20,  15,  35 ],
            on_surface:         [180, 170, 200],
            surface_variant:    [45,  35,  65 ],
            on_surface_variant: [160, 150, 185],
            error:              [200, 50,  50 ],
        }
    }

    #[test]
    fn test_recolor_runs() {
        let mut img = RgbImage::new(64, 64);
        for p in img.pixels_mut() { *p = Rgb([128, 64, 192]); }
        assert_eq!(recolor_wallpaper(&img, &test_theme()).dimensions(), (64, 64));
    }

    #[test]
    fn test_dark_corner_stays_dark() {
        // A dark near-black corner pixel must not map to a bright or
        // highly-chromatic color regardless of the theme.
        let mut img = RgbImage::new(128, 128);
        // Dark corners, bright center
        for y in 0..128u32 {
            for x in 0..128u32 {
                let cx = (x as i32 - 64).abs();
                let cy = (y as i32 - 64).abs();
                let v = (255 - (cx + cy).min(120) * 2) as u8;
                img.put_pixel(x, y, Rgb([v/8, v/8, v/8]));
            }
        }
        let result = recolor_wallpaper(&img, &test_theme());
        // Corner pixel should still be dark
        let corner = rgb_to_oklch(result.get_pixel(0, 0));
        assert!(corner.l < 0.35, "corner should stay dark, got L={}", corner.l);
    }

    #[test]
    fn test_edge_map_fires_on_boundary() {
        let w = 64u32; let h = 64u32;
        let mut img = RgbImage::new(w, h);
        for y in 0..h { for x in 0..w {
            img.put_pixel(x, y, Rgb([if x < w/2 { 0 } else { 255 }, 0, 0]));
        }}
        let l = compute_lightness_map(&img);
        let e = compute_edge_map(&l, w, h);
        // Boundary column should have high edge value
        let boundary_edge = e[(32*w+31) as usize];
        assert!(boundary_edge > 0.3, "boundary should have high edge, got {}", boundary_edge);
    }

    #[test]
    fn test_bg_pool_excludes_bright_accent() {
        // Background pool must only contain dark/achromatic colors
        let theme = test_theme();
        let pool = background_pool(&theme);
        for c in &pool {
            assert!(c.l < 0.6, "bg pool color should be dark, got L={}", c.l);
        }
    }

    #[test]
    fn test_subject_pool_contains_chromatic() {
        // Subject pool should contain the primary (most chromatic) color
        let theme = test_theme();
        let pool = subject_pool(&theme);
        let max_chroma = pool.iter().map(|c| c.chroma).fold(0.0f32, f32::max);
        assert!(max_chroma > 0.05, "subject pool should have chromatic colors");
    }

    #[test]
    fn test_gradient_monotone() {
        let mut img = RgbImage::new(128, 1);
        for x in 0..128u32 {
            let v = (x * 2) as u8;
            img.put_pixel(x, 0, Rgb([v, v, v]));
        }
        let result = recolor_wallpaper(&img, &test_theme());
        let left  = rgb_to_oklch(result.get_pixel(5, 0));
        let right = rgb_to_oklch(result.get_pixel(122, 0));
        assert!(left.l < right.l + 0.1,
            "gradient should stay directional: L_left={} L_right={}", left.l, right.l);
    }

    #[test]
    fn test_center_more_salient_than_corner() {
        let w = 128u32; let h = 128u32;
        let img = RgbImage::from_pixel(w, h, Rgb([128u8, 128, 128]));
        let l = compute_lightness_map(&img);
        let s = compute_saliency_map(&l, w, h);
        assert!(s[(h/2*w+w/2) as usize] > s[0],
            "center should be more salient than corner");
    }
}
