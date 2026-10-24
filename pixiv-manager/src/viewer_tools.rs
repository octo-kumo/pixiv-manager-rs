//! Display-only image transforms for the Twitter viewer.
//!
//! Artists sometimes split one image into several tweet photos (side by side
//! or stacked) or post it rotated sideways. These helpers detect such splits,
//! recombine pages and rotate the result — all on decoded copies, never on
//! the files on disk. The chosen transform per tweet is persisted separately
//! (`viewer_transforms.json`) so it is restored the next time the tweet is
//! opened.

use image::{imageops, RgbaImage};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

/// Rotation slot of the combined (merged) image in [`ViewTransform::rotations`].
pub const MERGED_SLOT: usize = usize::MAX;

/// Largest side (px) of a recombined image; bigger results are downscaled so
/// they stay within common GPU texture limits.
const MAX_SIDE: u32 = 8192;

/// How the pages of a split image are laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MergeAxis {
    /// Pages sit next to each other (the image was cut with vertical lines).
    Horizontal,
    /// Pages are stacked (the image was cut with horizontal lines).
    Vertical,
}

/// What a rendered transform shows: one rotated page, or the combined image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransformTarget {
    Page(usize),
    Merged(MergeAxis),
}

/// Identifies one rendered transform result, so a finished render can be
/// matched against what the viewer currently wants to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransformKey {
    pub id: u64,
    pub target: TransformTarget,
    /// Clockwise quarter turns applied after combining.
    pub rotation: u8,
}

/// The persisted display transform of one tweet.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewTransform {
    /// Show all pages combined into one image.
    pub merge: Option<MergeAxis>,
    /// Clockwise quarter turns (1–3) per page index, or per [`MERGED_SLOT`]
    /// for the combined image. Absent = not rotated.
    pub rotations: BTreeMap<usize, u8>,
}

impl ViewTransform {
    pub fn is_identity(&self) -> bool {
        self.merge.is_none() && self.rotations.is_empty()
    }

    /// Quarter turns applied to `slot` (a page index or [`MERGED_SLOT`]).
    pub fn rotation(&self, slot: usize) -> u8 {
        self.rotations.get(&slot).copied().unwrap_or(0)
    }

    /// Turn `slot` by `quarters` clockwise (negative = counter-clockwise).
    pub fn rotate(&mut self, slot: usize, quarters: i8) {
        let next = (self.rotation(slot) as i8 + quarters).rem_euclid(4) as u8;
        if next == 0 {
            self.rotations.remove(&slot);
        } else {
            self.rotations.insert(slot, next);
        }
    }

    /// Switch merging to `axis`, or off when it is already on that axis.
    pub fn toggle_merge(&mut self, axis: MergeAxis) {
        self.merge = if self.merge == Some(axis) {
            None
        } else {
            Some(axis)
        };
    }
}

// ── Persistence ────────────────────────────────────────────────────────

fn transforms_path() -> PathBuf {
    crate::io::library::library_dir().join("viewer_transforms.json")
}

/// Load the saved transforms (missing/corrupt → empty).
pub fn load() -> HashMap<u64, ViewTransform> {
    std::fs::read_to_string(transforms_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Persist the transforms (identity entries are dropped). Best-effort.
pub fn save(transforms: &HashMap<u64, ViewTransform>) {
    let path = transforms_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let kept: BTreeMap<&u64, &ViewTransform> = transforms
        .iter()
        .filter(|(_, t)| !t.is_identity())
        .collect();
    match serde_json::to_string(&kept) {
        Ok(json) => {
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, json).is_ok() {
                if let Err(e) = std::fs::rename(&tmp, &path) {
                    log::warn!("[viewer_tools] save failed: {e}");
                }
            }
        }
        Err(e) => log::warn!("[viewer_tools] serialize failed: {e}"),
    }
}

// ── Image operations ───────────────────────────────────────────────────

/// Decode encoded image bytes to RGBA.
pub fn decode(bytes: &[u8]) -> Result<RgbaImage, String> {
    image::load_from_memory(bytes)
        .map(|i| i.to_rgba8())
        .map_err(|e| e.to_string())
}

/// Rotate clockwise by `quarters` quarter turns.
pub fn rotate(img: RgbaImage, quarters: u8) -> RgbaImage {
    match quarters % 4 {
        1 => imageops::rotate90(&img),
        2 => imageops::rotate180(&img),
        3 => imageops::rotate270(&img),
        _ => img,
    }
}

/// Lay `pages` out along `axis` into a single image (top/left aligned),
/// downscaled when it exceeds [`MAX_SIDE`].
pub fn combine(pages: &[RgbaImage], axis: MergeAxis) -> RgbaImage {
    let (w, h) = match axis {
        MergeAxis::Horizontal => (
            pages.iter().map(|p| p.width()).sum::<u32>(),
            pages.iter().map(|p| p.height()).max().unwrap_or(0),
        ),
        MergeAxis::Vertical => (
            pages.iter().map(|p| p.width()).max().unwrap_or(0),
            pages.iter().map(|p| p.height()).sum::<u32>(),
        ),
    };
    let mut out = RgbaImage::new(w.max(1), h.max(1));
    let mut offset = 0i64;
    for page in pages {
        match axis {
            MergeAxis::Horizontal => {
                imageops::overlay(&mut out, page, offset, 0);
                offset += page.width() as i64;
            }
            MergeAxis::Vertical => {
                imageops::overlay(&mut out, page, 0, offset);
                offset += page.height() as i64;
            }
        }
    }
    if out.width().max(out.height()) > MAX_SIDE {
        let scale = MAX_SIDE as f64 / out.width().max(out.height()) as f64;
        let nw = ((out.width() as f64 * scale).round() as u32).max(1);
        let nh = ((out.height() as f64 * scale).round() as u32).max(1);
        out = imageops::resize(&out, nw, nh, imageops::FilterType::Triangle);
    }
    out
}

/// Which pixel line of an image to sample.
#[derive(Clone, Copy)]
enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

/// The pixel line `depth` px inward from `edge`, cropped to `len` pixels.
fn line(img: &RgbaImage, edge: Edge, depth: u32, len: u32) -> Vec<[u8; 3]> {
    let rgb = |x: u32, y: u32| {
        let p = img.get_pixel(x, y).0;
        [p[0], p[1], p[2]]
    };
    match edge {
        Edge::Left => (0..len.min(img.height()))
            .map(|y| rgb(depth.min(img.width() - 1), y))
            .collect(),
        Edge::Right => (0..len.min(img.height()))
            .map(|y| rgb(img.width() - 1 - depth.min(img.width() - 1), y))
            .collect(),
        Edge::Top => (0..len.min(img.width()))
            .map(|x| rgb(x, depth.min(img.height() - 1)))
            .collect(),
        Edge::Bottom => (0..len.min(img.width()))
            .map(|x| rgb(x, img.height() - 1 - depth.min(img.height() - 1)))
            .collect(),
    }
}

/// Mean absolute per-channel difference of two pixel lines.
fn line_diff(a: &[[u8; 3]], b: &[[u8; 3]]) -> f64 {
    let n = a.len().min(b.len());
    if n == 0 {
        return f64::MAX;
    }
    let sum: u64 = a
        .iter()
        .zip(b)
        .map(|(p, q)| (0..3).map(|c| p[c].abs_diff(q[c]) as u64).sum::<u64>())
        .sum();
    sum as f64 / (n * 3) as f64
}

/// Standard deviation of a pixel line's luma — flat lines (plain borders)
/// carry no information about whether two images continue into each other.
fn line_detail(l: &[[u8; 3]]) -> f64 {
    if l.is_empty() {
        return 0.0;
    }
    let lumas: Vec<f64> = l
        .iter()
        .map(|p| 0.299 * p[0] as f64 + 0.587 * p[1] as f64 + 0.114 * p[2] as f64)
        .collect();
    let mean = lumas.iter().sum::<f64>() / lumas.len() as f64;
    (lumas.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / lumas.len() as f64).sqrt()
}

/// Whether `a`'s `a_edge` visually continues into `b`'s `b_edge`: the seam
/// difference is about as small as the difference between neighbouring lines
/// inside each image, and the seam has real detail (not a plain border).
fn seam_continues(a: &RgbaImage, a_edge: Edge, b: &RgbaImage, b_edge: Edge, len: u32) -> bool {
    let a0 = line(a, a_edge, 0, len);
    let b0 = line(b, b_edge, 0, len);
    if line_detail(&a0).min(line_detail(&b0)) < 10.0 {
        return false;
    }
    let seam = line_diff(&a0, &b0);
    let inner_a = line_diff(&a0, &line(a, a_edge, 1, len));
    let inner_b = line_diff(&b0, &line(b, b_edge, 1, len));
    seam <= (inner_a + inner_b) * 1.25 + 4.0
}

/// Detect whether `pages` are the pieces of one image split along a single
/// axis: same extent across the seams and visually continuous seams. Favors
/// precision — a false prompt is worse than a missed one, and the manual
/// buttons stay available either way.
pub fn detect_split(pages: &[RgbaImage]) -> Option<MergeAxis> {
    if !(2..=8).contains(&pages.len()) {
        return None;
    }
    let same = |f: fn(&RgbaImage) -> u32| {
        let first = f(&pages[0]);
        pages.iter().all(|p| f(p).abs_diff(first) <= 1)
    };
    if same(RgbaImage::height) {
        let len = pages.iter().map(|p| p.height()).min()?;
        let ok = pages
            .windows(2)
            .all(|w| seam_continues(&w[0], Edge::Right, &w[1], Edge::Left, len));
        if ok {
            return Some(MergeAxis::Horizontal);
        }
    }
    if same(RgbaImage::width) {
        let len = pages.iter().map(|p| p.width()).min()?;
        let ok = pages
            .windows(2)
            .all(|w| seam_continues(&w[0], Edge::Bottom, &w[1], Edge::Top, len));
        if ok {
            return Some(MergeAxis::Vertical);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    /// A smooth test picture with detail along both axes.
    fn picture(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_fn(w, h, |x, y| {
            let (fx, fy) = (x as f32, y as f32);
            let v = 128.0 + 100.0 * (fx * 0.13).sin() * (fy * 0.11).cos();
            let u = 128.0 + 90.0 * (fx * 0.05 + fy * 0.07).sin();
            Rgba([v as u8, u as u8, ((fx + fy) as u32 % 200) as u8 + 20, 255])
        })
    }

    fn crop(img: &RgbaImage, x: u32, y: u32, w: u32, h: u32) -> RgbaImage {
        imageops::crop_imm(img, x, y, w, h).to_image()
    }

    #[test]
    fn rotation_wraps_and_clears() {
        let mut t = ViewTransform::default();
        t.rotate(0, 1);
        assert_eq!(t.rotation(0), 1);
        t.rotate(0, -1);
        assert!(t.is_identity());
        t.rotate(2, -1);
        assert_eq!(t.rotation(2), 3);
        t.rotate(2, 2);
        assert_eq!(t.rotation(2), 1);
    }

    #[test]
    fn merge_toggles_off_on_same_axis() {
        let mut t = ViewTransform::default();
        t.toggle_merge(MergeAxis::Horizontal);
        assert_eq!(t.merge, Some(MergeAxis::Horizontal));
        t.toggle_merge(MergeAxis::Vertical);
        assert_eq!(t.merge, Some(MergeAxis::Vertical));
        t.toggle_merge(MergeAxis::Vertical);
        assert!(t.is_identity());
    }

    #[test]
    fn transform_roundtrips_through_json() {
        let mut t = ViewTransform::default();
        t.toggle_merge(MergeAxis::Horizontal);
        t.rotate(MERGED_SLOT, 1);
        t.rotate(3, 2);
        let json = serde_json::to_string(&HashMap::from([(7u64, t.clone())])).unwrap();
        let back: HashMap<u64, ViewTransform> = serde_json::from_str(&json).unwrap();
        assert_eq!(back[&7], t);
    }

    #[test]
    fn combine_and_rotate_dimensions() {
        let a = picture(10, 20);
        let b = picture(15, 20);
        let h = combine(&[a.clone(), b.clone()], MergeAxis::Horizontal);
        assert_eq!((h.width(), h.height()), (25, 20));
        assert_eq!(h.get_pixel(10, 0), b.get_pixel(0, 0));
        let v = combine(&[a, picture(10, 5)], MergeAxis::Vertical);
        assert_eq!((v.width(), v.height()), (10, 25));
        let r = rotate(h, 1);
        assert_eq!((r.width(), r.height()), (20, 25));
    }

    #[test]
    fn detects_horizontal_split() {
        let whole = picture(400, 300);
        let pieces = [crop(&whole, 0, 0, 200, 300), crop(&whole, 200, 0, 200, 300)];
        assert_eq!(detect_split(&pieces), Some(MergeAxis::Horizontal));
    }

    #[test]
    fn detects_vertical_split() {
        let whole = picture(300, 600);
        let pieces = [
            crop(&whole, 0, 0, 300, 200),
            crop(&whole, 0, 200, 300, 200),
            crop(&whole, 0, 400, 300, 200),
        ];
        assert_eq!(detect_split(&pieces), Some(MergeAxis::Vertical));
    }

    #[test]
    fn rejects_unrelated_same_size_images() {
        let a = picture(200, 300);
        let b = RgbaImage::from_fn(200, 300, |x, y| {
            Rgba([(x * 3 % 256) as u8, (y * 5 % 256) as u8, 200, 255])
        });
        assert_eq!(detect_split(&[a, b]), None);
    }

    #[test]
    fn rejects_plain_borders_and_wrong_counts() {
        let white = RgbaImage::from_pixel(100, 100, Rgba([255, 255, 255, 255]));
        assert_eq!(detect_split(&[white.clone(), white.clone()]), None);
        assert_eq!(detect_split(&[white]), None);
    }
}
