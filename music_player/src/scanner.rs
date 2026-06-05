use std::fs;
// use std::path::Path;
use reqwest::blocking::Client;
use serde::Deserialize;

#[derive(Clone)]
pub struct Playlist {
    pub name: String,
    pub songs: Vec<String>,
}

pub fn scan_music(root: &str) -> Vec<Playlist> {
    let mut playlists = vec![];
    
    println!("🔍 Сканування папки з музикою: {}", root);

    let mut root_songs = vec![];

    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();

            if path.is_dir() {
                let name = path.file_name().unwrap().to_string_lossy().to_string();
                let mut songs = vec![];

                if let Ok(files) = fs::read_dir(&path) {
                    for file in files.flatten() {
                        let file_path = file.path();
                        if file_path.extension().map_or(false, |ext| ext == "mp3") {
                            songs.push(file_path.to_string_lossy().to_string());
                        }
                    }
                }

                if !songs.is_empty() {
                    println!("📁 Знайдено плейліст '{}' (пісен: {})", name, songs.len());
                    playlists.push(Playlist { name, songs });
                }
            } else if path.is_file() && path.extension().map_or(false, |ext| ext == "mp3") {
                root_songs.push(path.to_string_lossy().to_string());
            }
        }
    } else {
        println!("❌ ПОМИЛКА: Не вдалося відкрити або знайти папку '{}'!", root);
        println!("Перевірте, чи вона лежить поруч із папкою 'music_player', а не всередині неї.");
    }

    if !root_songs.is_empty() {
        println!("🎵 Знайдено {} треків прямо в корені папки {}", root_songs.len(), root);
        playlists.push(Playlist {
            name: "Усі треки".to_string(),
            songs: root_songs,
        });
    }

    if playlists.is_empty() {
        println!("⚠️ УВАГА: Не знайдено жодного MP3-файлу у вказаному шляху.");
    }

    playlists
}

use id3::{Tag, TagLike};

// Структура для хранения одной строчки текста и её таймкода
#[derive(Debug, Clone)]
pub struct LyricLine {
    pub time_ms: u32,   // Время появления строчки в миллисекундах
    pub text: String,   // Сам текст
}

/// Пытается извлечь синхронизированный текст (SYLT) из MP3-файла.
pub fn get_synced_lyrics(file_path: &str) -> Option<Vec<LyricLine>> {
    println!("🎤 Пытаемся достать текст из файла: {}", file_path);
    
    let tag = match id3::Tag::read_from_path(file_path) {
        Ok(t) => {
            println!("✅ Базовые теги в файле есть. Ищем караоке-текст (SYLT)...");
            t
        },
        Err(e) => {
            println!("❌ Ошибка чтения тегов или тегов нет вообще: {}", e);
            return None;
        }
    };

    let mut found = false;
    for sync_lyric in tag.synchronised_lyrics() {
        found = true;
        let mut lines = Vec::new();
        for (time, text) in &sync_lyric.content {
            lines.push(LyricLine { time_ms: *time, text: text.to_string() });
        }
        if !lines.is_empty() {
            println!("🎉 УРА! Синхронизированный текст найден!");
            return Some(lines);
        }
    }

    if !found {
        println!("😔 В этом файле НЕТ синхронизированного текста (SYLT).");
    }
    None
}

// Структура для чтения ответа от базы данных
#[derive(Deserialize)]
struct LrcResponse {
    duration: Option<f64>,
    #[serde(rename = "syncedLyrics")]
    synced_lyrics: Option<String>,
}

/// Достаём название, артиста и альбом из ID3-тегов.
fn read_track_meta(file_path: &str) -> (String, String, String) {
    let mut title = String::new();
    let mut artist = String::new();
    let mut album = String::new();

    if let Ok(tag) = Tag::read_from_path(file_path) {
        title = tag.title().unwrap_or("").trim().to_string();
        artist = tag.artist().unwrap_or("").trim().to_string();
        album = tag.album().unwrap_or("").trim().to_string();
    }

    // Если названия нет — берём имя файла без расширения
    if title.is_empty() {
        title = std::path::Path::new(file_path)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
    }

    // Частый случай у скачанной музыки: в "названии" лежит "Артист - Трек",
    // а в поле "артист" — мусор. Разбор названия надёжнее.
    if let Some((a, t)) = title.split_once(" - ") {
        let (a, t) = (a.trim(), t.trim());
        if !a.is_empty() && !t.is_empty() {
            artist = a.to_string();
            title = t.to_string();
        }
    }

    (title, artist, album)
}

/// Ищет текст песни в интернете
/// Ищет синхронизированный текст в интернете по тегам и длительности трека.
/// Ищет синхронизированный текст в интернете, опрашивая источники по очереди.
pub fn fetch_lyrics_from_internet(
    file_path: &str,
    duration: Option<std::time::Duration>,
) -> Option<Vec<LyricLine>> {
    let (title, artist, album) = read_track_meta(file_path);
    if title.is_empty() {
        println!("❌ Не удалось определить название трека.");
        return None;
    }

    let dur_secs = duration.map(|d| d.as_secs());

    // Источники опрашиваются по порядку — берём первый, где нашёлся текст.
    // Чтобы добавить новый источник, допиши ещё одну строчку.
    if let Some(l) = lrclib_lyrics(&title, &artist, &album, dur_secs) {
        return Some(l);
    }
    if let Some(l) = netease_lyrics(&title, &artist, dur_secs) {
        return Some(l);
    }

    println!("❌ Ни в одном источнике подходящий текст не найден.");
    None
}

/// Источник №1: lrclib.net (точный, синхронизированный текст).
fn lrclib_lyrics(
    title: &str,
    artist: &str,
    album: &str,
    dur_secs: Option<u64>,
) -> Option<Vec<LyricLine>> {
    println!("🌐 [lrclib] Ищем: артист='{}', трек='{}'", artist, title);

    let client = Client::builder().user_agent("Elysium/1.0.2").build().ok()?;

    // --- Способ 1: точное совпадение через /api/get (учитывает длительность ±2с) ---
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
                                println!("✅ [lrclib] Точное совпадение (api/get).");
                                return parse_lrc_string(&s);
                            }
                        }
                    }
                }
            }
        }
    }

    // --- Способ 2: поиск по артисту+названию, выбираем по длительности ---
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
            .map_or(false, |s| !s.trim().is_empty());
        if !has_synced {
            continue;
        }
        match (dur_secs, r.duration) {
            (Some(ours), Some(theirs)) => {
                if (theirs - ours as f64).abs() <= 3.0 {
                    best = Some(r);
                    break;
                }
            }
            _ => {
                if best.is_none() {
                    best = Some(r);
                }
            }
        }
    }

    if let Some(r) = best {
        if let Some(s) = &r.synced_lyrics {
            println!("✅ [lrclib] Текст найден через поиск.");
            return parse_lrc_string(s);
        }
    }

    println!("❌ [lrclib] Не найдено.");
    None
}

// ---- Источник №2: NetEase Cloud Music ----
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
    duration: Option<u64>, // длительность в МИЛЛИсекундах
}
#[derive(Deserialize)]
struct NeteaseLyricResp {
    lrc: Option<NeteaseLrc>,
}
#[derive(Deserialize)]
struct NeteaseLrc {
    lyric: Option<String>,
}

/// Нормализуем строку для сравнения: нижний регистр, только буквы и цифры.
fn normalize(s: &str) -> String {
    s.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect()
}

/// Похожи ли названия (одно содержит другое после нормализации).
fn titles_match(a: &str, b: &str) -> bool {
    let na = normalize(a);
    let nb = normalize(b);
    !na.is_empty() && !nb.is_empty() && (na.contains(&nb) || nb.contains(&na))
}

/// Источник №2: NetEase Cloud Music (огромный каталог, каверы, неанглоязычное).
fn netease_lyrics(title: &str, artist: &str, dur_secs: Option<u64>) -> Option<Vec<LyricLine>> {
    let query = if artist.is_empty() {
        title.to_string()
    } else {
        format!("{} {}", artist, title)
    };
    println!("🌐 [NetEase] Ищем: {}", query);

    // NetEase капризен к заголовкам — притворяемся браузером + ставим Referer.
    let client = Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36")
        .build()
        .ok()?;

    // 1. Поиск трека → его id (type=1 — искать песни)
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
        println!("❌ [NetEase] Трек не найден.");
        return None;
    }

    // Выбираем песню по длительности (NetEase отдаёт мс → делим на 1000), иначе первую.
    // Берём песню, чьё НАЗВАНИЕ похоже на наше (отсекает случайные совпадения),
    // а при известной длительности — ещё и близкую по времени (±5 c).
    let song = songs.iter().find(|s| {
        let name_ok = s.name.as_deref().map_or(false, |n| titles_match(n, title));
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
            println!("❌ [NetEase] Похожий трек не найден (название/длительность не совпали).");
            return None;
        }
    };

    // 2. Тянем текст по id
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
        println!("😔 [NetEase] У трека нет синхротекста.");
        return None;
    }

    println!("✅ [NetEase] Текст найден!");
    parse_lrc_string(&lrc)
}

/// Превращает сырой текст с таймкодами в готовые строчки для плеера
pub fn parse_lrc_string(content: &str) -> Option<Vec<LyricLine>> {
    let mut lines = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            if let Some(close_idx) = line.find(']') {
                let time_str = &line[1..close_idx];
                let text = line[close_idx + 1..].trim().to_string();
                
                let parts: Vec<&str> = time_str.split(':').collect();
                if parts.len() == 2 {
                    let min: u32 = parts[0].parse().unwrap_or(0);
                    let sec_parts: Vec<&str> = parts[1].split('.').collect();
                    let sec: u32 = sec_parts[0].parse().unwrap_or(0);
                    
                    let ms: u32 = if sec_parts.len() > 1 {
                        let ms_str = sec_parts[1];
                        let ms_val: u32 = ms_str.parse().unwrap_or(0);
                        if ms_str.len() == 2 { ms_val * 10 } 
                        else if ms_str.len() == 1 { ms_val * 100 } 
                        else { ms_val }
                    } else { 0 };

                    let time_ms = (min * 60 * 1000) + (sec * 1000) + ms;
                    let display_text = if text.is_empty() { " ".to_string() } else { text };
                    lines.push(LyricLine { time_ms, text: display_text });
                }
            }
        }
    }
    if lines.is_empty() { None } else { Some(lines) }
}