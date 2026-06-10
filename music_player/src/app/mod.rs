//! The application: state, startup, and the per-frame update loop.
//!
//! [`App`] holds every piece of runtime state and implements [`eframe::App`].
//! The work is split across child modules so this file stays focused on the
//! struct definition and the high-level frame orchestration in [`App::update`]:
//!
//! * [`library`]  — likes, playlist membership, deletion, drag-and-drop import.
//! * [`playback`] — what plays next, transport actions, the playback queue.
//! * [`input`]    — keyboard handling and global media hotkeys.
//! * [`ui`]       — every panel, modal and overlay that gets drawn.
//!
//! ## Key model notes
//! * Playback uses a *snapshot* queue ([`App::playback_queue`]) captured when a
//!   track starts, so switching pages mid-song never changes "what plays next".
//! * The "Liked music" playlist is a normal [`Playlist`] whose name equals
//!   [`LIKED_PLAYLIST_NAME`]. In the sidebar it is shown via a dedicated button
//!   and addressed by the sentinel index [`LIKED_PAGE_IDX`] rather than a real
//!   list position.

mod input;
mod library;
mod playback;
mod ui;

use eframe::egui;
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::config::*;
use crate::lang::*;
use crate::meta::*;
use crate::player::Player;
use crate::scanner::{scan_music, LyricLine, Playlist};
use crate::shortcuts::*;

/// Name of the special "Liked music" playlist.
///
/// This exact string is also the on-disk key, so it must not change without a
/// migration. It is stored in Russian regardless of UI language; the display
/// layer swaps in the localized label when rendering.
pub(crate) const LIKED_PLAYLIST_NAME: &str = "Понравившаяся музыка";

/// Sentinel value for [`App::selected_playlist_idx`] meaning "the Liked music
/// page" (which has no real position in the playlist list).
pub(crate) const LIKED_PAGE_IDX: usize = usize::MAX;

/// Folder scanned for music and where new playlist folders are created,
/// relative to the executable's working directory.
pub(crate) const MUSIC_ROOT: &str = "../DownloadedMusic";

/// State of the self-update check/flow, shared with the background updater
/// thread via `Arc<Mutex<_>>`.
#[derive(Default, Clone)]
struct UpdateState {
    /// A newer release was found and the prompt should be shown.
    available: bool,
    /// Version string of the newer release.
    latest_version: String,
    /// The initial "is there an update?" check is still running.
    checking: bool,
    /// A download/replace is currently in progress.
    updating: bool,
    /// Error text from the last failed update attempt, if any.
    error: Option<String>,
}

/// All application state. Lives for the whole program run.
pub struct App {
    // --- Library ---
    /// All playlists, including the Liked music playlist (see
    /// [`LIKED_PLAYLIST_NAME`]).
    playlists: Vec<Playlist>,
    /// Per-track metadata (title/artist/cover), keyed by file path. Filled in
    /// gradually by the background loader.
    track_meta: HashMap<String, TrackMeta>,

    // --- Playback ---
    player: Player,
    /// Path of the track currently loaded into the player.
    current_song: String,
    /// Snapshot of the queue taken when the current track started. Page changes
    /// do not affect it, so next/prev stay predictable.
    playback_queue: Vec<String>,
    is_playing: bool,
    volume: f32,
    /// Total length of the current track, when known.
    total_duration: Option<Duration>,
    /// How far into the current track we are (advanced each frame by `dt`).
    elapsed_duration: Duration,
    /// Timestamp of the previous frame, used to compute `dt`.
    last_frame_instant: Instant,

    // --- Navigation / view ---
    /// Which page is open: `None` = Home, `Some(LIKED_PAGE_IDX)` = Liked music,
    /// `Some(i)` = `playlists[i]`.
    selected_playlist_idx: Option<usize>,
    search_query: String,

    /// Cached Home track list (deduplicated across playlists + search-filtered).
    /// With large libraries (thousands of tracks) rebuilding this every frame is
    /// expensive, so it is recomputed only when its inputs change — see
    /// [`App::home_track_list`] and the signature in [`App::home_cache_sig`].
    home_cache: Vec<String>,
    /// Signature the cache was built for: `(playlist count, total song count,
    /// loaded-metadata count, search query)`. When it changes, the cache is
    /// rebuilt.
    home_cache_sig: (usize, usize, usize, String),

    // --- Background loading channel ---
    /// Receiver for messages from the background scanner/metadata loader.
    loader_rx: Receiver<LoaderMsg>,
    /// A clone of the loader sender, used to load metadata for tracks added at
    /// runtime via drag-and-drop.
    loader_tx: Sender<LoaderMsg>,

    // --- Settings / localization ---
    show_settings: bool,
    language: Lang,

    /// Whether the full "Now Playing" view (big cover + synced lyrics, with a
    /// cover-adaptive background) is open. Toggled by clicking the bottom-bar
    /// cover; closed by its chevron.
    show_now_playing: bool,

    // --- Hotkeys ---
    /// Current action → key bindings.
    shortcuts: HashMap<Shortcut, egui::Key>,
    /// The action awaiting a key press in settings (`None` = not rebinding).
    rebinding: Option<Shortcut>,
    /// Receiver of globally-captured key presses from the `rdev::grab` thread.
    global_key_rx: Receiver<egui::Key>,
    /// State shared with the grab thread (which keys to swallow, whether active).
    grab_shared: Arc<Mutex<GrabShared>>,

    // --- Self-update ---
    update: Arc<Mutex<UpdateState>>,

    // --- "New playlist" modal ---
    show_new_playlist: bool,
    new_playlist_name: String,
    /// Request focus for the input field exactly once after opening.
    focus_new_playlist: bool,

    // --- Lyrics ---
    /// Lyrics for the current track, if loaded.
    current_lyrics: Option<Vec<LyricLine>>,
    /// Playback position used to highlight the active lyric line, in ms.
    current_playback_time_ms: u32,
    /// When the current track started (reserved for lyric timing).
    song_start_time: Option<Instant>,
    /// Receiver for lyrics being fetched on a background thread.
    lyrics_receiver: Option<Receiver<Option<Vec<LyricLine>>>>,
    /// Cache of already-fetched lyrics, keyed by track path, to avoid refetching.
    lyrics_cache: HashMap<String, Vec<LyricLine>>,

    // --- Track "⋮" context menu (playlist page) ---
    /// Path of the track whose context menu is open, if any.
    track_context_menu: Option<String>,
    /// Fixed on-screen position of that popup.
    context_menu_pos: egui::Pos2,
    /// Skip the first frame's outside-click check, so opening does not
    /// immediately close the menu.
    context_menu_just_opened: bool,

    // --- "Rename playlist" modal ---
    rename_playlist_idx: Option<usize>,
    rename_playlist_name: String,
    focus_rename_playlist: bool,
}

impl App {
    /// Builds the app and kicks off all background work.
    ///
    /// Three threads are spawned here so the window appears instantly instead
    /// of blocking on I/O:
    /// 1. Global hotkey grabber (`rdev::grab`) — captures media keys system-wide.
    /// 2. Library loader — scans folders, merges saved playlists/likes, then
    ///    streams per-track metadata (tags + covers) back to the UI.
    /// 3. Update checker — asks GitHub whether a newer release exists.
    pub fn new(ctx: &egui::Context) -> Self {
        // Apply the theme once at startup. egui keeps visuals until changed, so
        // there is no need to rebuild them every frame.
        crate::theme::apply_custom_theme(ctx);

        let (tx, loader_rx) = channel();
        // Extra sender so drag-and-drop imports can request metadata loading.
        let loader_tx = tx.clone();

        // --- Thread 1: GLOBAL hotkeys via rdev::grab ---
        // `grab` intercepts the keyboard system-wide and, unlike `listen`, can
        // "swallow" a press (return None) so Windows stays silent and the key
        // does not reach the focused game/app. What to swallow comes from
        // `grab_shared`, kept in sync by the UI each frame.
        let (global_tx, global_key_rx) = channel::<egui::Key>();
        let grab_shared = Arc::new(Mutex::new(GrabShared {
            keys: std::collections::HashSet::new(),
            active: true,
        }));
        let shared_for_thread = grab_shared.clone();
        let ctx_global = ctx.clone();
        thread::spawn(move || {
            let callback = move |event: rdev::Event| -> Option<rdev::Event> {
                if let rdev::EventType::KeyPress(rkey) = event.event_type {
                    if let Some(ekey) = rdev_to_egui(rkey) {
                        let consume = {
                            let st = shared_for_thread.lock().unwrap();
                            st.active && st.keys.contains(&ekey)
                        };
                        if consume {
                            let _ = global_tx.send(ekey);
                            ctx_global.request_repaint(); // wake the UI even if minimized
                            return None; // swallow: silent + does not reach other apps
                        }
                    }
                }
                Some(event) // pass everything else through
            };
            if let Err(err) = rdev::grab(callback) {
                eprintln!("⚠️ Failed to start global hotkeys: {:?}", err);
            }
        });

        // --- Thread 2: library scan + metadata streaming ---
        let ctx_clone = ctx.clone();
        thread::spawn(move || {
            let mut playlists = scan_music(MUSIC_ROOT);

            // Hide playlists the user deleted (their files stay on disk).
            let deleted = load_deleted_playlists();
            if !deleted.is_empty() {
                playlists.retain(|p| !deleted.contains(&p.name));
            }

            // Merge manually-added tracks (added via "⋮"/right-click) saved in
            // the config back into their playlists.
            for (name, songs) in load_saved_playlists() {
                if name == LIKED_PLAYLIST_NAME || deleted.contains(&name) {
                    continue; // likes are restored separately; deleted stay gone
                }
                if let Some(p) = playlists.iter_mut().find(|p| p.name == name) {
                    for s in songs {
                        // Skip duplicates and paths to files that no longer exist.
                        if std::path::Path::new(&s).exists() && !p.songs.contains(&s) {
                            p.songs.push(s);
                        }
                    }
                } else {
                    // Saved playlist whose folder the scanner did not find: recreate it.
                    let songs: Vec<String> = songs
                        .into_iter()
                        .filter(|s| std::path::Path::new(s).exists())
                        .collect();
                    playlists.push(Playlist { name, songs });
                }
            }

            // Gather every unique track path for metadata loading (a track can
            // appear in several playlists, but we load it only once).
            let mut seen_paths = std::collections::HashSet::new();
            let all_paths: Vec<String> = playlists
                .iter()
                .flat_map(|p| p.songs.iter().cloned())
                .filter(|s| seen_paths.insert(s.clone()))
                .collect();

            // Restore the saved Liked music playlist and put it first, mirroring
            // what the heart button does at runtime.
            let liked = load_liked_songs();
            if !liked.is_empty() {
                playlists.insert(
                    0,
                    Playlist {
                        name: LIKED_PLAYLIST_NAME.to_string(),
                        songs: liked,
                    },
                );
            }

            // Send playlists first so cards appear immediately (without covers).
            if tx.send(LoaderMsg::Playlists(playlists)).is_err() {
                return; // window already closed
            }

            // Then stream covers/tags one by one; they fill in as they arrive.
            for path in all_paths {
                let meta = read_track_meta(&ctx_clone, &path);
                if tx.send(LoaderMsg::Meta(path, meta)).is_err() {
                    break; // window closed
                }
                ctx_clone.request_repaint(); // wake the UI to show the new card
            }
        });

        // --- Thread 3: check GitHub for a newer release ---
        let update = Arc::new(Mutex::new(UpdateState {
            checking: true,
            ..Default::default()
        }));
        {
            let update_clone = update.clone();
            let ctx_update = ctx.clone();
            thread::spawn(move || {
                let result = self_update::backends::github::Update::configure()
                    .repo_owner("Px228-Da-Da")
                    .repo_name("Elysium")
                    .bin_name("Elysium")
                    .show_download_progress(false)
                    .current_version(env!("CARGO_PKG_VERSION"))
                    .build();

                if let Ok(updater) = result {
                    if let Ok(release) = updater.get_latest_release() {
                        let latest = release.version.trim_start_matches('v').to_string();
                        let mut state = update_clone.lock().unwrap();
                        state.checking = false;

                        // Only prompt if the release is strictly newer than us.
                        let current = env!("CARGO_PKG_VERSION");
                        let newer =
                            self_update::version::bump_is_greater(current, &latest).unwrap_or(false);
                        if newer {
                            state.available = true;
                            state.latest_version = latest;
                            // Wake the UI so the prompt shows even if it is idle.
                            ctx_update.request_repaint();
                        }
                    }
                }
            });
        }

        Self {
            playlists: Vec::new(),
            player: Player::new(),
            current_song: String::new(),
            playback_queue: Vec::new(),
            is_playing: false,
            volume: 0.5,
            total_duration: None,
            elapsed_duration: Duration::ZERO,
            last_frame_instant: Instant::now(),
            selected_playlist_idx: None,
            track_meta: HashMap::new(),
            loader_rx,
            loader_tx,
            search_query: String::new(),
            home_cache: Vec::new(),
            home_cache_sig: (0, 0, 0, String::new()),
            show_settings: false,
            language: load_language(),
            show_now_playing: false,
            shortcuts: load_shortcuts(),
            rebinding: None,
            global_key_rx,
            grab_shared,
            update,
            show_new_playlist: false,
            new_playlist_name: String::new(),
            focus_new_playlist: false,
            current_lyrics: None,
            current_playback_time_ms: 0,
            song_start_time: None,
            lyrics_receiver: None,
            lyrics_cache: HashMap::new(),
            track_context_menu: None,
            context_menu_pos: egui::pos2(0.0, 0.0),
            context_menu_just_opened: false,
            rename_playlist_idx: None,
            rename_playlist_name: String::new(),
            focus_rename_playlist: false,
        }
    }
}

impl eframe::App for App {
    /// Runs once per frame: advances state, drains background channels, and
    /// draws every panel/overlay. Heavy work stays on background threads; this
    /// only consumes their results so the UI never blocks.
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 1. Advance the lyrics clock while playing.
        if self.is_playing {
            // We use our own `elapsed_duration` timer because rodio's position
            // query is not available in this version.
            self.current_playback_time_ms = self.elapsed_duration.as_millis() as u32;
        }

        // 2. Did a background lyrics fetch finish?
        if let Some(rx) = &self.lyrics_receiver {
            if let Ok(lyrics_result) = rx.try_recv() {
                self.current_lyrics = lyrics_result.clone();
                self.lyrics_receiver = None;
                // Cache successful results so we never refetch the same track.
                if let Some(lyrics) = lyrics_result {
                    self.lyrics_cache.insert(self.current_song.clone(), lyrics);
                }
            }
        }

        // 3. Drain the loader channel: take whatever is ready, draw it now.
        while let Ok(msg) = self.loader_rx.try_recv() {
            match msg {
                LoaderMsg::Playlists(playlists) => self.playlists = playlists,
                LoaderMsg::Meta(path, meta) => {
                    self.track_meta.insert(path, meta);
                }
            }
        }

        // 4. Drag-and-drop: folder → new playlist, files → open playlist.
        let dropped: Vec<std::path::PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if !dropped.is_empty() {
            self.add_dropped_paths(ctx, dropped);
        }

        // 5. Advance playback time and auto-advance at end of track.
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame_instant);
        self.last_frame_instant = now;
        if self.is_playing {
            self.elapsed_duration += dt;
            if let Some(total) = self.total_duration {
                if self.elapsed_duration >= total {
                    self.play_next_track();
                }
            }
        }

        // 6. Keyboard: rebinding capture, then global hotkey actions.
        self.handle_shortcuts(ctx);
        self.handle_global_keys(ctx);

        // 7. Draw everything. Order matters: panels first, overlays last so
        //    modals and hints render on top.
        self.ui_update_modal(ctx);
        self.ui_bottom_bar(ctx);
        self.ui_sidebar(ctx);
        self.ui_central(ctx);
        self.ui_settings(ctx);
        self.ui_new_playlist(ctx);
        self.ui_rename_playlist(ctx);
        self.ui_drop_hint(ctx);

        // Only schedule periodic repaints while there is something to animate:
        // playback progress + lyrics, or a pending lyrics fetch. When fully idle
        // (paused, nothing loading) the UI sleeps until the next input or
        // background event — a large CPU/GPU saving on weak machines. Background
        // loader threads call `request_repaint` themselves when they have data,
        // and any mouse/keyboard input wakes the UI on its own.
        if self.is_playing || self.lyrics_receiver.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
}
