//! Playback logic: starting tracks, advancing the queue and hotkey actions.
//!
//! The queue model is the important bit. [`App::play_track`] is the single
//! entry point that loads a file and (re)starts lyric fetching. Track stepping
//! ([`App::play_next_track`] / [`App::play_previous_track`]) walks
//! [`App::playback_queue`] — a snapshot taken when playback started — so
//! navigating the UI mid-song never changes what plays next.

use super::{App, LIKED_PAGE_IDX, LIKED_PLAYLIST_NAME};
use crate::scanner::{self, Playlist};
use crate::shortcuts::Shortcut;
use std::time::Duration;

/// Reads a track's total length from the file (used off the UI thread).
///
/// MP3: `mp3-duration` scans frames, which stays accurate even for VBR files
/// that lack a Xing/Info header. Every other format: read the duration from its
/// headers via `lofty`. Returns `None` if the file can't be read.
pub(crate) fn track_duration(path: &str) -> Option<Duration> {
    let is_mp3 = std::path::Path::new(path)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("mp3"));
    if is_mp3 {
        mp3_duration::from_path(path).ok()
    } else {
        crate::meta::read_raw_meta(path).duration
    }
}

impl App {
    /// Plays the next track in the snapshot queue.
    ///
    /// If the current track is the last (or not found in the queue at all),
    /// playback stops. An empty queue also stops playback.
    pub(super) fn play_next_track(&mut self) {
        if self.playback_queue.is_empty() {
            self.is_playing = false;
            return;
        }

        if let Some(current_idx) = self.playback_queue.iter().position(|s| s == &self.current_song) {
            let next_idx = current_idx + 1;
            if next_idx < self.playback_queue.len() {
                self.play_track(&self.playback_queue[next_idx].clone());
            } else {
                // Reached the end of the queue.
                self.is_playing = false;
                self.elapsed_duration = Duration::ZERO;
            }
        } else {
            // Current track is not in the queue: start from the top.
            self.play_track(&self.playback_queue[0].clone());
        }
    }

    /// Plays the previous track, or restarts the current one if already first.
    pub(super) fn play_previous_track(&mut self) {
        if self.playback_queue.is_empty() {
            return;
        }

        if let Some(current_idx) = self.playback_queue.iter().position(|s| s == &self.current_song) {
            if current_idx > 0 {
                self.play_track(&self.playback_queue[current_idx - 1].clone());
            } else {
                // Already first: seek back to the start instead of going further.
                self.player.seek(&self.current_song, Duration::ZERO);
                self.elapsed_duration = Duration::ZERO;
            }
        }
    }

    /// Executes a hotkey action against the current playback state.
    pub(super) fn do_shortcut(&mut self, action: Shortcut) {
        match action {
            Shortcut::PlayPause => {
                if self.current_song.is_empty() {
                    return;
                }
                if self.is_playing {
                    self.player.pause();
                    self.is_playing = false;
                } else {
                    self.player.resume();
                    self.is_playing = true;
                }
            }
            Shortcut::Next => self.play_next_track(),
            Shortcut::Prev => self.play_previous_track(),
            Shortcut::VolumeUp => {
                self.volume = (self.volume + 0.05).min(1.0);
                self.player.set_volume(self.volume);
            }
            Shortcut::VolumeDown => {
                self.volume = (self.volume - 0.05).max(0.0);
                self.player.set_volume(self.volume);
            }
            Shortcut::ToggleLike => {
                if !self.current_song.is_empty() {
                    let song = self.current_song.clone();
                    self.toggle_like(&song);
                }
            }
        }
    }

    /// Loads `path` into the player and starts it, then arranges lyrics.
    ///
    /// Lyrics are taken from the in-memory cache when available; otherwise a
    /// background thread fetches them (embedded SYLT first, then online) and
    /// sends the result back via [`App::lyrics_receiver`]. Playback timers are
    /// reset so progress and lyric highlighting start from zero.
    pub(super) fn play_track(&mut self, path: &str) {
        self.current_song = path.to_string();
        self.total_duration = self.player.play(path);

        // Pre-render the whole-track waveform for the Lab window off-thread
        // (it decodes the entire file). Clear the old one first so the Lab
        // never shows the previous track's shape, and tag the result with the
        // path so a slow decode that finishes late is discarded.
        if let Ok(mut wf) = self.waveform.lock() {
            *wf = None;
        }
        {
            let waveform = self.waveform.clone();
            let path_for_wave = path.to_string();
            std::thread::spawn(move || {
                if let Some(data) = crate::lab::dsp::compute_waveform(&path_for_wave, 1600) {
                    if let Ok(mut slot) = waveform.lock() {
                        // Only store if it is still the track being requested.
                        if data.path == path_for_wave {
                            *slot = Some(data);
                        }
                    }
                }
            });
        }

        // If the decoder couldn't report a duration (the common case for MP3),
        // read it from the file's headers/tags off the UI thread. Doing this on
        // the UI thread would freeze it on every track change. The result is
        // picked up in `App::update`. Replacing the receiver here also discards
        // any in-flight result from a previous track. `lofty` reports duration
        // for every supported format, not just MP3.
        self.duration_receiver = None;
        if self.total_duration.is_none() {
            let (dtx, drx) = std::sync::mpsc::channel();
            self.duration_receiver = Some(drx);
            let path_for_duration = path.to_string();
            std::thread::spawn(move || {
                let _ = dtx.send(track_duration(&path_for_duration));
            });
        }

        // Check the cache first (before spawning any thread).
        if let Some(cached) = self.lyrics_cache.get(path) {
            self.current_lyrics = Some(cached.clone());
            self.lyrics_receiver = None;
        } else {
            // Not cached: clear old lyrics and fetch on a background thread.
            self.current_lyrics = None;
            let (tx, rx) = std::sync::mpsc::channel();
            self.lyrics_receiver = Some(rx);

            let path_for_thread = path.to_string();
            let duration_for_thread = self.total_duration; // Option<Duration> is Copy

            std::thread::spawn(move || {
                // Prefer lyrics embedded directly in the MP3.
                if let Some(lyrics) = scanner::get_synced_lyrics(&path_for_thread) {
                    let _ = tx.send(Some(lyrics));
                    return;
                }
                // Otherwise search online by tags + duration. Duration greatly
                // sharpens matching (it lets us safely fall back to a title-only
                // search for files with messy artist tags), but rodio reports
                // `None` for most MP3s — so read it from the file here when the
                // decoder couldn't. Pass the FULL path (needed to read tags).
                let duration =
                    duration_for_thread.or_else(|| track_duration(&path_for_thread));
                let internet_lyrics =
                    scanner::fetch_lyrics_from_internet(&path_for_thread, duration);
                let _ = tx.send(internet_lyrics);
            });
        }

        self.current_playback_time_ms = 0;
        self.song_start_time = Some(std::time::Instant::now());
        self.elapsed_duration = Duration::ZERO;
        self.is_playing = true;
    }

    /// Builds the playback queue for the currently selected page.
    ///
    /// * Liked page ([`LIKED_PAGE_IDX`]): the Liked music playlist's tracks.
    /// * A specific playlist: just that playlist's tracks.
    /// * Home (`None`): every playlist concatenated.
    ///
    /// Uses `get` rather than indexing so a stale index can never panic.
    pub(super) fn get_current_queue(&self) -> Vec<String> {
        if self.selected_playlist_idx == Some(LIKED_PAGE_IDX) {
            return self
                .playlists
                .iter()
                .find(|p| p.name == LIKED_PLAYLIST_NAME)
                .map(|p| p.songs.clone())
                .unwrap_or_default();
        }

        let mut queue = Vec::new();
        let filtered_playlists: Vec<&Playlist> = match self.selected_playlist_idx {
            Some(idx) => self.playlists.get(idx).into_iter().collect(),
            None => self.playlists.iter().collect(),
        };
        for playlist in filtered_playlists {
            for song in &playlist.songs {
                queue.push(song.clone());
            }
        }
        queue
    }
}
