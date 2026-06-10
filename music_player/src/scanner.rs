//! Library scanning and song-lyrics retrieval.
//!
//! Two unrelated responsibilities live here, both I/O-heavy and run off the UI
//! thread:
//!
//! * **Scanning** ([`scan_music`]): turn a music folder into [`Playlist`]s —
//!   each subfolder with MP3s becomes a playlist, and loose MP3s in the root
//!   become an "all tracks" playlist.
//! * **Lyrics** ([`get_synced_lyrics`], [`fetch_lyrics_from_internet`]): find
//!   time-synced ("karaoke") lyrics, first from the file's own ID3 tags, then
//!   from online providers (lrclib, then NetEase).

use id3::{Tag, TagLike};
use reqwest::blocking::Client;
use serde::Deserialize;
use std::fs;

/// A named group of track file paths.
#[derive(Clone)]
pub struct Playlist {
    pub name: String,
    pub songs: Vec<String>,
}

/// Scans `root` and builds playlists from its contents.
///
/// Layout rules:
/// * Each immediate subfolder containing `.mp3` files becomes a playlist named
///   after the folder.
/// * Loose `.mp3` files directly in `root` are grouped into a single
///   "Усі треки" (all tracks) playlist.
///
/// Returns an empty vector (after logging) if `root` is missing or has no MP3s.
pub fn scan_music(root: &str) -> Vec<Playlist> {
    let mut playlists = vec![];

    println!("🔍 Scanning music folder: {}", root);

    let mut root_songs = vec![];

    let Ok(entries) = fs::read_dir(root) else {
        println!("❌ ERROR: could not open or find folder '{}'!", root);
        println!("Make sure it sits next to the 'music_player' folder, not inside it.");
        return playlists;
    };

    for entry in entries.flatten() {
        let path = entry.path();

        if path.is_dir() {
            // A subfolder becomes a playlist named after the folder.
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            let mut songs = vec![];

            if let Ok(files) = fs::read_dir(&path) {
                for file in files.flatten() {
                    let file_path = file.path();
                    if file_path.extension().is_some_and(|ext| ext == "mp3") {
                        songs.push(file_path.to_string_lossy().to_string());
                    }
                }
            }

            if !songs.is_empty() {
                println!("📁 Found playlist '{}' ({} songs)", name, songs.len());
                playlists.push(Playlist { name, songs });
            }
        } else if path.is_file() && path.extension().is_some_and(|ext| ext == "mp3") {
            // A loose MP3 in the root folder.
            root_songs.push(path.to_string_lossy().to_string());
        }
    }

    if !root_songs.is_empty() {
        println!("🎵 Found {} tracks directly in '{}'", root_songs.len(), root);
        playlists.push(Playlist {
            name: "Усі треки".to_string(),
            songs: root_songs,
        });
    }

    if playlists.is_empty() {
        println!("⚠️ WARNING: no MP3 files found at the given path.");
    }

    playlists
}

/// One timestamped line of lyrics.
#[derive(Debug, Clone)]
pub struct LyricLine {
    /// When this line appears, in milliseconds from the start of the track.
    pub time_ms: u32,
    /// The line's text.
    pub text: String,
}

/// Tries to extract synchronized lyrics (the ID3 `SYLT` frame) from a file.
///
/// Returns `None` if the file has no tags or no synced-lyrics frame.
pub fn get_synced_lyrics(file_path: &str) -> Option<Vec<LyricLine>> {
    println!("🎤 Looking for embedded lyrics in: {}", file_path);

    let tag = match id3::Tag::read_from_path(file_path) {
        Ok(t) => {
            println!("✅ File has ID3 tags. Searching for karaoke lyrics (SYLT)...");
            t
        }
        Err(e) => {
            println!("❌ Could not read tags (or none present): {}", e);
            return None;
        }
    };

    let mut found = false;
    for sync_lyric in tag.synchronised_lyrics() {
        found = true;
        let mut lines = Vec::new();
        for (time, text) in &sync_lyric.content {
            // Drop credit/metadata lines our font cannot render (see
            // `contains_unrenderable`).
            if contains_unrenderable(text) {
                continue;
            }
            lines.push(LyricLine {
                time_ms: *time,
                text: text.to_string(),
            });
        }
        if !lines.is_empty() {
            println!("🎉 Synchronized lyrics found!");
            return Some(lines);
        }
    }

    if !found {
        println!("😔 This file has no synchronized lyrics (SYLT).");
    }
    None
}

/// Subset of the lrclib API response we care about.
#[derive(Deserialize)]
struct LrcResponse {
    duration: Option<f64>,
    #[serde(rename = "syncedLyrics")]
    synced_lyrics: Option<String>,
}

/// Reads `(title, artist, album)` from a file's ID3 tags, with sensible
/// fallbacks for messy downloaded files.
///
/// If the title is empty, the file stem is used. A common pattern in downloaded
/// music is a title of the form `"Artist - Track"` with a junk artist tag; when
/// detected, the title is split so the real artist and track are recovered.
fn read_track_meta(file_path: &str) -> (String, String, String) {
    let mut title = String::new();
    let mut artist = String::new();
    let mut album = String::new();

    if let Ok(tag) = Tag::read_from_path(file_path) {
        title = tag.title().unwrap_or("").trim().to_string();
        artist = tag.artist().unwrap_or("").trim().to_string();
        album = tag.album().unwrap_or("").trim().to_string();
    }

    // No title tag: fall back to the file name without extension.
    if title.is_empty() {
        title = std::path::Path::new(file_path)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
    }

    // Recover "Artist - Track" titles, which are more reliable than the tags.
    if let Some((a, t)) = title.split_once(" - ") {
        let (a, t) = (a.trim(), t.trim());
        if !a.is_empty() && !t.is_empty() {
            artist = a.to_string();
            title = t.to_string();
        }
    }

    (title, artist, album)
}

/// Searches online providers for synchronized lyrics matching the track.
///
/// Providers are queried in order and the first hit wins. `duration` (when
/// known) sharpens matching so we do not grab lyrics for the wrong edit of a
/// song. To add a provider, query it in the chain below.
pub fn fetch_lyrics_from_internet(
    file_path: &str,
    duration: Option<std::time::Duration>,
) -> Option<Vec<LyricLine>> {
    let (title, artist, album) = read_track_meta(file_path);
    if title.is_empty() {
        println!("❌ Could not determine the track title.");
        return None;
    }

    let dur_secs = duration.map(|d| d.as_secs());

    if let Some(l) = lrclib_lyrics(&title, &artist, &album, dur_secs) {
        return Some(l);
    }
    if let Some(l) = netease_lyrics(&title, &artist, dur_secs) {
        return Some(l);
    }

    println!("❌ No matching lyrics found in any source.");
    None
}

/// Provider #1: lrclib.net — accurate, well-synced lyrics.
///
/// Tries an exact `/api/get` lookup first (which honors duration within ±2s),
/// then falls back to `/api/search` and picks the best result by duration.
fn lrclib_lyrics(
    title: &str,
    artist: &str,
    album: &str,
    dur_secs: Option<u64>,
) -> Option<Vec<LyricLine>> {
    println!("🌐 [lrclib] Searching: artist='{}', track='{}'", artist, title);

    let client = Client::builder().user_agent("Elysium/1.0.2").build().ok()?;

    // --- Strategy 1: exact match via /api/get (uses duration, ±2s). ---
    if !artist.is_empty() {
        if let Some(secs) = dur_secs {
            let dur_str = secs.to_string();
            let resp = client
                .get("https://lrclib.net/api/get")
                .query(&[
                    ("track_name", title),
                    ("artist_name", artist),
                    ("album_name", album),
                    ("duration", dur_str.as_str()),
                ])
                .send();

            if let Ok(r) = resp {
                if r.status().is_success() {
                    if let Ok(rec) = r.json::<LrcResponse>() {
                        if let Some(s) = rec.synced_lyrics {
                            if !s.trim().is_empty() {
                                println!("✅ [lrclib] Exact match (api/get).");
                                return parse_lrc_string(&s);
                            }
                        }
                    }
                }
            }
        }
    }

    // --- Strategy 2: search by artist + title, choose the closest duration. ---
    let mut req = client
        .get("https://lrclib.net/api/search")
        .query(&[("track_name", title)]);
    if !artist.is_empty() {
        req = req.query(&[("artist_name", artist)]);
    }

    let results: Vec<LrcResponse> = req.send().ok()?.json().ok()?;

    let mut best: Option<&LrcResponse> = None;
    for r in &results {
        let has_synced = r
            .synced_lyrics
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty());
        if !has_synced {
            continue;
        }
        match (dur_secs, r.duration) {
            // Both durations known: accept the first within 3 seconds.
            (Some(ours), Some(theirs)) => {
                if (theirs - ours as f64).abs() <= 3.0 {
                    best = Some(r);
                    break;
                }
            }
            // Duration unknown on either side: keep the first synced result.
            _ => {
                if best.is_none() {
                    best = Some(r);
                }
            }
        }
    }

    if let Some(r) = best {
        if let Some(s) = &r.synced_lyrics {
            println!("✅ [lrclib] Found via search.");
            return parse_lrc_string(s);
        }
    }

    println!("❌ [lrclib] Not found.");
    None
}

// --- Provider #2: NetEase Cloud Music response types ---

#[derive(Deserialize)]
struct NeteaseSearch {
    result: Option<NeteaseResult>,
}
#[derive(Deserialize)]
struct NeteaseResult {
    songs: Option<Vec<NeteaseSong>>,
}
#[derive(Deserialize)]
struct NeteaseSong {
    id: u64,
    name: Option<String>,
    /// Duration in MILLIseconds (NetEase's unit).
    duration: Option<u64>,
}
#[derive(Deserialize)]
struct NeteaseLyricResp {
    lrc: Option<NeteaseLrc>,
}
#[derive(Deserialize)]
struct NeteaseLrc {
    lyric: Option<String>,
}

/// Normalizes a string for fuzzy comparison: lowercase, letters and digits only.
fn normalize(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

/// Returns `true` if two titles are "close enough": after normalization, one
/// contains the other.
fn titles_match(a: &str, b: &str) -> bool {
    let na = normalize(a);
    let nb = normalize(b);
    !na.is_empty() && !nb.is_empty() && (na.contains(&nb) || nb.contains(&na))
}

/// Provider #2: NetEase Cloud Music — huge catalog, good for covers and
/// non-English songs.
///
/// NetEase is picky about headers, so we pose as a browser and set a `Referer`.
/// A candidate is accepted only if its title is similar (and, when duration is
/// known, within ±5 seconds), which filters out random matches.
fn netease_lyrics(title: &str, artist: &str, dur_secs: Option<u64>) -> Option<Vec<LyricLine>> {
    let query = if artist.is_empty() {
        title.to_string()
    } else {
        format!("{} {}", artist, title)
    };
    println!("🌐 [NetEase] Searching: {}", query);

    let client = Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36")
        .build()
        .ok()?;

    // 1. Search for the track to obtain its id (type=1 means "songs").
    let search: NeteaseSearch = client
        .get("https://music.163.com/api/search/get")
        .header("Referer", "https://music.163.com")
        .query(&[("s", query.as_str()), ("type", "1"), ("limit", "10")])
        .send()
        .ok()?
        .json()
        .ok()?;

    let songs = search.result?.songs?;
    if songs.is_empty() {
        println!("❌ [NetEase] Track not found.");
        return None;
    }

    // Pick a song whose title is similar and (if we know our duration) close in
    // length. NetEase reports milliseconds, so divide by 1000 to compare.
    let song = songs.iter().find(|s| {
        let name_ok = s.name.as_deref().is_some_and(|n| titles_match(n, title));
        if !name_ok {
            return false;
        }
        match dur_secs {
            Some(ours) => {
                let theirs = s.duration.unwrap_or(0) / 1000;
                (theirs as i64 - ours as i64).abs() <= 5
            }
            None => true,
        }
    });

    let song = match song {
        Some(s) => s,
        None => {
            println!("❌ [NetEase] No similar track (title/duration mismatch).");
            return None;
        }
    };

    // 2. Fetch lyrics by song id.
    let id_str = song.id.to_string();
    let lyric: NeteaseLyricResp = client
        .get("https://music.163.com/api/song/lyric")
        .header("Referer", "https://music.163.com")
        .query(&[("id", id_str.as_str()), ("lv", "-1"), ("kv", "-1"), ("tv", "-1")])
        .send()
        .ok()?
        .json()
        .ok()?;

    let lrc = lyric.lrc?.lyric?;
    if lrc.trim().is_empty() {
        println!("😔 [NetEase] Track has no synced lyrics.");
        return None;
    }

    println!("✅ [NetEase] Lyrics found!");
    parse_lrc_string(&lrc)
}

/// Parses raw LRC text (lines like `[mm:ss.xx] text`) into timed [`LyricLine`]s.
///
/// Lines without a valid `[mm:ss]` timestamp are skipped. Fractional seconds
/// are interpreted by length: 2 digits are centiseconds (×10), 1 digit is
/// deciseconds (×100), 3+ digits are taken as milliseconds. Empty lines are
/// kept as a single space so the lyrics view preserves spacing. Returns `None`
/// when nothing parseable was found.
/// Returns `true` if `s` contains characters the bundled font cannot render
/// (East Asian scripts), which would otherwise appear as empty boxes.
///
/// The font covers Latin, Cyrillic and emoji but no CJK/Japanese/Korean, so
/// such text is always provider metadata (composer/lyricist credits) rather
/// than singable lyrics — dropping it is strictly better than showing boxes.
fn contains_unrenderable(s: &str) -> bool {
    s.chars().any(|c| {
        matches!(c as u32,
            0x3000..=0x303F   // CJK symbols and punctuation
            | 0x3040..=0x30FF // Hiragana + Katakana
            | 0x3400..=0x4DBF // CJK Extension A
            | 0x4E00..=0x9FFF // CJK Unified Ideographs
            | 0xAC00..=0xD7AF // Hangul syllables
            | 0xF900..=0xFAFF // CJK compatibility ideographs
            | 0xFF00..=0xFFEF // Halfwidth/Fullwidth forms
        )
    })
}

pub fn parse_lrc_string(content: &str) -> Option<Vec<LyricLine>> {
    let mut lines = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if !line.starts_with('[') {
            continue;
        }
        let Some(close_idx) = line.find(']') else {
            continue;
        };

        let time_str = &line[1..close_idx];
        let text = line[close_idx + 1..].trim().to_string();

        // Skip lines our font cannot render (NetEase prepends East Asian credit
        // lines like "作词 : ..." which would show as missing-glyph boxes).
        if contains_unrenderable(&text) {
            continue;
        }

        let parts: Vec<&str> = time_str.split(':').collect();
        if parts.len() != 2 {
            continue;
        }

        let min: u32 = parts[0].parse().unwrap_or(0);
        let sec_parts: Vec<&str> = parts[1].split('.').collect();
        let sec: u32 = sec_parts[0].parse().unwrap_or(0);

        let ms: u32 = if sec_parts.len() > 1 {
            let ms_str = sec_parts[1];
            let ms_val: u32 = ms_str.parse().unwrap_or(0);
            match ms_str.len() {
                2 => ms_val * 10,  // centiseconds
                1 => ms_val * 100, // deciseconds
                _ => ms_val,       // already milliseconds
            }
        } else {
            0
        };

        let time_ms = (min * 60 * 1000) + (sec * 1000) + ms;
        let display_text = if text.is_empty() {
            " ".to_string()
        } else {
            text
        };
        lines.push(LyricLine { time_ms, text: display_text });
    }

    if lines.is_empty() {
        None
    } else {
        Some(lines)
    }
}
