//! Persistent application state stored as a single `config.json` file.
//!
//! Everything the app needs to remember between launches lives in one JSON
//! document inside the per-user config directory:
//!
//! * Windows: `%APPDATA%\Elysium\config.json`
//! * Linux:   `~/.config/Elysium/config.json`
//!
//! Earlier versions stored each piece of state in its own `.txt` file. On first
//! launch we transparently migrate those legacy files (see [`migrate_from_txt`])
//! so existing users keep their likes, playlists and key bindings.

use std::collections::{HashMap, HashSet};

/// The complete on-disk configuration document.
///
/// `#[serde(default)]` means missing fields fall back to their `Default`, so
/// adding a new field never breaks an older `config.json`.
#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Config {
    /// File paths of liked tracks (the "Liked music" playlist).
    pub liked: Vec<String>,
    /// User-created / imported playlists.
    pub playlists: Vec<PlaylistData>,
    /// Names of playlists the user deleted, so they are not re-created on
    /// the next folder scan.
    pub deleted_playlists: Vec<String>,
    /// Interface language code, e.g. `"ru"` or `"uk"`.
    pub language: String,
    /// Maps a shortcut action code to a key name, or `"None"` when unbound.
    pub shortcuts: HashMap<String, String>,

    // --- First-run onboarding ---
    /// `true` once the user has finished (or skipped) the welcome wizard.
    pub onboarded: bool,
    /// Chosen theme: `"dark"` (default) or `"light"`. Saved by the onboarding
    /// wizard; the light theme is not applied yet.
    pub theme: String,
    /// Chosen accent color key: `"purple"`, `"teal"`, `"green"` or `"coral"`.
    pub accent: String,
    /// Folders the user picked as music sources during onboarding. When empty,
    /// the app falls back to the default [`crate::app::MUSIC_ROOT`].
    pub music_folders: Vec<String>,
    /// Individual audio files the user dragged in during onboarding.
    pub music_files: Vec<String>,
}

/// One playlist as stored on disk: a name plus its ordered track paths.
#[derive(Default, serde::Serialize, serde::Deserialize)]
pub struct PlaylistData {
    pub name: String,
    pub songs: Vec<String>,
}

/// Returns the path to `config.json`, creating the parent directory if needed.
///
/// Falls back to the current directory if the OS config directory cannot be
/// determined, so the app still runs (just storing config locally).
pub fn config_path() -> std::path::PathBuf {
    let mut dir = dirs::config_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    dir.push("Elysium");
    let _ = std::fs::create_dir_all(&dir);
    dir.push("config.json");
    dir
}

/// Returns the path to the persistent lyrics cache (`lyrics_cache.json`),
/// creating the parent directory if needed. Lives next to `config.json`.
pub fn lyrics_cache_path() -> std::path::PathBuf {
    let mut dir = dirs::config_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    dir.push("Elysium");
    let _ = std::fs::create_dir_all(&dir);
    dir.push("lyrics_cache.json");
    dir
}

/// Loads the full config.
///
/// Resolution order:
/// 1. Parse `config.json` if it exists (invalid JSON falls back to defaults).
/// 2. Otherwise try a one-time migration from legacy `.txt` files.
/// 3. Otherwise return an empty default config.
pub fn load_config() -> Config {
    if let Ok(text) = std::fs::read_to_string(config_path()) {
        return serde_json::from_str(&text).unwrap_or_default();
    }
    if let Some(cfg) = migrate_from_txt() {
        save_config(&cfg);
        return cfg;
    }
    Config::default()
}

/// Writes the full config back to disk as pretty-printed JSON.
///
/// Errors are logged but not propagated: failing to persist a like or a key
/// binding should never crash the player.
pub fn save_config(cfg: &Config) {
    match serde_json::to_string_pretty(cfg) {
        Ok(text) => {
            if let Err(e) = std::fs::write(config_path(), text) {
                println!("⚠️ Failed to save config: {:?}", e);
            }
        }
        Err(e) => println!("⚠️ Failed to serialize config: {:?}", e),
    }
}

/// One-time migration of legacy `.txt` state files into a [`Config`].
///
/// Looks for the old files in the working directory. Returns `Some(config)` if
/// at least one legacy file was found (so the caller can save it as JSON), or
/// `None` when there is nothing to migrate.
pub fn migrate_from_txt() -> Option<Config> {
    let mut cfg = Config::default();
    let mut found = false;

    // liked_songs.txt: one track path per line.
    if let Ok(c) = std::fs::read_to_string("liked_songs.txt") {
        cfg.liked = c
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();
        found = true;
    }

    // playlists.txt: tab-separated "name<TAB>track_path" lines. Lines sharing a
    // name belong to the same playlist; the first appearance fixes the order.
    if let Ok(c) = std::fs::read_to_string("playlists.txt") {
        let mut order: Vec<String> = Vec::new();
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        for line in c.lines() {
            if let Some((name, path)) = line.split_once('\t') {
                let (name, path) = (name.trim(), path.trim());
                if name.is_empty() {
                    continue;
                }
                if !map.contains_key(name) {
                    order.push(name.to_string());
                }
                let entry = map.entry(name.to_string()).or_default();
                if !path.is_empty() {
                    entry.push(path.to_string());
                }
            }
        }
        cfg.playlists = order
            .into_iter()
            .map(|name| {
                let songs = map.remove(&name).unwrap_or_default();
                PlaylistData { name, songs }
            })
            .collect();
        found = true;
    }

    // deleted_playlists.txt: one deleted playlist name per line.
    if let Ok(c) = std::fs::read_to_string("deleted_playlists.txt") {
        cfg.deleted_playlists = c
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();
        found = true;
    }

    // language.txt: a single language code.
    if let Ok(c) = std::fs::read_to_string("language.txt") {
        cfg.language = c.trim().to_string();
        found = true;
    }

    // shortcuts.txt: "action_code=key_name" lines.
    if let Ok(c) = std::fs::read_to_string("shortcuts.txt") {
        for line in c.lines() {
            if let Some((code, value)) = line.split_once('=') {
                cfg.shortcuts
                    .insert(code.trim().to_string(), value.trim().to_string());
            }
        }
        found = true;
    }

    if found {
        println!("📦 Legacy .txt files migrated into config.json — they can be deleted.");
        Some(cfg)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Thin convenience wrappers.
//
// These keep call sites small and intention-revealing; each one loads the full
// config and returns just the slice the caller cares about.
// ---------------------------------------------------------------------------

/// Returns the saved liked-track paths.
pub fn load_liked_songs() -> Vec<String> {
    load_config().liked
}

/// Returns saved playlists as `(name, songs)` pairs.
pub fn load_saved_playlists() -> Vec<(String, Vec<String>)> {
    load_config()
        .playlists
        .into_iter()
        .map(|p| (p.name, p.songs))
        .collect()
}

/// Returns the set of playlist names the user has deleted.
pub fn load_deleted_playlists() -> HashSet<String> {
    load_config().deleted_playlists.into_iter().collect()
}

/// Persists the set of deleted playlist names (stored sorted for stable diffs).
pub fn save_deleted_playlists(set: &HashSet<String>) {
    let mut names: Vec<String> = set.iter().cloned().collect();
    names.sort();
    let mut cfg = load_config();
    cfg.deleted_playlists = names;
    save_config(&cfg);
}
