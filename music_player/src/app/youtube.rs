//! "ЮБ" tab logic: kicking off background searches/stream-resolution and
//! consuming their results. The drawing lives in [`super::ui::youtube_page`].
//!
//! Playback here **streams** straight from YouTube (no full download): a
//! background thread resolves the direct audio URL with `yt-dlp`, then the
//! player streams and decodes it on the fly. The UI thread only ever polls
//! (never blocks), so the player stays responsive.

use super::App;
use crate::meta::TrackMeta;
use crate::ytdl::YtTrack;
use eframe::egui;
use std::io::Read;
use std::sync::mpsc::channel;
use std::thread;
use std::time::{Duration, Instant};

impl App {
    /// Drains the YouTube search/stream channels. Called once per frame.
    pub(in crate::app) fn poll_youtube(&mut self, ctx: &egui::Context) {
        // A finished search → replace the results (or surface an error).
        if let Some(rx) = &self.yt_search_rx {
            if let Ok(result) = rx.try_recv() {
                self.yt_searching = false;
                self.yt_search_rx = None;
                match result {
                    Ok(tracks) => {
                        self.yt_results = tracks;
                        self.yt_error = None;
                        self.yt_searched = true;
                        // Warm the URL cache for the first results so the most
                        // likely clicks play almost instantly.
                        self.prefetch_yt_urls();
                        // Load cover art for the result cards in the background.
                        self.load_yt_thumbnails(ctx);
                    }
                    Err(e) => self.yt_error = Some(e),
                }
            }
        }

        // If the current track is a stream whose cover only just finished
        // loading, attach it so the bottom bar / Now Playing show the image.
        if self.current_song.starts_with("yt:") {
            let missing = self
                .track_meta
                .get(&self.current_song)
                .map(|m| m.cover.is_none())
                .unwrap_or(false);
            if missing {
                let id = self.current_song.trim_start_matches("yt:").to_string();
                let thumb = self.yt_thumbs.lock().ok().and_then(|t| t.get(&id).cloned());
                if let Some((tex, bg)) = thumb {
                    if let Some(m) = self.track_meta.get_mut(&self.current_song) {
                        m.cover = Some(tex);
                        m.bg = Some(bg);
                    }
                }
            }
        }

        // A resolved stream → start playing it immediately.
        if let Some(rx) = &self.yt_play_rx {
            if let Ok(result) = rx.try_recv() {
                self.yt_play_rx = None;
                let track = self.yt_loading_track.take();
                match (result, track) {
                    (Ok((url, ffmpeg)), Some(track)) => self.play_yt_stream(&url, &ffmpeg, track),
                    (Err(e), _) => self.yt_error = Some(e),
                    _ => {}
                }
            }
        }
    }

    /// Starts a background YouTube search for the current query box contents.
    pub(in crate::app) fn start_yt_search(&mut self, ctx: &egui::Context) {
        let query = self.yt_query.trim().to_string();
        if query.is_empty() || self.yt_searching {
            return;
        }
        self.yt_searching = true;
        self.yt_error = None;

        let (tx, rx) = channel();
        self.yt_search_rx = Some(rx);
        let status = self.yt_status.clone();
        let ctx2 = ctx.clone();
        thread::spawn(move || {
            let result = crate::ytdl::search(&query, 25, &status);
            if let Ok(mut s) = status.lock() {
                s.clear();
            }
            let _ = tx.send(result);
            ctx2.request_repaint();
        });
    }

    /// Resolves `track`'s direct stream URL in the background and plays it when
    /// ready. Ignored if another track is already being resolved.
    pub(in crate::app) fn start_yt_play(&mut self, ctx: &egui::Context, track: YtTrack) {
        if self.yt_loading_track.is_some() {
            return;
        }
        let id = track.id.clone();
        self.yt_loading_track = Some(track);
        self.yt_error = None;

        // Reuse a prefetched URL if we already have one for this track.
        let cached = self
            .yt_url_cache
            .lock()
            .ok()
            .and_then(|c| c.get(&id).cloned());

        let (tx, rx) = channel();
        self.yt_play_rx = Some(rx);
        let status = self.yt_status.clone();
        let cache = self.yt_url_cache.clone();
        let ctx2 = ctx.clone();
        thread::spawn(move || {
            let result = (|| -> Result<(String, std::path::PathBuf), String> {
                let url = match cached {
                    Some(u) => u,
                    None => {
                        let u = crate::ytdl::resolve_audio_url(&id, &status)?;
                        if let Ok(mut c) = cache.lock() {
                            c.insert(id.clone(), u.clone());
                        }
                        u
                    }
                };
                let ffmpeg = crate::ytdl::ensure_ffmpeg(&status)?;
                Ok((url, ffmpeg))
            })();
            if let Ok(mut s) = status.lock() {
                s.clear();
            }
            let _ = tx.send(result);
            ctx2.request_repaint();
        });
    }

    /// Resolves the direct URLs of the first few search results in the
    /// background and stores them in the cache, so a click on a top result does
    /// not have to wait for `yt-dlp`. Runs silently (does not touch the status
    /// line) and skips ids already cached or in flight.
    fn prefetch_yt_urls(&self) {
        for track in self.yt_results.iter().take(4) {
            let id = track.id.clone();
            if self
                .yt_url_cache
                .lock()
                .map(|c| c.contains_key(&id))
                .unwrap_or(true)
            {
                continue;
            }
            let cache = self.yt_url_cache.clone();
            thread::spawn(move || {
                // A throwaway status sink keeps prefetch from overwriting the
                // visible "Поиск…"/"Загрузка…" line.
                let quiet = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
                if let Ok(url) = crate::ytdl::resolve_audio_url(&id, &quiet) {
                    if let Ok(mut c) = cache.lock() {
                        c.insert(id, url);
                    }
                }
            });
        }
    }

    /// Fetches the cover art for the current results in the background and
    /// uploads each as a texture, so the result cards fill in with images.
    ///
    /// Uses YouTube's predictable per-video thumbnail URL (no extra metadata
    /// call). Textures stream into `yt_thumbs` and the UI repaints as they land.
    fn load_yt_thumbnails(&self, ctx: &egui::Context) {
        let ids: Vec<String> = self.yt_results.iter().map(|t| t.id.clone()).collect();
        let thumbs = self.yt_thumbs.clone();
        let ctx = ctx.clone();
        thread::spawn(move || {
            for id in ids {
                if thumbs.lock().map(|t| t.contains_key(&id)).unwrap_or(true) {
                    continue; // already loaded
                }
                let url = format!("https://i.ytimg.com/vi/{id}/mqdefault.jpg");
                let Ok(resp) = ureq::get(&url).call() else { continue };
                let mut bytes = Vec::new();
                if resp.into_reader().read_to_end(&mut bytes).is_err() {
                    continue;
                }
                let Ok(img) = image::load_from_memory(&bytes) else { continue };
                let mut rgba = img.to_rgba8();
                // Center-crop the 16:9 thumbnail to a square so it reads like
                // album art everywhere it is drawn (cards + player).
                let (w, h) = rgba.dimensions();
                let side = w.min(h);
                let rgba = image::imageops::crop(&mut rgba, (w - side) / 2, (h - side) / 2, side, side)
                    .to_image();
                // Dark average color for the Now Playing background.
                let bg = crate::meta::average_bg_color(&rgba);
                let color = egui::ColorImage::from_rgba_unmultiplied(
                    [side as usize, side as usize],
                    rgba.as_raw(),
                );
                let tex =
                    ctx.load_texture(format!("ytthumb:{id}"), color, egui::TextureOptions::LINEAR);
                if let Ok(mut t) = thumbs.lock() {
                    t.insert(id, (tex, bg));
                }
                ctx.request_repaint();
            }
        });
    }

    /// Seeks the currently playing **stream** to `position` by restarting
    /// ffmpeg from that offset (a remote stream can't be seeked like a file).
    /// Returns `false` if the current track isn't a seekable stream.
    pub(in crate::app) fn seek_yt_stream(&mut self, position: Duration) -> bool {
        if !self.current_song.starts_with("yt:") {
            return false;
        }
        let id = self.current_song.trim_start_matches("yt:").to_string();
        let Some(url) = self.yt_url_cache.lock().ok().and_then(|c| c.get(&id).cloned()) else {
            return false;
        };
        let quiet = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let Ok(ffmpeg) = crate::ytdl::ensure_ffmpeg(&quiet) else {
            return false;
        };
        self.player.play_stream(&ffmpeg, &url, position, self.total_duration);
        self.elapsed_duration = position;
        self.is_playing = true;
        true
    }

    /// Begins streaming `url` and wires up the now-playing state from `track`.
    ///
    /// Parallels [`App::play_track`] but for a remote stream: there is no local
    /// file, so the title/artist come from the search result and the cover and
    /// synced lyrics are simply absent.
    fn play_yt_stream(&mut self, url: &str, ffmpeg: &std::path::Path, track: YtTrack) {
        // A synthetic key identifies this stream in `track_meta`/`current_song`.
        let key = format!("yt:{}", track.id);
        // Reuse the already-loaded result thumbnail as the cover + background.
        let thumb = self.yt_thumbs.lock().ok().and_then(|t| t.get(&track.id).cloned());
        self.track_meta.insert(
            key.clone(),
            TrackMeta {
                title: track.title.clone(),
                artist: (!track.artist.is_empty()).then(|| track.artist.clone()),
                cover: thumb.as_ref().map(|(tex, _)| tex.clone()),
                bg: thumb.as_ref().map(|(_, bg)| *bg),
            },
        );

        self.current_song = key.clone();
        let hint = track.duration.map(|d| Duration::from_secs(d as u64));
        self.total_duration = self.player.play_stream(ffmpeg, url, Duration::ZERO, hint);
        self.duration_receiver = None;

        // Streams have no local waveform to render.
        if let Ok(mut wf) = self.waveform.lock() {
            *wf = None;
        }

        // Fetch lyrics online from the title/artist (no local tags to read).
        // YouTube's auto-music channels append " - Topic" to the artist; strip it
        // so the lyrics providers match.
        self.current_lyrics = None;
        let (tx, rx) = channel();
        self.lyrics_receiver = Some(rx);
        let title = track.title.clone();
        let artist = track
            .artist
            .trim_end_matches(" - Topic")
            .trim()
            .to_string();
        let dur = self.total_duration;
        thread::spawn(move || {
            let _ = tx.send(crate::scanner::fetch_lyrics_by_tags(&title, &artist, "", dur));
        });

        self.current_playback_time_ms = 0;
        self.song_start_time = Some(Instant::now());
        self.elapsed_duration = Duration::ZERO;
        self.is_playing = true;
        // A streamed track is a queue of one: next/prev simply stop.
        self.playback_queue = vec![key];
    }
}
