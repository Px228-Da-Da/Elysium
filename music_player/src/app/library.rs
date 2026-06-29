//! Library mutations: likes, playlist membership, deletion and import.
//!
//! Every method here keeps the in-memory [`App::playlists`] and the on-disk
//! config in sync. Likes and ordinary playlists are persisted separately
//! ([`App::save_liked`] vs [`App::save_playlists`]) because they have different
//! lifecycles, but both ultimately write to the same `config.json`.

use super::{App, LIKED_PAGE_IDX, LIKED_PLAYLIST_NAME};
use crate::audio::{collect_audio_files, is_audio_file};
use crate::config::*;
use crate::lang::Lang;
use crate::meta::{nfc, read_track_meta, LoaderMsg};
use crate::scanner::Playlist;
use eframe::egui;
use std::collections::HashSet;

impl App {
    /// Returns `true` if `song_path` is in the Liked music playlist.
    pub(super) fn is_liked(&self, song_path: &str) -> bool {
        self.playlists
            .iter()
            .find(|p| p.name == LIKED_PLAYLIST_NAME)
            .map(|p| p.songs.contains(&song_path.to_string()))
            .unwrap_or(false)
    }

    /// Toggles the like state of `song_path`, creating the Liked music playlist
    /// on first use and persisting the change immediately.
    pub(super) fn toggle_like(&mut self, song_path: &str) {
        if let Some(playlist) = self.playlists.iter_mut().find(|p| p.name == LIKED_PLAYLIST_NAME) {
            if let Some(pos) = playlist.songs.iter().position(|s| s == song_path) {
                playlist.songs.remove(pos); // already liked → unlike
            } else {
                playlist.songs.insert(0, song_path.to_string()); // newest like first
            }
        } else {
            // First like ever: create the playlist at the front of the list.
            self.playlists.insert(
                0,
                Playlist {
                    name: LIKED_PLAYLIST_NAME.to_string(),
                    songs: vec![song_path.to_string()],
                },
            );
            // Inserting at the front shifted every other playlist's index by +1.
            // If a real playlist page is open, bump its index so the page the
            // user is looking at does not silently change underneath them.
            if let Some(idx) = self.selected_playlist_idx {
                if idx != LIKED_PAGE_IDX {
                    self.selected_playlist_idx = Some(idx + 1);
                }
            }
        }

        // Persist immediately so likes survive a restart.
        self.save_liked();
    }

    /// Persists the Liked music playlist's tracks to the config.
    pub(super) fn save_liked(&self) {
        let songs: Vec<String> = self
            .playlists
            .iter()
            .find(|p| p.name == LIKED_PLAYLIST_NAME)
            .map(|p| p.songs.clone())
            .unwrap_or_default();
        let mut cfg = load_config();
        cfg.liked = songs;
        save_config(&cfg);
    }

    /// Persists all ordinary playlists (everything except Liked music).
    pub(super) fn save_playlists(&self) {
        let playlists: Vec<PlaylistData> = self
            .playlists
            .iter()
            .filter(|p| p.name != LIKED_PLAYLIST_NAME)
            .map(|p| PlaylistData {
                name: p.name.clone(),
                songs: p.songs.clone(),
            })
            .collect();
        let mut cfg = load_config();
        cfg.playlists = playlists;
        save_config(&cfg);
    }

    /// Deletes the playlist at `idx` from the app.
    ///
    /// The folder and its MP3 files are left untouched on disk; this only
    /// removes the playlist from the list and records the deletion so a folder
    /// scan does not bring it back. The Liked music playlist cannot be deleted
    /// here (it is managed via the heart button).
    pub(super) fn delete_playlist(&mut self, idx: usize) {
        if idx >= self.playlists.len() {
            return;
        }
        let name = self.playlists[idx].name.clone();
        if name == LIKED_PLAYLIST_NAME {
            return;
        }

        // 1) Remove from memory.
        self.playlists.remove(idx);

        // 2) Fix up the open page so it still points at the right playlist.
        match self.selected_playlist_idx {
            Some(sel) if sel == LIKED_PAGE_IDX => {}                       // Liked page: leave it
            Some(sel) if sel == idx => self.selected_playlist_idx = None,  // deleted the open one → Home
            Some(sel) if sel > idx => self.selected_playlist_idx = Some(sel - 1),
            _ => {}
        }

        // 3) Remember the deletion so it does not reappear on the next scan.
        let mut deleted = load_deleted_playlists();
        deleted.insert(name);
        save_deleted_playlists(&deleted);

        // 4) Rewrite saved playlists without the deleted one.
        self.save_playlists();
    }

    /// Handles paths dropped onto the window.
    ///
    /// * A **folder** becomes its own playlist named after the folder (audio is
    ///   collected recursively). A previously-deleted name is "un-deleted".
    /// * **Loose files** are added to the currently open playlist, or — if Home
    ///   or the Liked page is open — to a shared "Added tracks" playlist.
    ///
    /// After updating the model it persists the relevant parts and kicks off
    /// background metadata loading for any genuinely new tracks.
    pub(super) fn add_dropped_paths(&mut self, ctx: &egui::Context, paths: Vec<std::path::PathBuf>) {
        let mut loose_files: Vec<String> = Vec::new();
        let mut changed_playlists = false;
        let mut changed_liked = false;
        let mut new_meta_paths: Vec<String> = Vec::new();

        for path in paths {
            if path.is_dir() {
                // Folder → playlist named after the folder.
                let mut songs = Vec::new();
                collect_audio_files(&path, &mut songs);
                if songs.is_empty() {
                    continue;
                }
                let name = path
                    .file_name()
                    .map(|n| nfc(&n.to_string_lossy()))
                    .unwrap_or_else(|| "Новый плейлист".to_string());

                // If this name was previously deleted, bring it back.
                let mut deleted = load_deleted_playlists();
                if deleted.remove(&name) {
                    save_deleted_playlists(&deleted);
                }

                if let Some(pl) = self.playlists.iter_mut().find(|p| p.name == name) {
                    for s in &songs {
                        if !pl.songs.contains(s) {
                            pl.songs.push(s.clone());
                        }
                    }
                } else {
                    self.playlists.push(Playlist {
                        name,
                        songs: songs.clone(),
                    });
                }
                new_meta_paths.extend(songs);
                changed_playlists = true;
            } else if is_audio_file(&path) {
                if let Some(s) = path.to_str() {
                    loose_files.push(s.to_string());
                }
            }
        }

        // Loose files go to the open playlist, or a shared "Added tracks" one.
        if !loose_files.is_empty() {
            let target_idx = match self.selected_playlist_idx {
                Some(idx) if idx != LIKED_PAGE_IDX && idx < self.playlists.len() => Some(idx),
                _ => None,
            };

            if let Some(idx) = target_idx {
                let is_liked = self.playlists[idx].name == LIKED_PLAYLIST_NAME;
                for s in &loose_files {
                    if !self.playlists[idx].songs.contains(s) {
                        self.playlists[idx].songs.push(s.clone());
                    }
                }
                if is_liked {
                    changed_liked = true;
                } else {
                    changed_playlists = true;
                }
            } else {
                let name = match self.language {
                    Lang::Ru => "Добавленные треки",
                    Lang::Uk => "Додані треки",
                    Lang::En => "Added tracks",
                }
                .to_string();
                if let Some(pl) = self.playlists.iter_mut().find(|p| p.name == name) {
                    for s in &loose_files {
                        if !pl.songs.contains(s) {
                            pl.songs.push(s.clone());
                        }
                    }
                } else {
                    self.playlists.push(Playlist {
                        name,
                        songs: loose_files.clone(),
                    });
                }
                changed_playlists = true;
            }
            new_meta_paths.extend(loose_files);
        }

        if changed_liked {
            self.save_liked();
        }
        if changed_playlists {
            self.save_playlists();
        }

        // Load covers/tags for the new tracks (skipping dupes and known ones).
        let mut seen = HashSet::new();
        let to_load: Vec<String> = new_meta_paths
            .into_iter()
            .filter(|s| !self.track_meta.contains_key(s) && seen.insert(s.clone()))
            .collect();
        if !to_load.is_empty() {
            let tx = self.loader_tx.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                for path in to_load {
                    let meta = read_track_meta(&ctx, &path);
                    if tx.send(LoaderMsg::Meta(path, meta)).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            });
        }
    }
}
