//! Audio file helpers used by drag-and-drop import.
//!
//! This module knows nothing about playback; it only answers two questions:
//! "is this path an audio file?" and "which audio files live inside this
//! folder?". Both are used when the user drops files or folders onto the
//! window (see [`crate::app`]).

use std::path::Path;

/// Audio extensions the player is willing to import via drag-and-drop.
///
/// Note: actual decoding is handled by `rodio`; this list is intentionally a
/// bit broader than what is guaranteed to play, so that users are not silently
/// blocked from adding a file. Unsupported files simply fail to decode later.
const SUPPORTED_EXTENSIONS: &[&str] =
    &["mp3", "flac", "wav", "ogg", "m4a", "aac", "opus", "wma"];

/// Returns `true` if `path` has a known audio extension (case-insensitive).
pub fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .map(|ext| SUPPORTED_EXTENSIONS.contains(&ext.as_str()))
        .unwrap_or(false)
}

/// Recursively collects every audio file under `dir` into `out`.
///
/// Entries are visited in alphabetical order so that imported playlists keep a
/// stable, predictable track order. Directories are descended into depth-first.
/// Unreadable directories are skipped silently rather than aborting the scan.
pub fn collect_audio_files(dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    // Sort up front so the traversal order does not depend on filesystem order.
    let mut items: Vec<std::path::PathBuf> = entries.flatten().map(|e| e.path()).collect();
    items.sort();

    for path in items {
        if path.is_dir() {
            collect_audio_files(&path, out);
        } else if is_audio_file(&path) {
            if let Some(s) = path.to_str() {
                out.push(s.to_string());
            }
        }
    }
}
