//! MMCQ (Modified Median Cut Quantization, a.k.a. "Color Thief") — a
//! self-contained median-cut algorithm that reduces an image to its most
//! representative colors. Used to build per-illust palettes that power the
//! sRGB/Hue color sorts and the viewer's palette swatches.
//!
//! Port of the well-known public-domain algorithm (Lokesh Dhakar's
//! color-thief), adapted to Rust's iterator style. Runs offline on decoded
//! image pixels; no external crate needed.

use std::collections::{HashMap, VecDeque};

/// A single color as `(r, g, b)` with `u8` components.
pub type Rgb = (u8, u8, u8);

/// The largest number of colors a quantized image may use (the MMCQ vbox
/// split ceiling).
const MAX_COLORS: usize = 5;
/// Colors are bucketed by the top `SIGBITS` bits of each channel.
const SIGBITS: u8 = 5;
/// `1 << SIGBITS` — the number of buckets per channel.
const RSHIFT: u8 = 8 - SIGBITS;
/// The product of bucket counts (used to size the color histogram).
const MULT: usize = 1 << (3 * SIGBITS);

/// Extract a `MAX_COLORS`-color palette from raw RGBA8 pixels (`width ×
/// height`). Returns the palette sorted by population (most common first),
/// or an empty vec when there's no pixel data.
pub fn quantize(rgba: &[u8], width: u32, height: u32) -> Vec<Rgb> {
    if width == 0 || height == 0 || rgba.len() < (width as usize * height as usize) * 4 {
        return Vec::new();
    }

    // 1. Build a histogram of quantized colors (only fully-opaque pixels).
    let mut histo: HashMap<u32, usize> = HashMap::with_capacity(MULT / 2);
    for px in rgba.chunks_exact(4) {
        let alpha = px[3];
        if alpha < 125 {
            continue;
        }
        let r = px[0] >> RSHIFT;
        let g = px[1] >> RSHIFT;
        let b = px[2] >> RSHIFT;
        let index = ((r as u32) << (2 * SIGBITS)) | ((g as u32) << SIGBITS) | b as u32;
        *histo.entry(index).or_insert(0) += 1;
    }
    if histo.is_empty() {
        return Vec::new();
    }

    // 2. Median-cut into vboxes until we have MAX_COLORS (or run out).
    let vboxes = median_cut(&histo, MAX_COLORS);

    // 3. The palette is each vbox's population-weighted average color,
    //    sorted by population descending.
    let mut palette: Vec<(Rgb, usize)> = vboxes
        .iter()
        .filter_map(|v| v.avg(&histo).map(|rgb| (rgb, v.pop(&histo))))
        .collect();
    palette.sort_by_key(|x| std::cmp::Reverse(x.1));
    palette.into_iter().map(|(rgb, _)| rgb).collect()
}

/// A box in quantized color space, tracked by its channel ranges and pixel
/// count.
struct VBox {
    r_min: u8,
    r_max: u8,
    g_min: u8,
    g_max: u8,
    b_min: u8,
    b_max: u8,
}

impl VBox {
    fn new(
        r_min: u8,
        r_max: u8,
        g_min: u8,
        g_max: u8,
        b_min: u8,
        b_max: u8,
        histo: &HashMap<u32, usize>,
    ) -> Self {
        let _ = histo;
        Self {
            r_min,
            r_max,
            g_min,
            g_max,
            b_min,
            b_max,
        }
    }

    /// Number of pixels inside this box.
    fn pop(&self, histo: &HashMap<u32, usize>) -> usize {
        let mut total = 0;
        for r in self.r_min..=self.r_max {
            for g in self.g_min..=self.g_max {
                for b in self.b_min..=self.b_max {
                    let index = ((r as u32) << (2 * SIGBITS)) | ((g as u32) << SIGBITS) | b as u32;
                    total += histo.get(&index).copied().unwrap_or(0);
                }
            }
        }
        total
    }

    /// The population-weighted average color of the box (in 0..255 space).
    fn avg(&self, histo: &HashMap<u32, usize>) -> Option<Rgb> {
        let mut ntot = 0usize;
        let mut rsum = 0usize;
        let mut gsum = 0usize;
        let mut bsum = 0usize;
        for r in self.r_min..=self.r_max {
            for g in self.g_min..=self.g_max {
                for b in self.b_min..=self.b_max {
                    let index = ((r as u32) << (2 * SIGBITS)) | ((g as u32) << SIGBITS) | b as u32;
                    if let Some(&cnt) = histo.get(&index) {
                        ntot += cnt;
                        rsum += cnt * r as usize;
                        gsum += cnt * g as usize;
                        bsum += cnt * b as usize;
                    }
                }
            }
        }
        if ntot == 0 {
            return None;
        }
        Some((
            ((rsum / ntot) << RSHIFT) as u8,
            ((gsum / ntot) << RSHIFT) as u8,
            ((bsum / ntot) << RSHIFT) as u8,
        ))
    }
}

/// Repeatedly split the largest box along its longest channel's median until
/// `target` boxes exist or no box can be split further.
fn median_cut(histo: &HashMap<u32, usize>, target: usize) -> Vec<VBox> {
    let mut queue: VecDeque<VBox> = VecDeque::new();
    if let Some(root) = box_from_histo(histo) {
        queue.push_back(root);
    }
    while queue.len() < target && queue.iter().any(splittable) {
        let v = queue
            .iter()
            .enumerate()
            .filter(|(_, v)| splittable(v))
            .max_by_key(|(_, v)| v.pop(histo))
            .map(|(i, _)| i);
        let Some(i) = v else { break };
        let vbox = queue.remove(i).unwrap();
        if let Some((a, b)) = split(&vbox, histo) {
            queue.push_back(a);
            queue.push_back(b);
        } else {
            queue.push_back(vbox);
            break;
        }
    }
    queue.into_iter().collect()
}

/// Can this box be split (does it occupy more than one bucket on some axis)?
fn splittable(v: &VBox) -> bool {
    (v.r_max - v.r_min > 0) || (v.g_max - v.g_min > 0) || (v.b_max - v.b_min > 0)
}

/// Compute the initial box spanning the whole histogram.
fn box_from_histo(histo: &HashMap<u32, usize>) -> Option<VBox> {
    if histo.is_empty() {
        return None;
    }
    let mut r_min = u8::MAX;
    let mut r_max = 0u8;
    let mut g_min = u8::MAX;
    let mut g_max = 0u8;
    let mut b_min = u8::MAX;
    let mut b_max = 0u8;
    for &index in histo.keys() {
        let r = ((index >> (2 * SIGBITS)) & 0x1F) as u8;
        let g = ((index >> SIGBITS) & 0x1F) as u8;
        let b = (index & 0x1F) as u8;
        r_min = r_min.min(r);
        r_max = r_max.max(r);
        g_min = g_min.min(g);
        g_max = g_max.max(g);
        b_min = b_min.min(b);
        b_max = b_max.max(b);
    }
    Some(VBox::new(r_min, r_max, g_min, g_max, b_min, b_max, histo))
}

/// Split `v` along its longest channel at the median, returning the two
/// halves. Returns `None` when the box can't be split.
fn split(v: &VBox, histo: &HashMap<u32, usize>) -> Option<(VBox, VBox)> {
    let r_len = v.r_max - v.r_min;
    let g_len = v.g_max - v.g_min;
    let b_len = v.b_max - v.b_min;
    let half = v.pop(histo) / 2;
    if half == 0 {
        return None;
    }

    // Split on the longest axis at the median bucket (where the running
    // pixel count crosses half the box's population).
    if r_len >= g_len && r_len >= b_len {
        let mut cum = 0usize;
        for ch in v.r_min..v.r_max {
            cum += slice_pop_r(v, ch, histo);
            if cum >= half {
                return Some((
                    VBox { r_max: ch, ..*v },
                    VBox {
                        r_min: ch + 1,
                        ..*v
                    },
                ));
            }
        }
    } else if g_len >= b_len {
        let mut cum = 0usize;
        for ch in v.g_min..v.g_max {
            cum += slice_pop_g(v, ch, histo);
            if cum >= half {
                return Some((
                    VBox { g_max: ch, ..*v },
                    VBox {
                        g_min: ch + 1,
                        ..*v
                    },
                ));
            }
        }
    } else {
        let mut cum = 0usize;
        for ch in v.b_min..v.b_max {
            cum += slice_pop_b(v, ch, histo);
            if cum >= half {
                return Some((
                    VBox { b_max: ch, ..*v },
                    VBox {
                        b_min: ch + 1,
                        ..*v
                    },
                ));
            }
        }
    }
    None
}

/// Pixel count on a fixed `r` slice of the box.
fn slice_pop_r(v: &VBox, ch: u8, histo: &HashMap<u32, usize>) -> usize {
    let mut total = 0usize;
    for g in v.g_min..=v.g_max {
        for b in v.b_min..=v.b_max {
            let index = ((ch as u32) << (2 * SIGBITS)) | ((g as u32) << SIGBITS) | b as u32;
            total += histo.get(&index).copied().unwrap_or(0);
        }
    }
    total
}

/// Pixel count on a fixed `g` slice of the box.
fn slice_pop_g(v: &VBox, ch: u8, histo: &HashMap<u32, usize>) -> usize {
    let mut total = 0usize;
    for r in v.r_min..=v.r_max {
        for b in v.b_min..=v.b_max {
            let index = ((r as u32) << (2 * SIGBITS)) | ((ch as u32) << SIGBITS) | b as u32;
            total += histo.get(&index).copied().unwrap_or(0);
        }
    }
    total
}

/// Pixel count on a fixed `b` slice of the box.
fn slice_pop_b(v: &VBox, ch: u8, histo: &HashMap<u32, usize>) -> usize {
    let mut total = 0usize;
    for r in v.r_min..=v.r_max {
        for g in v.g_min..=v.g_max {
            let index = ((r as u32) << (2 * SIGBITS)) | ((g as u32) << SIGBITS) | ch as u32;
            total += histo.get(&index).copied().unwrap_or(0);
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Colors are 5-bit quantized averages, so expect bucket-center values.
    fn near(a: Rgb, b: Rgb, tol: u8) -> bool {
        (a.0.abs_diff(b.0) <= tol) && (a.1.abs_diff(b.1) <= tol) && (a.2.abs_diff(b.2) <= tol)
    }

    #[test]
    fn quantize_returns_palette_for_solid_image() {
        // A 2x2 image of pure red → palette is red.
        let rgba = [
            255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255,
        ];
        let palette = quantize(&rgba, 2, 2);
        assert_eq!(palette.len(), 1);
        assert!(near(palette[0], (255, 0, 0), 8));
    }

    #[test]
    fn quantize_empty_input() {
        assert!(quantize(&[], 0, 0).is_empty());
    }

    #[test]
    fn quantize_two_colors() {
        // 2x1: red + blue.
        let rgba = [255, 0, 0, 255, 0, 0, 255, 255];
        let palette = quantize(&rgba, 2, 1);
        assert!(palette.len() >= 2, "two distinct colors, got {palette:?}");
        assert!(palette.iter().any(|c| near(*c, (255, 0, 0), 8)));
        assert!(palette.iter().any(|c| near(*c, (0, 0, 255), 8)));
    }

    #[test]
    fn quantize_ignores_transparent_pixels() {
        let rgba = [255, 0, 0, 0, 0, 0, 255, 255];
        let palette = quantize(&rgba, 2, 1);
        assert_eq!(palette.len(), 1);
        assert!(near(palette[0], (0, 0, 255), 8));
    }
}
