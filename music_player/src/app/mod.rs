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
mod onboarding;
mod playback;
mod ui;
mod youtube;

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

/// Progress of the post-onboarding library scan, shown on the loading screen.
#[derive(Default)]
pub(crate) struct LoadingState {
    /// Set once the initial playlist scan has arrived (so an empty library still
    /// finishes loading instead of hanging on the screen).
    scanned: bool,
    /// Total unique tracks to load metadata for (known once playlists arrive).
    total: usize,
    /// How many have loaded so far.
    loaded: usize,
    /// File name of the most recently loaded track.
    last_file: String,
}

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
    /// Current window background color, eased toward a dark tint of the playing
    /// track's cover so the whole app subtly takes on the song's color.
    bg_tint: egui::Color32,
    /// Set once the font atlas has been pre-warmed (on the first frame), so it
    /// only happens a single time. See [`App::prewarm_fonts`].
    fonts_prewarmed: bool,
    /// Total length of the current track, when known.
    total_duration: Option<Duration>,
    /// Receives the track length computed off-thread when the decoder could not
    /// report it instantly (see [`App::play_track`]).
    duration_receiver: Option<Receiver<Option<Duration>>>,
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

    // --- First-run onboarding ---
    /// The welcome wizard, present only on the very first launch.
    onboarding: Option<onboarding::Onboarding>,
    /// The post-onboarding loading screen, present while the initial scan runs.
    loading: Option<LoadingState>,

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

    /// Index of the playlist awaiting a delete confirmation, if any.
    confirm_delete_playlist: Option<usize>,
    focus_rename_playlist: bool,

    // --- YouTube ("ЮБ") tab ---
    /// Whether the ad-free YouTube Music search tab is the active central view.
    show_youtube: bool,
    /// Current text in the YouTube search box (independent of the library search).
    yt_query: String,
    /// Results of the last YouTube search.
    yt_results: Vec<crate::ytdl::YtTrack>,
    /// A search is in flight on a background thread.
    yt_searching: bool,
    /// At least one search has completed (so an empty list means "nothing found"
    /// rather than the initial empty state).
    yt_searched: bool,
    /// Receives the result of the in-flight search.
    yt_search_rx: Option<Receiver<Result<Vec<crate::ytdl::YtTrack>, String>>>,
    /// Track currently being resolved for streaming (shows a spinner on its row
    /// and carries the title/artist/duration to display once it starts).
    yt_loading_track: Option<crate::ytdl::YtTrack>,
    /// Receives the resolved `(stream URL, ffmpeg path)` (or an error) for that track.
    yt_play_rx: Option<Receiver<Result<(String, std::path::PathBuf), String>>>,
    /// Cache of resolved direct stream URLs, keyed by video id. Filled both by
    /// playback and by background prefetch of the top results, so clicking a
    /// (prefetched) result starts almost instantly. URLs are valid for hours.
    yt_url_cache: Arc<Mutex<HashMap<String, String>>>,
    /// Result thumbnails, keyed by video id: the cover texture plus a dark
    /// average color for the Now Playing background. Loaded in the background
    /// after a search and drawn on the result cards / player.
    yt_thumbs: Arc<Mutex<HashMap<String, (egui::TextureHandle, egui::Color32)>>>,
    /// Live progress line written by the background backend (tool download, etc.).
    yt_status: crate::ytdl::Status,
    /// Last error to show in the tab, if any.
    yt_error: Option<String>,

    // --- Lab (audio analysis) window ---
    /// Whether the separate Lab viewport window is open.
    show_lab: bool,
    /// Persistent Lab UI state (visible widgets, smoothing, spectrogram).
    lab_state: crate::lab::LabState,
    /// Pre-rendered whole-track waveform for the current song, computed
    /// off-thread when a track loads. Shared with the Lab window.
    waveform: crate::lab::dsp::SharedWaveform,
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
        // Show the welcome wizard until it has been completed (or skipped) once.
        // Using the `onboarded` flag — not mere file existence — means existing
        // users whose config predates onboarding (`onboarded` defaults to false)
        // also see it. Their likes/playlists are preserved: the wizard only
        // rewrites its own config fields on finish. The library scan is deferred
        // until the wizard ends.
        let cfg = crate::config::load_config();
        let first_run = !cfg.onboarded;

        // Resolve the accent color the user picked in onboarding (stored as a
        // key) and install it globally, so the whole UI uses it. Unknown/empty
        // values keep the default accent.
        if let Some((_, color)) = onboarding::ACCENTS.iter().find(|(key, _)| *key == cfg.accent) {
            crate::theme::set_accent(*color);
        }
        // Light or dark palette, per the saved choice (defaults to dark).
        crate::theme::set_light_theme(cfg.theme == "light");

        // Enable egui's image loaders (SVG icons are drawn through them).
        egui_extras::install_image_loaders(ctx);

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
                        // This callback IS a Windows low-level keyboard hook
                        // (WH_KEYBOARD_LL). It must return almost instantly or
                        // Windows silently drops/kills the hook, which makes the
                        // global hotkeys stop working until restart. So never
                        // *block* on the mutex here: if the UI thread is holding
                        // it (or it is poisoned), just let the key pass through
                        // rather than stall the whole system's keyboard input.
                        let consume = match shared_for_thread.try_lock() {
                            Ok(st) => st.active && st.keys.contains(&ekey),
                            Err(_) => false,
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
        // On the very first launch this is deferred until the welcome wizard
        // finishes (the user chooses their folders there). Otherwise it starts
        // immediately, scanning the folders picked previously (or the default
        // when none were chosen).
        if !first_run {
            let cfg = load_config();
            // Legacy installs (never onboarded) keep scanning the default folder;
            // onboarded users get exactly what they chose (possibly nothing).
            let default_if_empty = !cfg.onboarded;
            spawn_library_scan(tx.clone(), ctx.clone(), cfg.music_folders, cfg.music_files, default_if_empty);
        }

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
            bg_tint: crate::theme::bg_main(),
            fonts_prewarmed: false,
            total_duration: None,
            duration_receiver: None,
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
            onboarding: if first_run { Some(onboarding::Onboarding::new()) } else { None },
            loading: None,
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
            confirm_delete_playlist: None,
            focus_rename_playlist: false,
            show_youtube: false,
            yt_query: String::new(),
            yt_results: Vec::new(),
            yt_searching: false,
            yt_searched: false,
            yt_search_rx: None,
            yt_loading_track: None,
            yt_play_rx: None,
            yt_url_cache: Arc::new(Mutex::new(HashMap::new())),
            yt_thumbs: Arc::new(Mutex::new(HashMap::new())),
            yt_status: Arc::new(Mutex::new(String::new())),
            yt_error: None,
            show_lab: false,
            lab_state: crate::lab::LabState::default(),
            waveform: std::sync::Arc::new(Mutex::new(None)),
        }
    }
}

impl App {
    /// Starts the background library scan for the chosen music `roots` plus any
    /// individually chosen `files`. Used after the welcome wizard finishes; the
    /// user's selection is taken literally (an empty selection loads nothing).
    pub(in crate::app) fn start_library_scan(&self, ctx: &egui::Context, roots: Vec<String>, files: Vec<String>) {
        spawn_library_scan(self.loader_tx.clone(), ctx.clone(), roots, files, false);
    }
}

/// Scans `roots` plus any loose `files` on a background thread, sends the
/// assembled playlists, then streams per-track metadata back over `tx`.
///
/// When `roots` is empty: `default_if_empty` decides whether to fall back to the
/// legacy [`MUSIC_ROOT`] (for pre-onboarding installs) or to load nothing (when
/// the user finished onboarding without choosing any folder).
fn spawn_library_scan(
    tx: Sender<LoaderMsg>,
    ctx: egui::Context,
    roots: Vec<String>,
    files: Vec<String>,
    default_if_empty: bool,
) {
    thread::spawn(move || {
        let mut playlists = if !roots.is_empty() {
            crate::scanner::scan_music_paths(&roots)
        } else if default_if_empty {
            scan_music(MUSIC_ROOT)
        } else {
            Vec::new()
        };

        // Add individually dragged-in files into an "all tracks" playlist
        // (merging with an existing one from a folder scan if present).
        let files: Vec<String> = files
            .into_iter()
            .filter(|f| std::path::Path::new(f).exists())
            .collect();
        if !files.is_empty() {
            const ALL: &str = "Усі треки";
            if let Some(p) = playlists.iter_mut().find(|p| p.name == ALL) {
                for f in files {
                    if !p.songs.contains(&f) {
                        p.songs.push(f);
                    }
                }
            } else {
                playlists.push(Playlist { name: ALL.to_string(), songs: files });
            }
        }

        // Drop exact-duplicate songs across the chosen folders/files. Two files
        // with the same name *and* the same byte size are treated as the same
        // track (a copy) and only the first is kept — otherwise selecting
        // overlapping folders would fill the library with identical songs.
        {
            let mut seen: std::collections::HashSet<(String, u64)> = std::collections::HashSet::new();
            for pl in &mut playlists {
                pl.songs.retain(|s| {
                    let p = std::path::Path::new(s);
                    match (p.file_name(), std::fs::metadata(p).ok()) {
                        (Some(name), Some(meta)) => {
                            seen.insert((name.to_string_lossy().to_lowercase(), meta.len()))
                        }
                        // Can't identify it (unnamed / unreadable) — keep it.
                        _ => true,
                    }
                });
            }
            // A playlist emptied entirely by dedup is dropped.
            playlists.retain(|p| !p.songs.is_empty());
        }

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
            let meta = read_track_meta(&ctx, &path);
            if tx.send(LoaderMsg::Meta(path, meta)).is_err() {
                break; // window closed
            }
            ctx.request_repaint(); // wake the UI to show the new card
        }
    });
}

impl eframe::App for App {
    /// Runs once per frame: advances state, drains background channels, and
    /// draws every panel/overlay. Heavy work stays on background threads; this
    /// only consumes their results so the UI never blocks.
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Warm the font atlas on the very first frame, before any UI is painted,
        // so it reaches its final size without growing mid-frame later (which
        // panics in epaint 0.29). Done here — not in `new` — because fonts are
        // unavailable until the first `Context::run`.
        if !self.fonts_prewarmed {
            prewarm_font_atlas(ctx);
            self.fonts_prewarmed = true;
        }

        // 0. First-run welcome wizard takes over the whole window until done.
        if self.onboarding.is_some() {
            let dropped: Vec<std::path::PathBuf> = ctx.input(|i| {
                i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect()
            });
            if !dropped.is_empty() {
                self.onboarding_handle_drop(&dropped);
            }
            self.ui_onboarding(ctx);
            return;
        }

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
                LoaderMsg::Playlists(playlists) => {
                    // Track total unique songs for the first-run loading screen.
                    if let Some(load) = &mut self.loading {
                        let mut seen = std::collections::HashSet::new();
                        load.total = playlists
                            .iter()
                            .flat_map(|p| p.songs.iter())
                            .filter(|s| seen.insert((*s).clone()))
                            .count();
                        load.scanned = true;
                    }
                    self.playlists = playlists;
                }
                LoaderMsg::Meta(path, meta) => {
                    if let Some(load) = &mut self.loading {
                        load.loaded += 1;
                        load.last_file = std::path::Path::new(&path)
                            .file_name()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_default();
                    }
                    self.track_meta.insert(path, meta);
                }
            }
        }

        // 3b. First-run loading screen: shown while the initial scan runs, then
        //     it hands off to the normal UI once every track's metadata is in.
        if let Some(load) = &self.loading {
            if load.scanned && load.loaded >= load.total {
                self.loading = None;
            } else {
                self.ui_loading(ctx);
                return;
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

        // 4b. Pick up a track length computed off-thread (see `play_track`).
        if let Some(rx) = &self.duration_receiver {
            if let Ok(duration) = rx.try_recv() {
                self.total_duration = duration;
                self.duration_receiver = None;
            }
        }

        // 4c. YouTube tab: drain its background search/download results.
        self.poll_youtube(ctx);

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

        // 6b. Ease the window background toward a dark tint of the current
        //     track's cover, so the app subtly takes on the song's color while
        //     something is loaded. Falls back to the neutral base when no cover.
        //     Skipped in the light theme, where a dark cover tint would muddy the
        //     bright palette.
        let bg_target = if crate::theme::is_light() || self.current_song.is_empty() {
            crate::theme::bg_main()
        } else {
            // Cover tone: the real cover's, or a dark version of the generated
            // cover's gradient when the track has no embedded art.
            let cover_bg = self.track_meta.get(&self.current_song).and_then(|m| m.bg).unwrap_or_else(|| {
                let (_, deep) = crate::theme::gen_gradient(&self.current_song);
                crate::theme::lerp_color(deep, egui::Color32::BLACK, 0.45)
            });
            crate::theme::lerp_color(crate::theme::bg_main(), cover_bg, 0.55)
        };
        let prev = self.bg_tint;
        self.bg_tint = crate::theme::lerp_color(prev, bg_target, 0.08);
        // Keep animating until it has essentially reached the target.
        let diff = (prev.r() as i32 - bg_target.r() as i32).abs()
            + (prev.g() as i32 - bg_target.g() as i32).abs()
            + (prev.b() as i32 - bg_target.b() as i32).abs();
        if diff > 2 {
            ctx.request_repaint();
        }

        // 7. Draw everything. Order matters: panels first, overlays last so
        //    modals and hints render on top.
        self.ui_update_modal(ctx);
        self.ui_bottom_bar(ctx);
        self.ui_sidebar(ctx);
        self.ui_central(ctx);
        self.ui_settings(ctx);
        self.ui_new_playlist(ctx);
        self.ui_rename_playlist(ctx);
        self.ui_confirm_delete_playlist(ctx);
        self.ui_drop_hint(ctx);
        self.ui_lab(ctx);

        // Only schedule periodic repaints while there is something to animate:
        // playback progress + lyrics, or a pending lyrics fetch. When fully idle
        // (paused, nothing loading) the UI sleeps until the next input or
        // background event — a large CPU/GPU saving on weak machines. Background
        // loader threads call `request_repaint` themselves when they have data,
        // and any mouse/keyboard input wakes the UI on its own.
        if self.is_playing || self.lyrics_receiver.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        // Keep the UI ticking while a YouTube search/download runs so its status
        // line and spinner update even when nothing else is animating.
        if self.yt_searching || self.yt_loading_track.is_some() {
            ctx.request_repaint_after(Duration::from_millis(120));
        }
        // The Lab window's live scopes need a steady high frame rate; drive the
        // parent loop (which hosts the immediate viewport) at ~60 fps while open.
        if self.show_lab {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }
}

/// Lays out every glyph the UI is likely to draw (ASCII, Latin-1, the full
/// Cyrillic block and the emoji we use) at the sizes we render, forcing the font
/// atlas to reach its final size in one go.
///
/// Without this, epaint 0.29 can panic with "Partial texture update is outside
/// the bounds of texture Managed(0)" the first time a *new* glyph (e.g. a
/// Cyrillic letter from a freshly-loaded track title or lyric) makes the atlas
/// grow mid-frame. Growing it once, up front, avoids that. Must run inside a
/// frame (fonts are unavailable before the first `Context::run`).
fn prewarm_font_atlas(ctx: &egui::Context) {
    use egui::{Color32, FontFamily, FontId};

    // Small/medium text glyphs: ASCII + Latin-1 + Cyrillic (+ supplement).
    let mut text = String::new();
    for c in (0x20u32..0x7f).chain(0xa0..0x180).chain(0x400..0x510) {
        if let Some(ch) = char::from_u32(c) {
            text.push(ch);
        }
    }
    text.push_str("📌📁🎵🛠⬇➕👤🔊🔍❤✏🗑⚙✖▶⏸⏮⏭⏳🏠📺→←…•");

    // Big glyphs are only used for generated-cover labels (leading digits/caps).
    let big: String = ('0'..='9')
        .chain('A'..='Z')
        .chain('А'..='Я')
        .chain(std::iter::once('#'))
        .collect();

    ctx.fonts(|fonts| {
        let layout = |s: &str, size: f32, fam: FontFamily| {
            let _ = fonts.layout_no_wrap(s.to_string(), FontId::new(size, fam), Color32::WHITE);
        };
        for &sz in &[11.0, 12.0, 13.0, 13.5, 14.0, 15.0, 16.0, 17.0, 18.0, 20.0, 22.0, 24.0, 28.0, 30.0, 34.0] {
            layout(&text, sz, FontFamily::Proportional);
            layout(&text, sz, FontFamily::Monospace);
        }
        for &sz in &[40.0, 56.0, 76.0, 100.0] {
            layout(&big, sz, FontFamily::Proportional);
        }
    });
}
