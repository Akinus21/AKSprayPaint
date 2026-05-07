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

/// Saliency below this → definitely background.
/// Above SALIENCY_FG_MIN → definitely subject.
/// Between the two → soft blend.
const SALIENCY_BG_MAX: f32 = 0.35;
const SALIENCY_FG_MIN: f32 = 0.55;

/// Box blur radius for smoothing saliency map.
const BLUR_RADIUS: u32 = 16;

/// Center-bias weight in saliency computation.
const CENTER_WEIGHT: f32 = 0.6;

// ─────────────────────────────────────────────────────────────────────────────
// PUBLIC API
// ─────────────────────────────────────────────────────────────────────────────

/// Recolor `input` so its colors match the noctalia `theme`.
///
/// Works on any image — no hardcoded source colors.
///
/// # Algorithm
///
/// ## Step 1 — CV background detection
/// Use saliency (local contrast × center-proximity) to produce a per-pixel
/// background weight in [0,1]. Pixels with low saliency are background;
/// pixels with high saliency are subject. The saliency map is blurred to
/// produce smooth boundaries.
///
/// Background pixels are processed entirely by CV:
///   - Mapped directly to surface/surface_variant by lightness ratio
///   - Never touch the hue-family matcher
///   - No color bleed possible — they never see the accent colors
///
/// ## Step 2 — Hue-family matching for subject pixels
/// Non-background pixels (subject, outlines, highlights) go through
/// hue-family matching exactly as in the best previous version:
///   - K-means clusters extracted from the image
///   - Achromatic clusters matched by lightness to achromatic theme colors
///   - Chromatic clusters matched by nearest hue to chromatic theme colors
///
/// ## Step 3 — Smooth blend
/// Each pixel's final color = lerp(cv_color, hue_color, subject_weight)
/// where subject_weight comes from the saliency map.
/// This gives clean background with smooth transitions at region boundaries.
pub fn recolor_wallpaper(input: &RgbImage, theme: &NoctaliaTheme) -> RgbImage {
    let (width, height) = input.dimensions();
    let n = (width * height) as usize;

    // ── Step 1: CV background detection ──────────────────────────────────
    let lightness   = compute_lightness_map(input);
    let saliency    = compute_saliency_map(&lightness, width, height);
    let saliency_sm = box_blur(&saliency, width, height, BLUR_RADIUS);

    // Per-pixel subject weight: 0.0 = pure background, 1.0 = pure subject
    let subject_weights: Vec<f32> = saliency_sm.iter()
        .map(|&s| smoothstep(SALIENCY_BG_MAX, SALIENCY_FG_MIN, s))
        .collect();

    // Background colors: map lightness to surface/surface_variant
    let surface         = rgb_arr_to_oklch(theme.surface);
    let surface_variant = rgb_arr_to_oklch(theme.surface_variant);

    // ── Step 2: hue-family matching for subject pixels ────────────────────
    let samples  = sample_pixels(input, MAX_SAMPLES);
    let clusters = kmeans(&samples, K, KMEANS_ITER);

    let theme_colors: Vec<Oklch<f32>> = theme.palette()
        .into_iter()
        .map(rgb_arr_to_oklch)
        .collect();

    let hue_mappings = match_clusters(&clusters, &theme_colors);

    // ── Step 3: per-pixel blend ───────────────────────────────────────────
    let mut output = RgbImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let idx = (y * width + x) as usize;
            let orig = rgb_to_oklch(input.get_pixel(x, y));
            let sw = subject_weights[idx];

            // CV background color: blend surface_variant→surface by lightness
            // Dark pixels → surface_variant, lighter pixels → surface
            let bg_blend = smoothstep(surface_variant.l, surface.l.max(surface_variant.l + 0.01), orig.l);
            let cv_color = blend_oklch(surface_variant, surface, bg_blend);

            // Hue-family color for subject
            let hue_color = transfer_pixel(orig, &hue_mappings);

            // Final: lerp between cv_color and hue_color by subject weight
            let final_color = blend_oklch(cv_color, hue_color, sw);
            output.put_pixel(x, y, oklch_to_rgb(&final_color));
        }
    }
    output
}

// ─────────────────────────────────────────────────────────────────────────────
// STEP 1 — CV BACKGROUND DETECTION
// ─────────────────────────────────────────────────────────────────────────────

fn compute_lightness_map(img: &RgbImage) -> Vec<f32> {
    img.pixels().map(|p| rgb_to_oklch(p).l).collect()
}

/// Saliency = local contrast × center-proximity, normalised to [0,1].
/// Low saliency = background. High saliency = subject.
fn compute_saliency_map(lightness: &[f32], width: u32, height: u32) -> Vec<f32> {
    let n = (width * height) as usize;
    let mean_l = lightness.iter().sum::<f32>() / n as f32;
    let cx = width  as f32 / 2.0;
    let cy = height as f32 / 2.0;
    let max_dist = (cx*cx + cy*cy).sqrt();

    let mut sal = vec![0.0f32; n];
    for y in 0..height {
        for x in 0..width {
            let idx = (y * width + x) as usize;
            let contrast = (lightness[idx] - mean_l).abs();
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let center = 1.0 - ((dx*dx + dy*dy).sqrt() / max_dist).clamp(0.0, 1.0);
            sal[idx] = (1.0 - CENTER_WEIGHT) * contrast + CENTER_WEIGHT * center;
        }
    }
    let max_s = sal.iter().cloned().fold(0.0f32, f32::max).max(1e-6);
    sal.iter_mut().for_each(|s| *s /= max_s);
    sal
}

fn box_blur(input: &[f32], width: u32, height: u32, radius: u32) -> Vec<f32> {
    let w = width as usize;
    let h = height as usize;
    let r = radius as usize;
    let mut tmp = vec![0.0f32; w * h];
    let mut out = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let lo = x.saturating_sub(r);
            let hi = (x + r).min(w - 1);
            let n  = (hi - lo + 1) as f32;
            tmp[y*w+x] = (lo..=hi).map(|xx| input[y*w+xx]).sum::<f32>() / n;
        }
    }
    for y in 0..h {
        for x in 0..w {
            let lo = y.saturating_sub(r);
            let hi = (y + r).min(h - 1);
            let n  = (hi - lo + 1) as f32;
            out[y*w+x] = (lo..=hi).map(|yy| tmp[yy*w+x]).sum::<f32>() / n;
        }
    }
    out
}

fn smoothstep(lo: f32, hi: f32, t: f32) -> f32 {
    let x = ((t - lo) / (hi - lo)).clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

// ─────────────────────────────────────────────────────────────────────────────
// STEP 2 — HUE-FAMILY MATCHING
// ─────────────────────────────────────────────────────────────────────────────

/// Match clusters to theme colors:
///   Achromatic clusters (low chroma) → nearest lightness among theme colors
///   Chromatic clusters               → nearest hue among theme colors
fn match_clusters(
    clusters: &[Oklch<f32>],
    theme_colors: &[Oklch<f32>],
) -> Vec<(Oklch<f32>, Oklch<f32>)> {
    clusters.iter().map(|&src| {
        let target = if src.chroma < 0.06 {
            // Achromatic: use lightness to find nearest theme color
            *theme_colors.iter()
                .min_by(|a, b| {
                    (a.l - src.l).abs()
                        .partial_cmp(&(b.l - src.l).abs())
                        .unwrap()
                })
                .unwrap()
        } else {
            // Chromatic: use hue to find nearest theme color
            *theme_colors.iter()
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
// STEP 2 — K-MEANS
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
// STEP 3 — PIXEL TRANSFER
// ─────────────────────────────────────────────────────────────────────────────

/// Inverse-power-distance weighted transfer for subject pixels.
/// Lightness ratio-preserved. Hue+chroma blended in cartesian (a,b) space.
fn transfer_pixel(orig: Oklch<f32>, mappings: &[(Oklch<f32>, Oklch<f32>)]) -> Oklch<f32> {
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

    let final_l = (out_l / total_w).clamp(0.0, 1.0);
    let a = out_a / total_w;
    let b = out_b / total_w;
    Oklch {
        l: final_l,
        chroma: (a*a + b*b).sqrt().clamp(0.0, 0.5),
        hue: OklabHue::from_degrees(b.atan2(a).to_degrees()),
    }
}

/// Blend two Oklch colors in cartesian (a,b) space. t=0 → a, t=1 → b.
fn blend_oklch(a: Oklch<f32>, b: Oklch<f32>, t: f32) -> Oklch<f32> {
    let (aa, ab) = hue_to_ab(a.chroma, a.hue);
    let (ba, bb) = hue_to_ab(b.chroma, b.hue);
    let l = a.l + (b.l - a.l) * t;
    let ra = aa + (ba - aa) * t;
    let rb = ab + (bb - ab) * t;
    let chroma = (ra*ra + rb*rb).sqrt().clamp(0.0, 0.5);
    let hue = OklabHue::from_degrees(rb.atan2(ra).to_degrees());
    Oklch { l, chroma, hue }
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
        for p in img.pixels_mut() { *p = Rgb([100, 100, 150]); }
        assert_eq!(recolor_wallpaper(&img, &test_theme()).dimensions(), (64, 64));
    }

    #[test]
    fn test_corner_maps_to_bg_colors() {
        // Dark corner pixels must map to surface/surface_variant,
        // never to a bright chromatic accent color.
        let mut img = RgbImage::new(256, 256);
        for y in 0..256u32 { for x in 0..256u32 {
            // Dark everywhere, bright center
            let cx = (x as i32 - 128).abs() as u32;
            let cy = (y as i32 - 128).abs() as u32;
            let d = ((cx*cx + cy*cy) as f32).sqrt();
            let v = (200.0 - d.min(200.0)) as u8;
            img.put_pixel(x, y, Rgb([v/6, v/6, v/4]));
        }}
        let result = recolor_wallpaper(&img, &test_theme());
        // Corner (0,0) should be dark and low-chroma
        let corner = rgb_to_oklch(result.get_pixel(0, 0));
        assert!(corner.l < 0.4, "corner should be dark, got L={}", corner.l);
        assert!(corner.chroma < 0.15, "corner should be low-chroma, got chroma={}", corner.chroma);
    }

    #[test]
    fn test_center_gets_subject_treatment() {
        // Center pixel of a high-contrast image should have higher saliency
        // and thus more subject influence
        let mut img = RgbImage::new(128, 128);
        for y in 0..128u32 { for x in 0..128u32 {
            let cx = (x as i32 - 64).abs() as u32;
            let cy = (y as i32 - 64).abs() as u32;
            let d = ((cx*cx + cy*cy) as f32).sqrt();
            let v = (255.0 - d.min(255.0)) as u8;
            img.put_pixel(x, y, Rgb([v, v/2, v]));
        }}
        let l = compute_lightness_map(&img);
        let s = compute_saliency_map(&l, 128, 128);
        let sm = box_blur(&s, 128, 128, BLUR_RADIUS);
        let center_sw = smoothstep(SALIENCY_BG_MAX, SALIENCY_FG_MIN, sm[64*128+64]);
        let corner_sw = smoothstep(SALIENCY_BG_MAX, SALIENCY_FG_MIN, sm[0]);
        assert!(center_sw > corner_sw,
            "center subject weight ({}) should exceed corner ({})", center_sw, corner_sw);
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
            "gradient should stay directional: L={} → L={}", left.l, right.l);
    }

    #[test]
    fn test_blend_oklch_midpoint() {
        let a = Oklch { l: 0.2, chroma: 0.0, hue: OklabHue::from_degrees(0.0) };
        let b = Oklch { l: 0.8, chroma: 0.0, hue: OklabHue::from_degrees(0.0) };
        let mid = blend_oklch(a, b, 0.5);
        assert!((mid.l - 0.5).abs() < 0.01, "midpoint blend L should be 0.5, got {}", mid.l);
    }

    #[test]
    fn test_kmeans_count() {
        let samples: Vec<Oklch<f32>> = (0..200)
            .map(|i| Oklch { l: i as f32/200.0, chroma: 0.1, hue: OklabHue::from_degrees(120.0) })
            .collect();
        assert_eq!(kmeans(&samples, K, KMEANS_ITER).len(), K);
    }
}
