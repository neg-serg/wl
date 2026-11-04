use std::path::PathBuf;

use walkdir::WalkDir;

/// Supported image extensions (case-insensitive).
pub const SUPPORTED_EXTENSIONS: &[&str] = &[
    "bmp", "gif", "hdr", "ico", "jpg", "jpeg", "png", "tif", "tiff", "webp",
];

/// Recursively scan directories for image files with supported extensions.
/// Warns on stderr for missing/unreadable directories, continues scanning others.
pub fn scan_directories(dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    for dir in dirs {
        if !dir.is_dir() {
            eprintln!("Warning: '{}' is not a directory, skipping", dir.display());
            continue;
        }

        for entry in WalkDir::new(dir).follow_links(true) {
            let entry = match entry {
                Ok(entry) => entry,
                Err(e) => {
                    // Permission denied / unreadable subtree: say so instead of
                    // silently returning a shorter list than the user expects.
                    eprintln!(
                        "Warning: failed to read an entry under '{}': {e}",
                        dir.display()
                    );
                    continue;
                }
            };
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            if let Some(ext) = path.extension() {
                let ext_lower = ext.to_string_lossy().to_lowercase();
                if SUPPORTED_EXTENSIONS.contains(&ext_lower.as_str()) {
                    candidates.push(path.to_path_buf());
                }
            }
        }
    }

    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("wl-scan-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn finds_supported_extensions_recursively_and_case_insensitively() {
        let dir = temp_dir("extensions");
        std::fs::create_dir_all(dir.join("nested")).unwrap();
        std::fs::write(dir.join("a.JPG"), b"x").unwrap();
        std::fs::write(dir.join("nested/b.png"), b"x").unwrap();
        std::fs::write(dir.join("notes.txt"), b"x").unwrap();
        std::fs::write(dir.join("nested/c.webp"), b"x").unwrap();

        let mut found: Vec<String> = scan_directories(std::slice::from_ref(&dir))
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        found.sort();

        assert_eq!(found, vec!["a.JPG", "b.png", "c.webp"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_directories_are_skipped() {
        let missing = std::env::temp_dir().join("wl-scan-definitely-missing");
        assert!(scan_directories(&[missing]).is_empty());
    }
}
