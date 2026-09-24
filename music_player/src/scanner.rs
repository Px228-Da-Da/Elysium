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

use crate::audio::is_audio_file;
use reqwest::blocking::Client;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::sync::{Mutex, OnceLock};

/// A named group of track file paths.
#[derive(Clone)]
pub struct Playlist {
    pub name: String,
    pub songs: Vec<String>,
}

/// Scans `root` and builds playlists from its contents.
///
/// Layout rules:
/// * Each immediate subfolder containing audio files becomes a playlist named
///   after the folder.
/// * Loose audio files directly in `root` are grouped into a single
///   "Усі треки" (all tracks) playlist.
///
/// Every format the player can import is picked up (MP3, FLAC, M4A, OGG, Opus,
/// WAV, …), not just MP3 — see [`crate::audio::is_audio_file`].
///
/// Returns an empty vector (after logging) if `root` is missing or has no audio.
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
                    if is_audio_file(&file_path) {
                        songs.push(file_path.to_string_lossy().to_string());
                    }
                }
            }

            if !songs.is_empty() {
                println!("📁 Found playlist '{}' ({} songs)", name, songs.len());
                playlists.push(Playlist { name, songs });
            }
        } else if path.is_file() && is_audio_file(&path) {
            // A loose audio file in the root folder.
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
        println!("⚠️ WARNING: no audio files found at the given path.");
    }

    playlists
}

/// Scans several root folders and concatenates their playlists.
///
/// Used when the user picked one or more music sources during onboarding. Each
/// root is scanned with [`scan_music`]; the results are merged in order.
pub fn scan_music_paths(roots: &[String]) -> Vec<Playlist> {
    let mut out = Vec::new();
    for root in roots {
        out.extend(scan_music(root));
    }
    out
}

/// One timestamped line of lyrics.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
    #[serde(rename = "trackName")]
    track_name: Option<String>,
    #[serde(rename = "artistName")]
    artist_name: Option<String>,
    duration: Option<f64>,
    #[serde(rename = "syncedLyrics")]
    synced_lyrics: Option<String>,
    /// Plain, un-timed lyrics — used as a fallback when no synced version
    /// exists for a track (many songs only have these).
    #[serde(rename = "plainLyrics")]
    plain_lyrics: Option<String>,
}

/// Returns `true` if these lines carry real timestamps (a "karaoke" track).
///
/// Un-timed (plain) lyrics are represented as [`LyricLine`]s that all sit at
/// `time_ms == 0`; a synced track always has at least one later line, so any
/// non-zero timestamp means the lyrics are synced.
pub fn is_synced(lines: &[LyricLine]) -> bool {
    lines.iter().any(|l| l.time_ms != 0)
}

/// Reads `(title, artist, album)` from a file's tags, with sensible fallbacks
/// for messy downloaded files.
///
/// Works for every supported format (not just MP3) via `lofty`. If the title is
/// empty, the file stem is used. A common pattern in downloaded music is a title
/// of the form `"Artist - Track"` with a junk artist tag; when detected, the
/// title is split so the real artist and track are recovered.
fn read_track_meta(file_path: &str) -> (String, String, String) {
    let raw = crate::meta::read_raw_meta(file_path);
    let mut title = raw.title.unwrap_or_default();
    let mut artist = raw.artist.unwrap_or_default();
    let album = raw.album.unwrap_or_default();

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
/// Providers are queried **in parallel** (each on its own thread) so total time
/// is the slowest single provider rather than their sum. lrclib is still
/// preferred: if it returns a result we use it, otherwise we fall back to the
/// NetEase result that was fetched concurrently. `duration` (when known)
/// sharpens matching so we do not grab lyrics for the wrong edit of a song.
pub fn fetch_lyrics_from_internet(
    file_path: &str,
    duration: Option<std::time::Duration>,
) -> Option<Vec<LyricLine>> {
    let (title, artist, album) = read_track_meta(file_path);
    fetch_lyrics_by_tags(&title, &artist, &album, duration)
}

/// Like [`fetch_lyrics_from_internet`] but takes the `title`/`artist` directly
/// instead of reading them from a file's tags. Used by the YouTube tab, where
/// the track is a remote stream with no local file to read.
pub fn fetch_lyrics_by_tags(
    title: &str,
    artist: &str,
    album: &str,
    duration: Option<std::time::Duration>,
) -> Option<Vec<LyricLine>> {
    if title.trim().is_empty() {
        println!("❌ Could not determine the track title.");
        return None;
    }

    let dur_secs = duration.map(|d| d.as_secs());
    let key = cache_key(title, artist, dur_secs);

    // 1. Persistent cache: an instant answer for anything looked up before —
    //    including a remembered "no lyrics" so we don't re-hit the network on
    //    every replay of a track that has none.
    if let Some(hit) = cache_lookup(&key) {
        match &hit {
            Some(l) => println!("⚡ [cache] {} lyric lines.", l.len()),
            None => println!("⚡ [cache] Known to have no lyrics."),
        }
        return hit;
    }

    // 2. Query lrclib and NetEase concurrently. lrclib is the primary source
    //    (huge, accurate, synced); NetEase is a backup. We return the moment
    //    lrclib yields a *synced* hit — no need to wait for the slower NetEase.
    let _ = album; // kept in the signature for callers; lrclib ranks locally.
    let (lrc_title, lrc_artist) = (title.to_string(), artist.to_string());
    let (net_title, net_artist) = (title.to_string(), artist.to_string());

    let lrclib = std::thread::spawn(move || lrclib_lyrics(&lrc_title, &lrc_artist, dur_secs));
    let netease = std::thread::spawn(move || netease_lyrics(&net_title, &net_artist, dur_secs));

    let lrc = lrclib.join().ok().flatten();
    // Fast path: lrclib already has synced lyrics — take them and let the
    // NetEase thread wind down on its own.
    if lrc.as_ref().is_some_and(|l| is_synced(l)) {
        cache_store(&key, &lrc);
        return lrc;
    }

    let net = netease.join().ok().flatten();
    // Prefer synced from either source, then plain (lrclib preferred).
    let synced = [lrc.as_ref(), net.as_ref()]
        .into_iter()
        .flatten()
        .find(|l| is_synced(l))
        .cloned();
    let result = synced.or(lrc).or(net);

    match &result {
        Some(l) if is_synced(l) => println!("✅ Synced lyrics found."),
        Some(_) => println!("ℹ️ Only un-synced (plain) lyrics available."),
        None => println!("❌ No matching lyrics found in any source."),
    }
    cache_store(&key, &result);
    result
}

// ---------------------------------------------------------------------------
// Persistent lyrics cache
//
// Every resolved lookup — including a "no lyrics" result — is remembered on disk
// (`lyrics_cache.json`), so replaying a track never re-hits the network. This is
// the biggest win for perceived speed: the slowest lookups are exactly the
// tracks with no lyrics (every provider is tried in full), and now they cost one
// file read on the second play. Negative results expire after `NEG_TTL_SECS` so
// a song that later gains lyrics is eventually retried.
// ---------------------------------------------------------------------------

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct LyricCacheEntry {
    found: bool,
    #[serde(default)]
    lines: Vec<LyricLine>,
    ts: u64,
}

/// How long a "no lyrics found" result stays cached (30 days).
const NEG_TTL_SECS: u64 = 60 * 60 * 24 * 30;

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Cache key: normalized title + artist + duration, so different-length edits of
/// a song don't collide and messy tag casing/spacing still hits.
fn cache_key(title: &str, artist: &str, dur_secs: Option<u64>) -> String {
    format!(
        "{}|{}|{}",
        normalize(title),
        normalize(artist),
        dur_secs.map(|d| d.to_string()).unwrap_or_default()
    )
}

fn cache_map() -> &'static Mutex<HashMap<String, LyricCacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<String, LyricCacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| {
        let map = std::fs::read_to_string(crate::config::lyrics_cache_path())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        Mutex::new(map)
    })
}

/// `Some(Some(lines))` = remembered hit; `Some(None)` = remembered miss still
/// within its TTL; `None` = nothing usable, the caller should fetch.
fn cache_lookup(key: &str) -> Option<Option<Vec<LyricLine>>> {
    let map = cache_map().lock().ok()?;
    let e = map.get(key)?;
    if e.found {
        Some(Some(e.lines.clone()))
    } else if now_secs().saturating_sub(e.ts) < NEG_TTL_SECS {
        Some(None)
    } else {
        None
    }
}

fn cache_store(key: &str, result: &Option<Vec<LyricLine>>) {
    if let Ok(mut map) = cache_map().lock() {
        map.insert(
            key.to_string(),
            LyricCacheEntry {
                found: result.is_some(),
                lines: result.clone().unwrap_or_default(),
                ts: now_secs(),
            },
        );
        if let Ok(text) = serde_json::to_string(&*map) {
            let _ = std::fs::write(crate::config::lyrics_cache_path(), text);
        }
    }
}

/// Runs a blocking request, retrying once on a transient failure (timeout,
/// dropped connection). Kept deliberately short — a single retry recovers from
/// a flaky moment without turning a dead provider into a multi-second stall.
fn send_with_retry<F>(mut make_request: F) -> reqwest::Result<reqwest::blocking::Response>
where
    F: FnMut() -> reqwest::Result<reqwest::blocking::Response>,
{
    const ATTEMPTS: u32 = 2;
    let mut last_err = None;
    for attempt in 1..=ATTEMPTS {
        match make_request() {
            Ok(resp) => return Ok(resp),
            Err(e) => {
                if attempt < ATTEMPTS {
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
                last_err = Some(e);
            }
        }
    }
    Err(last_err.expect("at least one attempt always runs"))
}

/// Builds an HTTP client with tight timeouts so a slow provider fails fast
/// instead of freezing the lyrics lookup.
fn http_client(user_agent: &str) -> Option<Client> {
    Client::builder()
        .user_agent(user_agent.to_string())
        .connect_timeout(std::time::Duration::from_secs(4))
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .ok()
}

/// Provider #1: lrclib.net — huge, accurate, synced-lyrics database.
///
/// Two-step, tuned for speed *and* coverage:
/// 1. A **narrow** `track_name + artist_name` search — a small, fast response
///    that resolves cleanly-tagged (usually mainstream) tracks immediately.
/// 2. If that finds no synced match, a **broad** title-only search — needed for
///    messy/multi-artist tags ("UMBRELLA, VAMPXL") that don't match lrclib's
///    artist field. Both are ranked locally by duration + artist similarity.
///
/// Prefers a synced version; falls back to plain (un-timed) lyrics.
fn lrclib_lyrics(title: &str, artist: &str, dur_secs: Option<u64>) -> Option<Vec<LyricLine>> {
    println!("🌐 [lrclib] Searching: artist='{}', track='{}'", artist, title);
    let client = http_client("Elysium/1.2.1")?;

    // Step 1: narrow search. Return only on a synced hit — otherwise fall through
    // so the broad search can find a synced version the artist filter hid.
    if !artist.is_empty() {
        let narrow = lrclib_search(&client, title, Some(artist));
        if let Some(l) = lrclib_pick(&narrow, title, artist, dur_secs, true) {
            println!("✅ [lrclib] Synced match (narrow).");
            return Some(l);
        }
    }

    // Step 2: broad title-only search; accept synced or plain.
    let broad = lrclib_search(&client, title, None);
    match lrclib_pick(&broad, title, artist, dur_secs, false) {
        Some(l) if is_synced(&l) => {
            println!("✅ [lrclib] Synced match (broad).");
            Some(l)
        }
        Some(l) => {
            println!("ℹ️ [lrclib] Only plain (un-synced) lyrics.");
            Some(l)
        }
        None => {
            println!("❌ [lrclib] Not found.");
            None
        }
    }
}

/// Ranks search `results` for our track and returns the best lyrics: a synced
/// match if any (highest duration+artist score wins), otherwise — unless
/// `synced_only` — the best plain match. Rejects candidates whose title doesn't
/// match or whose (known) duration is far off, so we never grab a different song.
fn lrclib_pick(
    results: &[LrcResponse],
    title: &str,
    artist: &str,
    dur_secs: Option<u64>,
    synced_only: bool,
) -> Option<Vec<LyricLine>> {
    let mut best_synced: Option<(i32, &str)> = None;
    let mut best_plain: Option<(i32, &str)> = None;
    for r in results {
        // The title must actually match — the search is fuzzy.
        if !r.track_name.as_deref().is_some_and(|n| titles_match(n, title)) {
            continue;
        }
        // Duration is the strongest signal; a known-but-far result is a
        // different song and is rejected outright.
        let Some(mut score) = duration_score(dur_secs, r.duration) else {
            continue;
        };
        if artist_match(artist, r.artist_name.as_deref()) {
            score += 50;
        }
        if let Some(s) = r.synced_lyrics.as_deref().filter(|s| !s.trim().is_empty()) {
            if best_synced.map_or(true, |(b, _)| score > b) {
                best_synced = Some((score, s));
            }
        }
        if let Some(p) = r.plain_lyrics.as_deref().filter(|p| !p.trim().is_empty()) {
            if best_plain.map_or(true, |(b, _)| score > b) {
                best_plain = Some((score, p));
            }
        }
    }

    if let Some((_, s)) = best_synced {
        return parse_lrc_string(s);
    }
    if synced_only {
        return None;
    }
    best_plain.and_then(|(_, p)| parse_plain_lyrics(p))
}

/// Runs one lrclib `/api/search` query — by title, optionally narrowed by
/// `artist` — returning the raw results (an empty vec on any network/parse error
/// so callers can simply move on).
fn lrclib_search(client: &Client, title: &str, artist: Option<&str>) -> Vec<LrcResponse> {
    match send_with_retry(|| {
        let mut req = client
            .get("https://lrclib.net/api/search")
            .query(&[("track_name", title)]);
        if let Some(a) = artist.filter(|a| !a.is_empty()) {
            req = req.query(&[("artist_name", a)]);
        }
        req.send()
    }) {
        Ok(r) => r.json::<Vec<LrcResponse>>().unwrap_or_default(),
        Err(e) => {
            println!("❌ [lrclib] Search request failed (network/timeout): {}", e);
            Vec::new()
        }
    }
}

/// Scores a candidate by how well its duration matches ours. Returns `None`
/// (reject) when both durations are known but differ by more than 12s — that is
/// a different song. When either duration is unknown, returns a small neutral
/// score so the result stays eligible but ranks below a real duration match.
fn duration_score(ours: Option<u64>, theirs: Option<f64>) -> Option<i32> {
    match (ours, theirs) {
        (Some(o), Some(t)) => {
            let diff = (t - o as f64).abs();
            if diff > 12.0 {
                None
            } else {
                Some(100 - (diff * 8.0) as i32)
            }
        }
        _ => Some(10),
    }
}

/// Loose artist match: `true` if any comma/&/`feat`-separated part of our
/// artist string overlaps (either way) with the candidate's, after
/// normalization. Handles multi-artist tags like "UMBRELLA, VAMPXL".
fn artist_match(ours: &str, theirs: Option<&str>) -> bool {
    let Some(theirs) = theirs else { return false };
    let nt = normalize(theirs);
    if nt.is_empty() {
        return false;
    }
    let whole = normalize(ours);
    if !whole.is_empty() && (nt.contains(&whole) || whole.contains(&nt)) {
        return true;
    }
    ours.split([',', '&', '/']).any(|part| {
        let np = normalize(part);
        !np.is_empty() && (nt.contains(&np) || np.contains(&nt))
    })
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

    let client = http_client("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36")?;

    // 1. Search for the track to obtain its id (type=1 means "songs").
    let search: NeteaseSearch = match send_with_retry(|| {
        client
            .get("https://music.163.com/api/search/get")
            .header("Referer", "https://music.163.com")
            .query(&[("s", query.as_str()), ("type", "1"), ("limit", "10")])
            .send()
    }) {
        Ok(r) => match r.json() {
            Ok(j) => j,
            Err(e) => {
                println!("❌ [NetEase] Could not parse search response: {}", e);
                return None;
            }
        },
        Err(e) => {
            println!("❌ [NetEase] Search request failed (network/timeout): {}", e);
            return None;
        }
    };

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
    let lyric: NeteaseLyricResp = match send_with_retry(|| {
        client
            .get("https://music.163.com/api/song/lyric")
            .header("Referer", "https://music.163.com")
            .query(&[("id", id_str.as_str()), ("lv", "-1"), ("kv", "-1"), ("tv", "-1")])
            .send()
    }) {
        Ok(r) => match r.json() {
            Ok(j) => j,
            Err(e) => {
                println!("❌ [NetEase] Could not parse lyric response: {}", e);
                return None;
            }
        },
        Err(e) => {
            println!("❌ [NetEase] Lyric request failed (network/timeout): {}", e);
            return None;
        }
    };

    let lrc = lyric.lrc?.lyric?;
    if lrc.trim().is_empty() {
        println!("😔 [NetEase] Track has no lyrics.");
        return None;
    }

    // Prefer the timestamped ("karaoke") version; if NetEase only returned plain
    // text (no `[mm:ss]` tags), keep it as un-timed lyrics rather than nothing.
    if let Some(synced) = parse_lrc_string(&lrc) {
        println!("✅ [NetEase] Synced lyrics found!");
        return Some(synced);
    }
    println!("ℹ️ [NetEase] Only plain (un-synced) lyrics.");
    parse_plain_lyrics(&lrc)
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

/// Strips leading `[...]` tags from a line (LRC timestamps like `[00:12.34]` or
/// metadata like `[ar: ...]`), returning the remaining text.
fn strip_leading_tags(mut s: &str) -> &str {
    s = s.trim_start();
    while s.starts_with('[') {
        match s.find(']') {
            Some(i) => s = s[i + 1..].trim_start(),
            None => break,
        }
    }
    s
}

/// Parses plain, un-timed lyrics into [`LyricLine`]s that all sit at
/// `time_ms == 0` — the representation the UI treats as "no karaoke sync".
///
/// Any leading `[...]` tags are stripped (some plain responses still carry
/// stray timestamps or credits), and lines our font cannot render are dropped.
/// Returns `None` if nothing renderable remains.
pub fn parse_plain_lyrics(content: &str) -> Option<Vec<LyricLine>> {
    let mut lines = Vec::new();
    for raw in content.lines() {
        let text = strip_leading_tags(raw.trim()).trim();
        if contains_unrenderable(text) {
            continue;
        }
        lines.push(LyricLine {
            time_ms: 0,
            text: if text.is_empty() { " ".to_string() } else { text.to_string() },
        });
    }
    // Require at least one line with actual words.
    if lines.iter().any(|l| !l.text.trim().is_empty()) {
        Some(lines)
    } else {
        None
    }
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
