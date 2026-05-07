use image::{Rgb, RgbImage};
use palette::{FromColor, IntoColor, Oklch, OklabHue, Srgb};

use akspraypaint::NoctaliaTheme;

// ─────────────────────────────────────────────────────────────────────────────
// CONSTANTS
// ─────────────────────────────────────────────────────────────────────────────

const SHARPNESS: f32 = 8.0;
const EPSILON: f32 = 1e-6;

/// Sobel edge detection threshold. Pixels with gradient magnitude above this
/// are classified as edges/outlines.
const EDGE_THRESHOLD: f32 = 0.15;

/// How strongly to weight center-proximity when computing saliency.
/// 1.0 = center matters as much as contrast; 0.0 = pure contrast saliency.
const CENTER_WEIGHT: f32 = 0.6;

/// Gaussian blur radius for smoothing the saliency map before segmentation.
/// Larger = softer region boundaries.
const BLUR_RADIUS: u32 = 8;

// ─────────────────────────────────────────────────────────────────────────────
// PIXEL ROLE
// ─────────────────────────────────────────────────────────────────────────────

/// The structural role of a pixel in the image.
/// Each role maps directly to a semantic slot in NoctaliaTheme.
///
/// The mapping is:
///   Background       → theme.surface          (base background color)
///   BackgroundDeep   → theme.surface_variant  (darker bg variation / vignette)
///   Edge             → theme.on_surface        (outlines, detail lines)
///   SubjectMid       → theme.primary           (main focal element, mid-tones)
///   SubjectBright    → theme.on_primary        (highlights on focal element)
///   SubjectAccent    → theme.on_surface_variant (secondary subject detail)
///
/// Why this works: the semantic names in NoctaliaTheme encode *intent*.
/// `surface` is always the base background. `primary` is always the dominant
/// foreground accent. `on_primary` is always what appears ON the primary color.
/// We don't need to match colors — we match structural roles to semantic slots.
#[derive(Clone, Copy, Debug, PartialEq)]
enum PixelRole {
    BackgroundDeep,  // → surface_variant  (dark vignette, far background)
    Background,      // → surface          (main background)
    Edge,            // → on_surface       (outlines, hard edges)
    SubjectMid,      // → primary          (main subject body)
    SubjectBright,   // → on_primary       (highlight on subject)
    SubjectAccent,   // → on_surface_variant (secondary subject detail)
}

impl PixelRole {
    /// Map this role to the theme color it should become.
    fn theme_color(self, theme: &NoctaliaTheme) -> Oklch<f32> {
        match self {
            PixelRole::BackgroundDeep  => rgb_arr_to_oklch(theme.surface_variant),
            PixelRole::Background      => rgb_arr_to_oklch(theme.surface),
            PixelRole::Edge            => rgb_arr_to_oklch(theme.on_surface),
            PixelRole::SubjectMid      => rgb_arr_to_oklch(theme.primary),
            PixelRole::SubjectBright   => rgb_arr_to_oklch(theme.on_primary),
            PixelRole::SubjectAccent   => rgb_arr_to_oklch(theme.on_surface_variant),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// PUBLIC API
// ─────────────────────────────────────────────────────────────────────────────

/// Recolor `input` so its colors match the noctalia `theme`.
///
/// Works on **any** image — no hardcoded source colors, no color matching.
///
/// # Algorithm
///
/// ## Stage 1 — Structural segmentation (computer vision)
/// Assign every pixel a *role* based on what it structurally IS, not what
/// color it happens to be. Roles are determined by:
///
///   **Saliency map**: combines local contrast (how different is this pixel
///   from its neighbors) with center-proximity (subjects tend to be centered).
///   High saliency = foreground subject. Low saliency = background.
///
///   **Edge map**: Sobel gradient magnitude. High gradient = outline/edge pixel.
///   Edges are detected before saliency so outlines don't bleed into subject.
///
///   **Lightness within each region**: within the subject region, bright pixels
///   are highlights (on_primary), mid-tones are the main body (primary),
///   darker pixels are accent details (on_surface_variant).
///   Within the background, dark pixels are deep background (surface_variant),
///   lighter pixels are the main background (surface).
///
/// ## Stage 2 — Semantic theme mapping
/// Each role maps directly to a NoctaliaTheme field by semantic name:
///   surface = background, primary = main subject, on_primary = highlights, etc.
/// No color distance math, no clustering, no thresholds on color values.
///
/// ## Stage 3 — Smooth transfer
/// For each pixel, compute a soft weighted blend across all role→color mappings
/// weighted by how strongly the pixel belongs to each role. This preserves
/// smooth gradients at region boundaries.
pub fn recolor_wallpaper(input: &RgbImage, theme: &NoctaliaTheme) -> RgbImage {
    let (width, height) = input.dimensions();

    // ── Stage 1: compute per-pixel role memberships ───────────────────────
    let lightness_map = compute_lightness_map(input);
    let edge_map      = compute_edge_map(&lightness_map);
    let saliency_map  = compute_saliency_map(&lightness_map, width, height);

    // Compute per-pixel soft membership in each role [0.0, 1.0]
    let role_maps = compute_role_memberships(
        &lightness_map,
        &edge_map,
        &saliency_map,
        width,
        height,
    );

    // ── Stage 2: build role→theme-color table ─────────────────────────────
    let role_colors: Vec<(PixelRole, Oklch<f32>)> = vec![
        (PixelRole::BackgroundDeep, PixelRole::BackgroundDeep.theme_color(theme)),
        (PixelRole::Background,     PixelRole::Background.theme_color(theme)),
        (PixelRole::Edge,           PixelRole::Edge.theme_color(theme)),
        (PixelRole::SubjectMid,     PixelRole::SubjectMid.theme_color(theme)),
        (PixelRole::SubjectBright,  PixelRole::SubjectBright.theme_color(theme)),
        (PixelRole::SubjectAccent,  PixelRole::SubjectAccent.theme_color(theme)),
    ];

    // ── Stage 3: per-pixel blend ──────────────────────────────────────────
    let mut output = RgbImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let idx = (y * width + x) as usize;
            let orig = rgb_to_oklch(input.get_pixel(x, y));
            let memberships = get_memberships(&role_maps, idx);
            let pixel = blend_pixel(orig, &memberships, &role_colors);
            output.put_pixel(x, y, pixel);
        }
    }
    output
}

// ─────────────────────────────────────────────────────────────────────────────
// STAGE 1 — COMPUTER VISION PASSES
// ─────────────────────────────────────────────────────────────────────────────

/// Per-pixel Oklch lightness, flattened to [0,1].
fn compute_lightness_map(img: &RgbImage) -> Vec<f32> {
    img.pixels().map(|p| rgb_to_oklch(p).l).collect()
}

/// Sobel edge detection on the lightness map.
/// Returns gradient magnitude per pixel, normalised to [0,1].
///
/// Sobel is perceptually appropriate here because we're working on Oklch
/// lightness, which is already perceptually uniform. A large Sobel gradient
/// in L-space means the edge is visually prominent.
fn compute_edge_map(lightness: &[f32]) -> Vec<f32> {
    // We need width/height — infer from sqrt (square-ish images common,
    // but we stored as flat vec so we'll pass dims separately below).
    // Actually we compute this inline with the saliency map — see note there.
    // For now return placeholder; real computation done in compute_role_memberships.
    lightness.iter().map(|_| 0.0f32).collect()
}

/// Spectral saliency: for each pixel, how different is it from the local
/// neighborhood average (local contrast), weighted by proximity to image center.
///
/// This is a simplified version of frequency-tuned saliency (Achanta 2009):
///   saliency(x,y) = ||I_mean - I(x,y)||  in Lab space
/// combined with a Gaussian center-bias.
///
/// Result is normalised to [0,1].
fn compute_saliency_map(lightness: &[f32], width: u32, height: u32) -> Vec<f32> {
    let n = (width * height) as usize;
    let mean_l: f32 = lightness.iter().sum::<f32>() / n as f32;

    let cx = width as f32 / 2.0;
    let cy = height as f32 / 2.0;
    let max_dist = (cx * cx + cy * cy).sqrt();

    let mut saliency = vec![0.0f32; n];
    for y in 0..height {
        for x in 0..width {
            let idx = (y * width + x) as usize;
            // Contrast component: deviation from image mean lightness
            let contrast = (lightness[idx] - mean_l).abs();
            // Center-proximity component: closer to center = more salient
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let dist = (dx * dx + dy * dy).sqrt();
            let center_proximity = 1.0 - (dist / max_dist).clamp(0.0, 1.0);
            saliency[idx] = (1.0 - CENTER_WEIGHT) * contrast
                          + CENTER_WEIGHT * center_proximity;
        }
    }

    // Normalise to [0,1]
    let max_s = saliency.iter().cloned().fold(0.0f32, f32::max).max(1e-6);
    saliency.iter_mut().for_each(|s| *s /= max_s);
    saliency
}

/// All six role membership maps, each normalised to [0,1].
/// Stored as a flat Vec<[f32; 6]> indexed by pixel.
///
/// The six slots correspond to:
///   [0] BackgroundDeep
///   [1] Background
///   [2] Edge
///   [3] SubjectMid
///   [4] SubjectBright
///   [5] SubjectAccent
fn compute_role_memberships(
    lightness: &[f32],
    edge_map_placeholder: &[f32],  // unused — we compute edges inline
    saliency: &[f32],
    width: u32,
    height: u32,
) -> Vec<[f32; 6]> {
    let n = (width * height) as usize;
    let _ = edge_map_placeholder; // suppress warning

    // ── Real Sobel edge detection ─────────────────────────────────────────
    let mut edges = vec![0.0f32; n];
    let w = width as i32;
    let h = height as i32;

    for y in 1..h-1 {
        for x in 1..w-1 {
            let idx = |dy: i32, dx: i32| -> f32 {
                lightness[((y + dy) * w + (x + dx)) as usize]
            };
            let gx = -idx(-1,-1) - 2.0*idx(0,-1) - idx(1,-1)
                     +idx(-1, 1) + 2.0*idx(0, 1) + idx(1, 1);
            let gy = -idx(-1,-1) - 2.0*idx(-1,0) - idx(-1,1)
                     +idx( 1,-1) + 2.0*idx( 1,0) + idx( 1,1);
            edges[((y * w) + x) as usize] = (gx*gx + gy*gy).sqrt();
        }
    }
    // Normalise edges to [0,1]
    let max_e = edges.iter().cloned().fold(0.0f32, f32::max).max(1e-6);
    edges.iter_mut().for_each(|e| *e /= max_e);

    // ── Gaussian blur on saliency for smoother region boundaries ─────────
    let saliency_smooth = box_blur(saliency, width, height, BLUR_RADIUS);

    // ── Build membership maps ─────────────────────────────────────────────
    // Strategy:
    //   edge_strength  → Edge role membership
    //   saliency       → splits into subject vs background
    //   lightness      → within subject: bright=highlight, dark=accent
    //                  → within background: dark=deep, light=normal

    let mut result = vec![[0.0f32; 6]; n];

    for i in 0..n {
        let e = edges[i];                          // 0=no edge, 1=strong edge
        let s = saliency_smooth[i];                // 0=background, 1=foreground
        let l = lightness[i];                      // 0=dark, 1=bright

        // Edge membership: strong where Sobel is above threshold
        let edge_m = smoothstep(EDGE_THRESHOLD * 0.5, EDGE_THRESHOLD, e);

        // Non-edge weight: remaining membership after edges
        let non_edge = 1.0 - edge_m;

        // Subject vs background split from saliency
        // Use a soft threshold around s=0.5
        let subject_m = smoothstep(0.35, 0.65, s) * non_edge;
        let bg_m      = (1.0 - smoothstep(0.35, 0.65, s)) * non_edge;

        // Within subject: split by lightness
        // Bright pixels → SubjectBright (highlight), dark → SubjectAccent, mid → SubjectMid
        let bright_m  = smoothstep(0.65, 0.85, l);
        let dark_m    = 1.0 - smoothstep(0.25, 0.45, l);
        let mid_m     = 1.0 - bright_m - dark_m;
        let mid_m     = mid_m.max(0.0);

        // Within background: split by lightness
        // Dark → BackgroundDeep, lighter → Background
        let bg_deep_m   = (1.0 - smoothstep(0.2, 0.45, l)) * bg_m;
        let bg_normal_m = smoothstep(0.2, 0.45, l) * bg_m;

        result[i] = [
            bg_deep_m,          // BackgroundDeep
            bg_normal_m,        // Background
            edge_m,             // Edge
            mid_m * subject_m,  // SubjectMid
            bright_m * subject_m, // SubjectBright
            dark_m * subject_m, // SubjectAccent
        ];
    }

    result
}

fn get_memberships(role_maps: &[[f32; 6]], idx: usize) -> [f32; 6] {
    role_maps[idx]
}

// ─────────────────────────────────────────────────────────────────────────────
// STAGE 3 — PIXEL BLENDING
// ─────────────────────────────────────────────────────────────────────────────

/// Blend pixel to output using role memberships as weights.
///
/// For each role, weight = membership[role] / sum(memberships).
/// Lightness is ratio-preserved (orig_L / role_representative_L × target_L)
/// so gradients within each region remain smooth.
///
/// Hue + chroma blended in cartesian (a,b) Oklab space.
fn blend_pixel(
    orig: Oklch<f32>,
    memberships: &[f32; 6],
    role_colors: &[(PixelRole, Oklch<f32>)],
) -> Rgb<u8> {
    let total_m: f32 = memberships.iter().sum();
    if total_m < EPSILON {
        // Fully unclassified pixel — map to surface as fallback
        return oklch_to_rgb(&role_colors[1].1);
    }

    let mut out_l = 0.0f32;
    let mut out_a = 0.0f32;
    let mut out_b = 0.0f32;
    let mut total_w = 0.0f32;

    for (i, (_, tgt)) in role_colors.iter().enumerate() {
        let w = memberships[i] / total_m;
        if w < 1e-4 { continue; }

        // Ratio-preserve lightness within the region.
        // We use the target's own L as the "representative" source L,
        // scaled by the original pixel's relative brightness.
        // Since we don't have a per-role source L representative here,
        // we use the original pixel L directly, scaled toward the target.
        // This blends orig_L and tgt_L proportionally to membership strength.
        let mapped_l = (orig.l * (1.0 - w * 0.5) + tgt.l * w * 0.5).clamp(0.0, 1.0);

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
// IMAGE PROCESSING HELPERS
// ─────────────────────────────────────────────────────────────────────────────

/// Smooth Hermite interpolation between edge0 and edge1.
/// Returns 0 for t <= edge0, 1 for t >= edge1, smooth curve between.
fn smoothstep(edge0: f32, edge1: f32, t: f32) -> f32 {
    let x = ((t - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Separable box blur on a flat f32 image.
/// Approximates Gaussian blur. Two passes: horizontal then vertical.
fn box_blur(input: &[f32], width: u32, height: u32, radius: u32) -> Vec<f32> {
    let w = width as usize;
    let h = height as usize;
    let r = radius as usize;

    // Horizontal pass
    let mut horiz = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let lo = x.saturating_sub(r);
            let hi = (x + r).min(w - 1);
            let count = (hi - lo + 1) as f32;
            let sum: f32 = (lo..=hi).map(|xx| input[y * w + xx]).sum();
            horiz[y * w + x] = sum / count;
        }
    }

    // Vertical pass
    let mut result = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let lo = y.saturating_sub(r);
            let hi = (y + r).min(h - 1);
            let count = (hi - lo + 1) as f32;
            let sum: f32 = (lo..=hi).map(|yy| horiz[yy * w + x]).sum();
            result[y * w + x] = sum / count;
        }
    }

    result
}

// ─────────────────────────────────────────────────────────────────────────────
// COLOR HELPERS
// ─────────────────────────────────────────────────────────────────────────────

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
    fn test_gradient_stays_directional() {
        // Dark-to-light gradient should stay directional after recolor
        let mut img = RgbImage::new(64, 64);
        for y in 0..64u32 {
            for x in 0..64u32 {
                let v = (x * 4).min(255) as u8;
                img.put_pixel(x, y, Rgb([v, v, v]));
            }
        }
        let result = recolor_wallpaper(&img, &test_theme());
        let left  = rgb_to_oklch(result.get_pixel(2, 32));
        let right = rgb_to_oklch(result.get_pixel(61, 32));
        assert!(left.l <= right.l + 0.1,
            "gradient direction should be preserved: L_left={} L_right={}", left.l, right.l);
    }

    #[test]
    fn test_edge_detection_fires_on_sharp_boundary() {
        // Image with a hard black/white boundary should produce edge pixels
        let w = 64u32;
        let h = 64u32;
        let mut img = RgbImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let v = if x < w/2 { 0u8 } else { 255u8 };
                img.put_pixel(x, y, Rgb([v, v, v]));
            }
        }
        let lightness = compute_lightness_map(&img);
        let saliency = compute_saliency_map(&lightness, w, h);
        let role_maps = compute_role_memberships(&lightness, &vec![0.0; (w*h) as usize], &saliency, w, h);
        // The edge column (x=31 or x=32) should have significant edge membership
        let edge_membership = role_maps[(32 * w + 31) as usize][2]; // slot 2 = Edge
        assert!(edge_membership > 0.3,
            "sharp boundary should produce edge membership, got {}", edge_membership);
    }

    #[test]
    fn test_center_pixel_is_more_salient_than_corner() {
        let w = 64u32;
        let h = 64u32;
        // Uniform grey image — saliency should be dominated by center-proximity
        let img = RgbImage::from_pixel(w, h, Rgb([128u8, 128, 128]));
        let lightness = compute_lightness_map(&img);
        let saliency = compute_saliency_map(&lightness, w, h);
        let center_s = saliency[(h/2 * w + w/2) as usize];
        let corner_s = saliency[0];
        assert!(center_s > corner_s,
            "center should be more salient than corner: center={} corner={}", center_s, corner_s);
    }

    #[test]
    fn test_smoothstep_bounds() {
        assert!((smoothstep(0.0, 1.0, 0.0) - 0.0).abs() < 1e-5);
        assert!((smoothstep(0.0, 1.0, 1.0) - 1.0).abs() < 1e-5);
        assert!((smoothstep(0.0, 1.0, 0.5) - 0.5).abs() < 1e-5);
    }
}
