//! Portability regressions: names and paths that are legal on one OS but not
//! on another.

use std::path::PathBuf;

use pixiv_manager::core::{download_dir, file_extension, image_filename, sanitize_filename};

#[test]
fn reserved_windows_names_are_defanged() {
    for name in ["CON", "PRN", "AUX", "NUL", "COM1", "LPT9", "con", "nul"] {
        let sanitized = sanitize_filename(name);
        assert_ne!(
            sanitized.to_ascii_uppercase(),
            name.to_ascii_uppercase(),
            "{name:?} must not stay a reserved device name"
        );
        assert!(!sanitized.is_empty());
    }
}

/// Reserved names stay reserved with an extension (`CON.txt`).
#[test]
fn reserved_names_with_extensions_are_defanged() {
    for name in ["CON.txt", "nul.log", "COM1.tar.gz"] {
        let sanitized = sanitize_filename(name);
        assert!(
            !sanitized.to_ascii_uppercase().starts_with(
                name.split('.')
                    .next()
                    .unwrap()
                    .to_ascii_uppercase()
                    .as_str()
            ),
            "{name:?} kept a reserved device name: {sanitized:?}"
        );
    }
}

#[test]
fn windows_invalid_characters_are_removed() {
    let sanitized = sanitize_filename(r#"a<b>c:d"e/f\g|h?i*j"#);
    for bad in ['<', '>', ':', '"', '/', '\\', '|', '?', '*'] {
        assert!(
            !sanitized.contains(bad),
            "{bad:?} survived in {sanitized:?}"
        );
    }
}

#[test]
fn trailing_dots_and_spaces_are_trimmed() {
    assert_eq!(sanitize_filename("name...  "), "name");
    assert_eq!(sanitize_filename("name . . "), "name");
}

#[test]
fn unicode_names_survive() {
    assert_eq!(sanitize_filename("初音ミク ✓"), "初音ミク ✓");
    assert_eq!(sanitize_filename("エモい イラスト"), "エモい イラスト");
}

#[test]
fn download_paths_are_pure_integers() {
    let gallery = PathBuf::from("gallery");
    let dir = download_dir(&gallery, 12345, "a title with ../ and \\ in it");
    assert_eq!(dir, gallery.join("12345"));
    assert_eq!(image_filename(12345, 0, 2, "jpg"), "0.jpg");
    assert_eq!(image_filename(12345, 1, 2, "png"), "1.png");
    assert_eq!(file_extension("https://example.com/a/b.jpg?x=1"), "jpg");
}

#[test]
fn writes_into_a_unicode_directory_with_spaces() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("My Pictures").join("初音 ミク ✓");
    let path = dir.join("0.jpg");

    pixiv_manager::io::write_atomic(&path, b"first").unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"first");
    // Repeated replacement in the same directory.
    pixiv_manager::io::write_atomic(&path, b"second").unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"second");
}

#[test]
fn long_nested_paths_round_trip() {
    let root = tempfile::tempdir().unwrap();
    let mut dir = root.path().to_path_buf();
    // ~150 characters of nesting: exercises deep path handling without
    // depending on long-path opt-in.
    for i in 0..10 {
        dir = dir.join(format!("level-{i}-padding"));
    }
    let path = dir.join("illust.json");

    pixiv_manager::io::save_json_atomic(&serde_json::json!({"id": 7}), &path).unwrap();
    let read: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(read["id"], 7);
}
