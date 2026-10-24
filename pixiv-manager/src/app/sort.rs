use pixivj_rs::model::Illustration;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Sort Direction ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SortDirection {
    #[default]
    Undefined,
    Down,
    Up,
}

impl SortDirection {
    pub fn cycle(self, nullable: bool) -> Self {
        match self {
            SortDirection::Undefined => SortDirection::Down,
            SortDirection::Down => SortDirection::Up,
            SortDirection::Up if nullable => SortDirection::Undefined,
            SortDirection::Up => SortDirection::Down,
        }
    }
}

// ── Sort Key ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[allow(dead_code)] // Srgb/Hue wired to color extraction
pub enum SortKey {
    Id,
    Views,
    Bookmarks,
    /// Bookmarks per view (bookmarks ÷ views ratio).
    BookmarksPerView,
    Date,
    Sanity,
    Pixel,
    Pages,
    Srgb,
    Hue,
}

impl SortKey {
    /// The sortable value of `illust`. Color sorts (`Srgb`/`Hue`) measure the
    /// distance between the user-chosen `color` and the illust's palette
    /// (the first `level` entries); other keys are plain numeric extracts.
    /// When no color is chosen (or no palette exists) color sorts return 0.
    fn extract_value(
        &self,
        illust: &Illustration,
        color: Option<[u8; 3]>,
        level: u8,
        palettes: &HashMap<u64, Vec<[u8; 3]>>,
    ) -> f64 {
        match self {
            SortKey::Id => illust.id as f64,
            SortKey::Views => illust.total_view.unwrap_or(0) as f64,
            SortKey::Bookmarks => illust.total_bookmarks.unwrap_or(0) as f64,
            SortKey::BookmarksPerView => {
                let views = illust.total_view.unwrap_or(0) as f64;
                let bookmarks = illust.total_bookmarks.unwrap_or(0) as f64;
                if views > 0.0 {
                    bookmarks / views
                } else {
                    0.0
                }
            }
            SortKey::Date => illust
                .create_date
                .map(|d| d.timestamp() as f64)
                .unwrap_or(0.0),
            SortKey::Sanity => illust.sanity_level.unwrap_or(0) as f64,
            SortKey::Pixel => {
                let w = illust.width.unwrap_or(0) as f64;
                let h = illust.height.unwrap_or(0) as f64;
                w * h
            }
            SortKey::Pages => illust.page_count() as f64,
            SortKey::Srgb => match color {
                Some(c) => palettes
                    .get(&illust.id)
                    .and_then(|p| nearest_rgb_distance(c, p, level))
                    .unwrap_or(f64::MAX),
                None => 0.0,
            },
            SortKey::Hue => match color {
                Some(c) => palettes
                    .get(&illust.id)
                    .and_then(|p| nearest_hue_distance(c, p, level))
                    .unwrap_or(f64::MAX),
                None => 0.0,
            },
        }
    }
}

/// Euclidean RGB distance from `c` to the closest palette entry within the
/// first `level` entries (0-indexed clamp). `None` when the palette is empty.
fn nearest_rgb_distance(c: [u8; 3], palette: &[[u8; 3]], level: u8) -> Option<f64> {
    let n = (level as usize).clamp(1, 5).min(palette.len());
    palette[..n]
        .iter()
        .map(|p| {
            let dr = c[0] as f64 - p[0] as f64;
            let dg = c[1] as f64 - p[1] as f64;
            let db = c[2] as f64 - p[2] as f64;
            dr * dr + dg * dg + db * db
        })
        .min_by(|a, b| a.partial_cmp(b).unwrap())
}

/// Below this saturation a color counts as neutral (black/white/gray): its
/// hue is visually meaningless, so it must not compete with actually colored
/// entries in the Hue sort.
const HUE_SATURATION_THRESHOLD: f64 = 0.12;
/// Below this value the brightest channel is so dark that the hue carries
/// almost no visible color either.
const HUE_MIN_BRIGHTNESS: f64 = 0.075;

/// The hue of an sRGB color, but only when it is meaningfully colored.
/// Neutral colors (low saturation, incl. black/white/gray) return `None` —
/// without this, pure black (all channels equal) lands on hue 0 and sorts as
/// "deep red", and near-grays drift toward whatever channel is 1 bit higher.
fn saturated_hue([r, g, b]: [u8; 3]) -> Option<f64> {
    let (r, g, b) = (r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    if delta == 0.0 || max < HUE_MIN_BRIGHTNESS {
        return None;
    }
    let saturation = delta / max;
    if saturation < HUE_SATURATION_THRESHOLD {
        return None;
    }
    let hue = if max == r {
        60.0 * (((g - b) / delta).rem_euclid(6.0))
    } else if max == g {
        60.0 * (((b - r) / delta) + 2.0)
    } else {
        60.0 * (((r - g) / delta) + 4.0)
    };
    Some(hue.rem_euclid(360.0))
}

/// Circular hue distance (wrap-around aware) from `c` to the closest palette
/// entry within the first `level` entries. Neutral palette entries (and a
/// neutral target color) are skipped; if nothing colored remains the result
/// is [`f64::MAX`] (neutral images sort as "no hue at all").
fn nearest_hue_distance(c: [u8; 3], palette: &[[u8; 3]], level: u8) -> Option<f64> {
    let n = (level as usize).clamp(1, 5).min(palette.len());
    let Some(target) = saturated_hue(c) else {
        return (n > 0).then_some(f64::MAX);
    };
    let nearest = palette[..n]
        .iter()
        .filter_map(|p| {
            let hue = saturated_hue(*p)?;
            let d = (target - hue).abs();
            // Wrap-around: the shortest circular distance is min(d, 360-d).
            Some(d.min(360.0 - d))
        })
        .min_by(|a, b| a.partial_cmp(b).unwrap());
    Some(nearest.unwrap_or(f64::MAX))
}

// ── Color parsing ──────────────────────────────────────────────────────

/// Parse a user-typed color into sRGB. Accepts `#rgb`, `#rrggbb` (alpha
/// variants ignore alpha; the leading `#` is optional), `rgb(r, g, b)` /
/// `rgba(..)` with 0–255 or percentage channels, `hsl(h, s%, l%)` /
/// `hsla(..)`, and a few common color names. `None` if it doesn't parse.
pub fn parse_color(input: &str) -> Option<[u8; 3]> {
    let s = input.trim().to_ascii_lowercase();
    if s.is_empty() {
        return None;
    }
    if let Some(open) = s.find('(') {
        let body = s[open + 1..].strip_suffix(')')?;
        let func = s[..open].trim();
        let parts: Vec<&str> = body
            .split([',', ' ', '/'])
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .collect();
        if !(parts.len() == 3 || parts.len() == 4) {
            return None;
        }
        return match func {
            "rgb" | "rgba" => {
                let ch = |p: &str| -> Option<u8> {
                    if let Some(pct) = p.strip_suffix('%') {
                        let v: f64 = pct.trim().parse().ok()?;
                        (0.0..=100.0)
                            .contains(&v)
                            .then(|| (v / 100.0 * 255.0).round() as u8)
                    } else {
                        let v: f64 = p.parse().ok()?;
                        (0.0..=255.0).contains(&v).then(|| v.round() as u8)
                    }
                };
                Some([ch(parts[0])?, ch(parts[1])?, ch(parts[2])?])
            }
            "hsl" | "hsla" => {
                let h: f64 = parts[0].trim_end_matches("deg").parse().ok()?;
                let pct = |p: &str| -> Option<f64> {
                    let v: f64 = p.strip_suffix('%')?.trim().parse().ok()?;
                    (0.0..=100.0).contains(&v).then_some(v / 100.0)
                };
                Some(hsl_to_rgb(
                    h.rem_euclid(360.0),
                    pct(parts[1])?,
                    pct(parts[2])?,
                ))
            }
            _ => None,
        };
    }
    let hex = s.strip_prefix('#').unwrap_or(&s);
    if hex.chars().all(|c| c.is_ascii_hexdigit()) {
        let digit = |c: char| c.to_digit(16).unwrap() as u8;
        let cs: Vec<char> = hex.chars().collect();
        return match cs.len() {
            3 | 4 => Some([digit(cs[0]) * 17, digit(cs[1]) * 17, digit(cs[2]) * 17]),
            6 | 8 => Some([
                digit(cs[0]) * 16 + digit(cs[1]),
                digit(cs[2]) * 16 + digit(cs[3]),
                digit(cs[4]) * 16 + digit(cs[5]),
            ]),
            _ => None,
        };
    }
    match s.as_str() {
        "red" => Some([255, 0, 0]),
        "orange" => Some([255, 165, 0]),
        "yellow" => Some([255, 255, 0]),
        "green" => Some([0, 128, 0]),
        "lime" => Some([0, 255, 0]),
        "cyan" | "aqua" => Some([0, 255, 255]),
        "blue" => Some([0, 0, 255]),
        "purple" => Some([128, 0, 128]),
        "magenta" | "fuchsia" => Some([255, 0, 255]),
        "pink" => Some([255, 192, 203]),
        "brown" => Some([165, 42, 42]),
        "black" => Some([0, 0, 0]),
        "white" => Some([255, 255, 255]),
        "gray" | "grey" => Some([128, 128, 128]),
        _ => None,
    }
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> [u8; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [r, g, b].map(|v| ((v + m) * 255.0).round() as u8)
}

// ── Sort State ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
#[allow(dead_code)] // color/color_level used by color sort
pub struct SortState {
    pub multi_sort: bool,
    pub active: Vec<(SortKey, SortDirection)>,
    pub color: Option<[u8; 3]>,
    pub color_level: u8,
    /// Raw text of the custom-color input; `Some` while the custom swatch is
    /// the selected color source. Never rewritten by parsing.
    pub custom_color_text: Option<String>,
}

impl SortState {
    #[allow(dead_code)]
    pub fn single(key: SortKey, dir: SortDirection) -> Self {
        Self {
            multi_sort: false,
            active: vec![(key, dir)],
            color: None,
            color_level: 0,
            custom_color_text: None,
        }
    }

    #[allow(dead_code)]
    pub fn by(&mut self, key: SortKey, dir: SortDirection) -> &mut Self {
        if !self.multi_sort {
            self.active.clear();
        }
        // Remove existing entry for this key
        self.active.retain(|(k, _)| *k != key);
        if dir != SortDirection::Undefined {
            self.active.push((key, dir));
        }
        self
    }

    /// Pure function: sort illustrations in-place by all active keys.
    /// `palettes` feeds the sRGB/Hue color comparators.
    pub fn sort(&self, illusts: &mut [Illustration], palettes: &HashMap<u64, Vec<[u8; 3]>>) {
        if self.active.is_empty() {
            return;
        }
        let color = self.color;
        let level = self.color_level.max(1);
        // Stable sort, applied in reverse order so the first key is primary
        for (key, dir) in self.active.iter().rev() {
            match dir {
                SortDirection::Undefined => {}
                SortDirection::Down => {
                    illusts.sort_by(|a, b| {
                        let va = key.extract_value(a, color, level, palettes);
                        let vb = key.extract_value(b, color, level, palettes);
                        // Higher values first
                        vb.partial_cmp(&va).unwrap_or(std::cmp::Ordering::Equal)
                    });
                }
                SortDirection::Up => {
                    illusts.sort_by(|a, b| {
                        let va = key.extract_value(a, color, level, palettes);
                        let vb = key.extract_value(b, color, level, palettes);
                        // Lower values first
                        va.partial_cmp(&vb).unwrap_or(std::cmp::Ordering::Equal)
                    });
                }
            }
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_illust(id: u64, views: u64, bookmarks: u64) -> Illustration {
        Illustration {
            id,
            title: None,
            illust_type: None,
            image_urls: None,
            caption: None,
            restrict: None,
            user: None,
            tags: vec![],
            tools: vec![],
            create_date: None,
            page_count: Some(1),
            width: Some(800),
            height: Some(600),
            sanity_level: None,
            x_restrict: None,
            series: None,
            meta_single_page: None,
            meta_pages: vec![],
            total_view: Some(views),
            total_bookmarks: Some(bookmarks),
            is_bookmarked: None,
            visible: None,
            is_muted: None,
            illust_ai_type: None,
            provider_data: None,
        }
    }

    #[test]
    fn parse_color_formats() {
        assert_eq!(parse_color("#ff8000"), Some([255, 128, 0]));
        assert_eq!(parse_color("#F80"), Some([255, 136, 0]));
        assert_eq!(parse_color("  00ff00 "), Some([0, 255, 0]));
        assert_eq!(parse_color("#11223344"), Some([0x11, 0x22, 0x33]));
        assert_eq!(parse_color("rgb(10, 20, 30)"), Some([10, 20, 30]));
        assert_eq!(parse_color("RGB(10 20 30)"), Some([10, 20, 30]));
        assert_eq!(parse_color("rgba(10,20,30,0.5)"), Some([10, 20, 30]));
        assert_eq!(parse_color("rgb(100%, 0%, 50%)"), Some([255, 0, 128]));
        assert_eq!(parse_color("hsl(120, 100%, 50%)"), Some([0, 255, 0]));
        assert_eq!(parse_color("Red"), Some([255, 0, 0]));
    }

    #[test]
    fn parse_color_rejects_garbage() {
        for bad in [
            "",
            "#",
            "#12",
            "#12345",
            "#gg0000",
            "rgb(1,2)",
            "rgb(256,0,0)",
            "rgb(a,b,c)",
            "hsl(0,1,1)",
            "foo(1,2,3)",
            "nope",
            "rgb(1,2,3",
        ] {
            assert_eq!(parse_color(bad), None, "{bad}");
        }
    }

    #[test]
    fn sort_direction_cycle_nullable() {
        let mut d = SortDirection::Undefined;
        d = d.cycle(true);
        assert_eq!(d, SortDirection::Down);
        d = d.cycle(true);
        assert_eq!(d, SortDirection::Up);
        d = d.cycle(true);
        assert_eq!(d, SortDirection::Undefined);
    }

    #[test]
    fn sort_direction_cycle_non_nullable() {
        let mut d = SortDirection::Undefined;
        d = d.cycle(false);
        assert_eq!(d, SortDirection::Down);
        d = d.cycle(false);
        assert_eq!(d, SortDirection::Up);
        d = d.cycle(false);
        assert_eq!(d, SortDirection::Down);
    }

    #[test]
    fn sort_by_views_descending() {
        let mut illusts = vec![
            test_illust(1, 100, 10),
            test_illust(2, 500, 5),
            test_illust(3, 50, 20),
        ];
        let sort = SortState::single(SortKey::Views, SortDirection::Down);
        sort.sort(&mut illusts, &Default::default());
        assert_eq!(illusts[0].id, 2); // 500 views
        assert_eq!(illusts[1].id, 1); // 100 views
        assert_eq!(illusts[2].id, 3); // 50 views
    }

    #[test]
    fn sort_by_views_ascending() {
        let mut illusts = vec![
            test_illust(1, 100, 10),
            test_illust(2, 500, 5),
            test_illust(3, 50, 20),
        ];
        let sort = SortState::single(SortKey::Views, SortDirection::Up);
        sort.sort(&mut illusts, &Default::default());
        assert_eq!(illusts[0].id, 3); // 50 views
        assert_eq!(illusts[1].id, 1); // 100 views
        assert_eq!(illusts[2].id, 2); // 500 views
    }

    #[test]
    fn sort_undefined_no_change() {
        let mut illusts = vec![test_illust(1, 100, 10), test_illust(2, 500, 5)];
        let sort = SortState::single(SortKey::Views, SortDirection::Undefined);
        sort.sort(&mut illusts, &Default::default());
        assert_eq!(illusts[0].id, 1); // unchanged
        assert_eq!(illusts[1].id, 2);
    }

    #[test]
    fn sort_by_bpv() {
        let mut illusts = vec![
            test_illust(1, 1000, 100), // BPV = 0.1
            test_illust(2, 100, 50),   // BPV = 0.5
            test_illust(3, 500, 10),   // BPV = 0.02
        ];
        let sort = SortState::single(SortKey::BookmarksPerView, SortDirection::Down);
        sort.sort(&mut illusts, &Default::default());
        assert_eq!(illusts[0].id, 2); // highest BPV
    }

    #[test]
    fn multi_sort_views_then_bookmarks() {
        let mut illusts = vec![
            test_illust(1, 100, 10),
            test_illust(2, 100, 50), // same views, more bookmarks
            test_illust(3, 200, 5),
        ];
        let mut sort = SortState {
            multi_sort: true,
            ..Default::default()
        };
        sort.by(SortKey::Views, SortDirection::Down);
        sort.by(SortKey::Bookmarks, SortDirection::Down);
        sort.sort(&mut illusts, &Default::default());
        assert_eq!(illusts[0].id, 3); // 200 views
        assert_eq!(illusts[1].id, 2); // 100 views, 50 bookmarks
        assert_eq!(illusts[2].id, 1); // 100 views, 10 bookmarks
    }

    #[test]
    fn sort_preserves_all_elements() {
        let mut illusts: Vec<Illustration> = (0..50).map(|i| test_illust(i, i * 10, i)).collect();
        let sort = SortState::single(SortKey::Views, SortDirection::Down);
        sort.sort(&mut illusts, &Default::default());
        assert_eq!(illusts.len(), 50);
    }

    #[test]
    fn srgb_sort_uses_palette_distance() {
        // Two illusts with palettes; one close to red, one close to blue.
        let mut red = test_illust(1, 100, 10);
        let mut blue = test_illust(2, 100, 10);
        red.id = 1;
        blue.id = 2;
        let palettes: HashMap<u64, Vec<[u8; 3]>> = HashMap::from([
            (1, vec![[255, 0, 0], [10, 10, 10]]),
            (2, vec![[0, 0, 255], [10, 10, 10]]),
        ]);
        let sort = SortState {
            active: vec![(SortKey::Srgb, SortDirection::Up)],
            color: Some([255, 0, 0]),
            color_level: 2,
            ..Default::default()
        };
        let mut illusts = vec![blue, red];
        sort.sort(&mut illusts, &palettes);
        // Nearest-to-red first.
        assert_eq!(illusts[0].id, 1);
    }

    #[test]
    fn hue_sort_wraps_around() {
        // Red (hue 0) and magenta (hue 300) are both near hue 350 (wrapping).
        let mut red = test_illust(1, 0, 0);
        red.id = 1;
        let mut magenta = test_illust(2, 0, 0);
        magenta.id = 2;
        let palettes: HashMap<u64, Vec<[u8; 3]>> = HashMap::from([
            (1, vec![[255, 0, 0]]),   // hue 0
            (2, vec![[255, 0, 255]]), // hue 300
        ]);
        let sort = SortState {
            active: vec![(SortKey::Hue, SortDirection::Up)],
            color: Some([255, 0, 100]), // hue ~340
            color_level: 1,
            ..Default::default()
        };
        let mut illusts = vec![red.clone(), magenta.clone()];
        sort.sort(&mut illusts, &palettes);
        // 340 is 20° from magenta (300) via wrap, 20° from red (0) via wrap
        // too — both 20; order is stable, so this just verifies no crash and
        // a sane, finite distance. Check the distance values directly:
        let d_red = SortKey::Hue.extract_value(&red, Some([255, 0, 100]), 1, &palettes);
        let d_magenta = SortKey::Hue.extract_value(&magenta, Some([255, 0, 100]), 1, &palettes);
        assert!(d_red.is_finite() && d_red < 180.0);
        assert!(d_magenta.is_finite() && d_magenta < 180.0);
    }

    #[test]
    fn color_sort_without_color_is_noop() {
        let mut red = test_illust(1, 0, 0);
        red.id = 1;
        let palettes: HashMap<u64, Vec<[u8; 3]>> = HashMap::from([(1, vec![[255, 0, 0]])]);
        // No color chosen → Srgb extracts 0.0 for everything (stable).
        let sort = SortState {
            active: vec![(SortKey::Srgb, SortDirection::Down)],
            color: None,
            color_level: 2,
            ..Default::default()
        };
        let mut illusts = vec![red.clone()];
        sort.sort(&mut illusts, &palettes);
        assert_eq!(illusts[0].id, 1);
    }

    #[test]
    fn hue_sort_ignores_neutral_colors() {
        // Black and white must NOT count as "hue 0" (deep red): their hue is
        // meaningless, so a black/white image is neutral for the Hue sort.
        let mut black = test_illust(1, 0, 0);
        black.id = 1;
        let mut white = test_illust(2, 0, 0);
        white.id = 2;
        let mut red = test_illust(3, 0, 0);
        red.id = 3;
        let palettes: HashMap<u64, Vec<[u8; 3]>> = HashMap::from([
            (1, vec![[10, 10, 10]]),    // black
            (2, vec![[250, 250, 250]]), // white
            (3, vec![[200, 30, 30]]),   // red
        ]);
        let color = Some([230, 40, 40]); // red
                                         // Ascending: nearest hue first → red image first, neutrals last.
        let sort = SortState {
            active: vec![(SortKey::Hue, SortDirection::Up)],
            color,
            color_level: 1,
            ..Default::default()
        };
        let mut illusts = vec![black, white, red];
        sort.sort(&mut illusts, &palettes);
        assert_eq!(illusts[0].id, 3, "the red image is closest");
        assert_eq!(illusts[1].id, 1, "a black image is neutral (f64::MAX)");
        assert_eq!(illusts[2].id, 2, "a white image is neutral too");

        // The distances directly: black is f64::MAX, red is a small angle.
        let d_red = SortKey::Hue.extract_value(&test_illust(3, 0, 0), color, 1, &palettes);
        assert!(d_red < 60.0, "red vs red ≈ {d_red}");
        let d_black = SortKey::Hue.extract_value(&test_illust(1, 0, 0), color, 1, &palettes);
        assert_eq!(d_black, f64::MAX, "black has no meaningful hue");
    }

    #[test]
    fn hue_target_color_neutral_sorts_nothing_first() {
        // A gray target color gives every entry f64::MAX (stable, no crash).
        let mut red = test_illust(1, 0, 0);
        red.id = 1;
        let palettes: HashMap<u64, Vec<[u8; 3]>> = HashMap::from([(1, vec![[255, 0, 0]])]);
        let sort = SortState {
            active: vec![(SortKey::Hue, SortDirection::Up)],
            color: Some([128, 128, 128]),
            color_level: 1,
            ..Default::default()
        };
        let mut illusts = vec![red];
        sort.sort(&mut illusts, &palettes);
        assert_eq!(illusts[0].id, 1);
    }
}
