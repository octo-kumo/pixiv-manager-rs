//! Centralized, persistent color storage.
//!
//! Every illust's MMCQ palette (5 representative colors) lives here, keyed by
//! illust id and persisted to a single `colors.json` in the data dir — the
//! single source of truth for the sRGB/Hue color sorts, the viewer swatches
//! and the bookmark "process all" metadata pass. Runtime access is through
//! [`crate::app::state::ImageState::palettes`]; this module owns load/save.

use std::collections::HashMap;
use std::path::PathBuf;

/// Directory holding the color map (alongside the library).
fn colors_dir() -> PathBuf {
    crate::io::library::library_dir()
}

/// Where the color map lives.
pub fn colors_path() -> PathBuf {
    colors_dir().join("colors.json")
}

/// Load the persisted color map (missing/corrupt → empty). Entries with an
/// empty palette are dropped — an empty palette means "not yet extracted".
pub fn load_colors() -> HashMap<u64, Vec<[u8; 3]>> {
    let map: HashMap<u64, Vec<[u8; 3]>> = match std::fs::read_to_string(colors_path()) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => HashMap::new(),
    };
    map.into_iter().filter(|(_, p)| !p.is_empty()).collect()
}

/// Save the color map atomically (temp file + rename).
pub fn save_colors(palettes: &HashMap<u64, Vec<[u8; 3]>>) {
    use std::io::Write;
    if let Some(parent) = colors_path().parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(content) = serde_json::to_string(palettes) else {
        return;
    };
    let tmp = colors_path().with_extension("tmp");
    let mut f = std::fs::File::create(&tmp).ok();
    if let Some(f) = f.as_mut() {
        let _ = f.write_all(content.as_bytes());
        let _ = f.flush();
    }
    drop(f);
    let _ = std::fs::rename(&tmp, colors_path());
}

#[cfg(test)]
mod tests {
    use super::*;

    // The two colors tests write to the same shared `colors.json` (all tests
    // share one temp config dir), so serialize them — otherwise they race on
    // the file when the suite runs in parallel.
    static COLORS_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn colors_roundtrip_through_disk() {
        let _guard = COLORS_TEST_LOCK.lock().unwrap();
        crate::test_util::set_test_config_dir();

        let mut palettes = HashMap::new();
        palettes.insert(1u64, vec![[255, 0, 0], [0, 255, 0]]);
        palettes.insert(2u64, vec![[10, 20, 30]]);
        save_colors(&palettes);

        let loaded = load_colors();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[&1], vec![[255, 0, 0], [0, 255, 0]]);
        assert_eq!(loaded[&2], vec![[10, 20, 30]]);
    }

    #[test]
    fn empty_palettes_are_dropped_on_load() {
        let _guard = COLORS_TEST_LOCK.lock().unwrap();
        crate::test_util::set_test_config_dir();

        let mut palettes = HashMap::new();
        palettes.insert(1u64, vec![[1, 2, 3]]);
        palettes.insert(2u64, Vec::new()); // a failed extraction artifact
        save_colors(&palettes);
        let loaded = load_colors();
        assert!(loaded.contains_key(&1));
        assert!(!loaded.contains_key(&2), "empty palettes count as missing");
    }
}
