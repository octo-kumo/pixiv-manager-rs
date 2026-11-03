//! Persisted MMCQ colour palettes, keyed by illust id.

use std::collections::HashMap;
use std::path::PathBuf;

pub fn colors_path() -> PathBuf {
    crate::io::paths::AppPaths::discover().colors_file()
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

/// Save the color map atomically through the shared writer.
///
/// Best effort: a failed colour save must never take the app down, it only
/// costs the user the (recomputable) palettes.
pub fn save_colors(palettes: &HashMap<u64, Vec<[u8; 3]>>) {
    if let Err(e) = crate::io::save_json_compact_atomic(palettes, &colors_path()) {
        log::warn!("[colors] save failed: {e}");
    }
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

    /// Palettes are re-saved on every colour extraction; overwriting the
    /// existing `colors.json` repeatedly must keep working on every OS.
    #[test]
    fn colors_survive_repeated_saves() {
        let _guard = COLORS_TEST_LOCK.lock().unwrap();
        crate::test_util::set_test_config_dir();

        for round in 1..=5u8 {
            let mut palettes = HashMap::new();
            palettes.insert(100u64 + u64::from(round), vec![[round, round, round]]);
            save_colors(&palettes);
            let loaded = load_colors();
            assert_eq!(
                loaded[&(100u64 + u64::from(round))],
                vec![[round, round, round]]
            );
        }
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
