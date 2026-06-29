//! YouTube Music backend for the "ЮБ" tab.
//!
//! This module talks to `yt-dlp` (and `ffmpeg`) as external command-line tools.
//! Going through `yt-dlp` is what makes the tab **ad-free**: we never load the
//! YouTube web player, we only pull the raw audio stream and play it locally.
//!
//! The two tools are **bootstrapped on demand**: the first time the user
//! searches or plays something, [`ensure_ytdlp`] / [`ensure_ffmpeg`] download a
//! standalone `yt-dlp.exe` (and a static `ffmpeg.exe`) into the app's data
//! folder. Nothing has to be pre-installed by the user.
//!
//! All functions here are blocking and meant to run on a background thread; the
//! UI polls their results over channels (see `app::youtube_page`). Progress is
//! reported by writing a short human-readable line into the shared `status`
//! string, which the page renders while work is in flight.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};

/// Shared, human-readable description of what the backend is doing right now
/// (e.g. "Загрузка ffmpeg…"). Empty means idle. The UI shows it verbatim.
pub type Status = Arc<Mutex<String>>;

/// One search result: enough to show a row and start playback.
#[derive(Clone)]
pub struct YtTrack {
    /// YouTube video id, used to build the watch URL.
    pub id: String,
    pub title: String,
    pub artist: String,
    /// Length in seconds, when yt-dlp reports it.
    pub duration: Option<u32>,
}

/// Direct download URL for the standalone Windows yt-dlp build.
const YTDLP_URL: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe";
/// Static FFmpeg build (BtbN). We only extract `ffmpeg.exe` from the archive.
const FFMPEG_ZIP_URL: &str =
    "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip";

/// Folder where the downloaded tools live (`%APPDATA%/Elysium/tools`).
fn tools_dir() -> PathBuf {
    let base = dirs::data_dir().unwrap_or_else(std::env::temp_dir);
    base.join("Elysium").join("tools")
}

fn set_status(status: &Status, text: &str) {
    if let Ok(mut s) = status.lock() {
        *s = text.to_string();
    }
}

/// Builds a [`Command`] that does not flash a console window on Windows.
fn quiet_command(program: &Path) -> Command {
    let cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW — keep the GUI app silent (no console pop-up).
        let mut cmd = cmd;
        cmd.creation_flags(0x0800_0000);
        return cmd;
    }
    #[allow(unreachable_code)]
    cmd
}

/// Downloads `url` fully into memory. Used for both tools (a few tens of MB).
fn download(url: &str) -> Result<Vec<u8>, String> {
    let resp = ureq::get(url)
        .call()
        .map_err(|e| format!("сеть: {e}"))?;
    let mut buf = Vec::new();
    resp.into_reader()
        .read_to_end(&mut buf)
        .map_err(|e| format!("чтение: {e}"))?;
    Ok(buf)
}

/// Ensures `yt-dlp.exe` exists, downloading it on first use. Returns its path.
pub fn ensure_ytdlp(status: &Status) -> Result<PathBuf, String> {
    let path = tools_dir().join("yt-dlp.exe");
    if path.exists() {
        return Ok(path);
    }
    set_status(status, "Загрузка yt-dlp…");
    std::fs::create_dir_all(tools_dir()).map_err(|e| e.to_string())?;
    let bytes = download(YTDLP_URL)?;
    std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Resolved ffmpeg location, cached after the first lookup so we don't probe
/// `PATH` (a process spawn) on every single play.
static FFMPEG_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Resolves an `ffmpeg` to use for streaming: an existing one on `PATH` if
/// present, otherwise a copy downloaded into the app's tools folder on first
/// use. Returns the program to invoke (`"ffmpeg"` or an absolute path). The
/// result is cached for the rest of the session.
pub fn ensure_ffmpeg(status: &Status) -> Result<PathBuf, String> {
    if let Some(p) = FFMPEG_PATH.get() {
        return Ok(p.clone());
    }
    let resolved = ensure_ffmpeg_uncached(status)?;
    let _ = FFMPEG_PATH.set(resolved.clone());
    Ok(resolved)
}

/// The actual ffmpeg lookup/download, run at most once via [`ensure_ffmpeg`].
fn ensure_ffmpeg_uncached(status: &Status) -> Result<PathBuf, String> {
    // 1. Already installed system-wide?
    let on_path = quiet_command(Path::new("ffmpeg"))
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if on_path {
        return Ok(PathBuf::from("ffmpeg"));
    }

    // 2. Already downloaded by us?
    let path = tools_dir().join("ffmpeg.exe");
    if path.exists() {
        return Ok(path);
    }

    // 3. Fetch a static build and extract just ffmpeg.exe.
    set_status(status, "Загрузка ffmpeg (один раз)…");
    std::fs::create_dir_all(tools_dir()).map_err(|e| e.to_string())?;
    let bytes = download(FFMPEG_ZIP_URL)?;

    set_status(status, "Распаковка ffmpeg…");
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name().replace('\\', "/");
        if name.ends_with("/bin/ffmpeg.exe") || name.ends_with("ffmpeg.exe") {
            let mut out = std::fs::File::create(&path).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
            return Ok(path);
        }
    }
    Err("ffmpeg.exe не найден в архиве".to_string())
}

/// Searches for `query` and returns matching tracks.
///
/// Primary path is the YouTube **Music** InnerTube API with the "Songs" filter,
/// so results are real songs (artist + duration) rather than arbitrary videos —
/// and it needs no `yt-dlp`. If that fails (network/format change) it falls back
/// to a plain `yt-dlp` YouTube search so the tab still works.
pub fn search(query: &str, limit: usize, status: &Status) -> Result<Vec<YtTrack>, String> {
    set_status(status, "Поиск…");
    let result = match search_music(query) {
        Ok(tracks) if !tracks.is_empty() => Ok(tracks),
        _ => search_ytdlp(query, limit, status),
    };
    set_status(status, "");
    result
}

/// YouTube Music search via the InnerTube `search` endpoint, restricted to the
/// "Songs" results with a fixed filter param (the same one `ytmusicapi` uses).
///
/// One response is a single shelf of ~20 songs, so we follow a few continuation
/// tokens to gather a fuller page of results.
fn search_music(query: &str) -> Result<Vec<YtTrack>, String> {
    // Public WEB_REMIX (YouTube Music) API key + "Songs only" filter param.
    const KEY: &str = "AIzaSyC9XL3ZjWddXya6X74dJoCTL-WEYFDNX30";
    const SONGS_FILTER: &str = "EgWKAQIIAWoMEAMQBBAJEAoQBRAV";
    const MAX_RESULTS: usize = 60;
    const MAX_PAGES: usize = 3;

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
    let context = serde_json::json!({ "client": {
        "clientName": "WEB_REMIX",
        "clientVersion": "1.20240101.01.00",
        "hl": "en",
        "gl": "US",
    }});

    // First page: the actual query + songs filter.
    let first_body = serde_json::json!({
        "context": context, "query": query, "params": SONGS_FILTER,
    });
    let mut json = innertube_search(&client, KEY, &first_body, None)?;

    let mut tracks = Vec::new();
    let mut seen = std::collections::HashSet::new();
    collect_tracks(&json, &mut tracks, &mut seen);

    // Following pages: a continuation token, body carries only the context.
    let mut token = extract_continuation(&json);
    let mut pages = 0;
    while let Some(t) = token {
        if pages >= MAX_PAGES || tracks.len() >= MAX_RESULTS {
            break;
        }
        pages += 1;
        let cont_body = serde_json::json!({ "context": context });
        match innertube_search(&client, KEY, &cont_body, Some(&t)) {
            Ok(j) => {
                collect_tracks(&j, &mut tracks, &mut seen);
                token = extract_continuation(&j);
                json = j;
            }
            Err(_) => break,
        }
    }
    let _ = json;

    if tracks.is_empty() {
        return Err("нет результатов".to_string());
    }
    Ok(tracks)
}

/// Posts one InnerTube `search` request. With `token` set it fetches the next
/// continuation page instead of a fresh query.
fn innertube_search(
    client: &reqwest::blocking::Client,
    key: &str,
    body: &serde_json::Value,
    token: Option<&str>,
) -> Result<serde_json::Value, String> {
    let url = format!("https://music.youtube.com/youtubei/v1/search?key={key}&prettyPrint=false");
    let mut req = client
        .post(&url)
        .header("Origin", "https://music.youtube.com")
        .header("User-Agent", "Mozilla/5.0")
        .json(body);
    if let Some(t) = token {
        req = req.query(&[("ctoken", t), ("continuation", t), ("type", "next")]);
    }
    req.send()
        .map_err(|e| format!("сеть: {e}"))?
        .json()
        .map_err(|e| format!("разбор: {e}"))
}

/// Appends every parsed song row found in `json` to `tracks`, skipping ids
/// already present (continuations occasionally repeat).
fn collect_tracks(
    json: &serde_json::Value,
    tracks: &mut Vec<YtTrack>,
    seen: &mut std::collections::HashSet<String>,
) {
    let mut items = Vec::new();
    collect_key(json, "musicResponsiveListItemRenderer", &mut items);
    for it in items {
        if let Some(track) = parse_music_item(it) {
            if seen.insert(track.id.clone()) {
                tracks.push(track);
            }
        }
    }
}

/// Finds the next-page continuation token in a search response, if any.
fn extract_continuation(json: &serde_json::Value) -> Option<String> {
    let mut conts = Vec::new();
    collect_key(json, "nextContinuationData", &mut conts);
    conts
        .iter()
        .find_map(|c| c.get("continuation").and_then(|v| v.as_str()).map(str::to_string))
}

/// Recursively collects every value stored under `key` anywhere in `v`.
fn collect_key<'a>(v: &'a serde_json::Value, key: &str, out: &mut Vec<&'a serde_json::Value>) {
    match v {
        serde_json::Value::Object(map) => {
            for (k, val) in map {
                if k == key {
                    out.push(val);
                }
                collect_key(val, key, out);
            }
        }
        serde_json::Value::Array(arr) => arr.iter().for_each(|val| collect_key(val, key, out)),
        _ => {}
    }
}

/// Extracts a [`YtTrack`] from one `musicResponsiveListItemRenderer`.
fn parse_music_item(it: &serde_json::Value) -> Option<YtTrack> {
    // Runs of flex column `col` (column 0 = title, column 1 = "artist • album • duration").
    let flex_runs = |col: usize| -> Option<&Vec<serde_json::Value>> {
        it.get("flexColumns")?
            .as_array()?
            .get(col)?
            .get("musicResponsiveListItemFlexColumnRenderer")?
            .get("text")?
            .get("runs")?
            .as_array()
    };

    let title = flex_runs(0)?.first()?.get("text")?.as_str()?.to_string();

    let subtitle = flex_runs(1);
    let artist = subtitle
        .and_then(|r| r.first())
        .and_then(|r| r.get("text"))
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();
    // Duration is the last run of the subtitle, formatted "M:SS" / "H:MM:SS".
    let duration = subtitle
        .and_then(|runs| runs.iter().rev().find_map(|r| r.get("text").and_then(|t| t.as_str()).and_then(parse_clock)));

    // Prefer the explicit playlist item id; fall back to any watch endpoint.
    let id = it
        .get("playlistItemData")
        .and_then(|p| p.get("videoId"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .or_else(|| {
            let mut eps = Vec::new();
            collect_key(it, "watchEndpoint", &mut eps);
            eps.iter()
                .find_map(|e| e.get("videoId").and_then(|v| v.as_str()))
                .map(str::to_string)
        })?;

    Some(YtTrack { id, title, artist, duration })
}

/// Parses a `"M:SS"` / `"H:MM:SS"` clock string into seconds.
fn parse_clock(s: &str) -> Option<u32> {
    let parts: Vec<&str> = s.trim().split(':').collect();
    if parts.len() < 2 || parts.len() > 3 {
        return None;
    }
    let mut secs = 0u32;
    for p in parts {
        secs = secs * 60 + p.trim().parse::<u32>().ok()?;
    }
    Some(secs)
}

/// Fallback search via `yt-dlp "ytsearchN:<query>"` (plain YouTube, may include
/// non-music videos). Used only when the YouTube Music API is unavailable.
fn search_ytdlp(query: &str, limit: usize, status: &Status) -> Result<Vec<YtTrack>, String> {
    let ytdlp = ensure_ytdlp(status)?;

    let output = quiet_command(&ytdlp)
        .arg(format!("ytsearch{limit}:{query}"))
        .args([
            "--flat-playlist",
            "--dump-single-json",
            "--no-warnings",
            "--ignore-errors",
        ])
        .output()
        .map_err(|e| format!("запуск yt-dlp: {e}"))?;

    if !output.stdout.iter().any(|b| *b == b'{') {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!("yt-dlp не вернул результатов: {}", err.trim()));
    }

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).map_err(|e| format!("разбор JSON: {e}"))?;

    let entries = json
        .get("entries")
        .and_then(|e| e.as_array())
        .ok_or_else(|| "пустой ответ".to_string())?;

    let tracks = entries
        .iter()
        .filter_map(|e| {
            let id = e.get("id")?.as_str()?.to_string();
            let title = e
                .get("title")
                .and_then(|t| t.as_str())
                .unwrap_or("Без названия")
                .to_string();
            let artist = e
                .get("uploader")
                .or_else(|| e.get("channel"))
                .and_then(|c| c.as_str())
                .unwrap_or("")
                .to_string();
            let duration = e
                .get("duration")
                .and_then(|d| d.as_f64())
                .map(|d| d as u32);
            Some(YtTrack {
                id,
                title,
                artist,
                duration,
            })
        })
        .collect();

    Ok(tracks)
}

/// Resolves the **direct audio stream URL** for video `id` without downloading
/// anything. This is what makes "play instantly" possible: ffmpeg then streams
/// straight from this URL instead of waiting for a full download + conversion.
///
/// Speed matters here (it runs on the click→sound path), so we first try the
/// `android_vr` player client, which skips the slow web-player JS handling and
/// returns an ffmpeg-friendly URL roughly a third faster than the default. If
/// that client has no audio for a given video we retry with the default client.
pub fn resolve_audio_url(id: &str, status: &Status) -> Result<String, String> {
    let ytdlp = ensure_ytdlp(status)?;
    set_status(status, "Подготовка потока…");

    let url = format!("https://www.youtube.com/watch?v={id}");
    // First the fast client, then the default as a reliability fallback.
    for client in ["android_vr", ""] {
        let mut cmd = quiet_command(&ytdlp);
        cmd.args(["-f", "bestaudio", "-g", "--no-playlist", "--no-warnings"]);
        if !client.is_empty() {
            cmd.args(["--extractor-args", &format!("youtube:player_client={client}")]);
        }
        cmd.arg(&url);

        if let Ok(output) = cmd.output() {
            // `-g` prints one direct URL per line; take the first http one.
            if let Some(u) = String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(str::trim)
                .find(|l| l.starts_with("http"))
            {
                set_status(status, "");
                return Ok(u.to_string());
            }
        }
    }

    set_status(status, "");
    Err("не удалось получить поток".to_string())
}
