use std::path::{Path, PathBuf};

// Re-export scan_directories from common crate
pub use wl_common::scan::scan_directories;

/// Pick a random element from candidates using OS randomness.
pub fn pick_random(candidates: &[PathBuf]) -> &Path {
    assert!(!candidates.is_empty(), "pick_random called with empty list");
    let mut buf = [0u8; 8];
    getrandom::fill(&mut buf).expect("failed to get random bytes");
    let index = u64::from_ne_bytes(buf) as usize % candidates.len();
    &candidates[index]
}

/// Copy `source` over `dest` atomically (sibling temp file + rename).
///
/// The greeter (or any other consumer polling the path) must never see a
/// half-copied image.
fn atomic_copy(source: &Path, dest: &Path) -> std::io::Result<()> {
    let file_name = dest
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "wallpaper".to_string());
    let tmp = dest.with_file_name(format!(".{file_name}.{}.tmp", std::process::id()));

    let result = std::fs::copy(source, &tmp).and_then(|_| std::fs::rename(&tmp, dest));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Copy a wallpaper image to the greeter cache path.
/// Creates parent directories as needed. Non-fatal: errors are printed to stderr.
pub fn greeter_sync(source: &Path, dest: &Path) {
    if let Some(parent) = dest.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        eprintln!("Warning: failed to create greeter cache directory: {e}");
        return;
    }
    if let Err(e) = atomic_copy(source, dest) {
        eprintln!("Warning: failed to sync greeter wallpaper: {e}");
    }
}

/// Write the wallpaper absolute path to a notification file.
/// Creates parent directories as needed. Non-fatal: errors are printed to stderr.
pub fn write_notify(wallpaper_path: &Path, notify_file: &Path) {
    if let Some(parent) = notify_file.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        eprintln!("Warning: failed to create notify file directory: {e}");
        return;
    }
    let abs_path = wallpaper_path
        .canonicalize()
        .unwrap_or_else(|_| wallpaper_path.to_path_buf());
    // Pollers read this file; a plain write can expose a truncated path.
    if let Err(e) =
        wl_common::cache::write_atomic(notify_file, abs_path.to_string_lossy().as_bytes())
    {
        eprintln!("Warning: failed to write notification file: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!("wl-random-{tag}-{}", std::process::id()));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn leftovers(dir: &Path) -> usize {
        std::fs::read_dir(dir)
            .unwrap()
            .filter(|entry| {
                entry
                    .as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".tmp")
            })
            .count()
    }

    #[test]
    fn greeter_sync_replaces_the_destination_without_temp_files() {
        let dir = TempDir::new("greeter");
        let source = dir.0.join("source.png");
        let dest = dir.0.join("greeter.png");
        std::fs::write(&source, b"new wallpaper").unwrap();
        std::fs::write(&dest, b"old wallpaper").unwrap();

        greeter_sync(&source, &dest);

        assert_eq!(std::fs::read(&dest).unwrap(), b"new wallpaper");
        assert_eq!(leftovers(&dir.0), 0);
    }

    #[test]
    fn write_notify_leaves_no_temp_file_and_an_absolute_path() {
        let dir = TempDir::new("notify");
        let image = dir.0.join("wall.jpg");
        let notify = dir.0.join("notify");
        std::fs::write(&image, b"x").unwrap();

        write_notify(&image, &notify);

        let written = std::fs::read_to_string(&notify).unwrap();
        assert!(std::path::Path::new(&written).is_absolute(), "{written}");
        assert_eq!(leftovers(&dir.0), 0);
    }
}
