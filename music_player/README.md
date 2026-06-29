# Elysium

A lightweight desktop music player written in Rust with an immediate-mode UI
([`egui`] / [`eframe`]) and a Spotify-like dark theme. It scans a local music
folder into playlists, plays audio with [`rodio`], shows embedded cover art, and
finds time-synced ("karaoke") lyrics — from the file's own tags or online.

[`egui`]: https://github.com/emilk/egui
[`eframe`]: https://docs.rs/eframe
[`rodio`]: https://docs.rs/rodio

---

## Features

- 🎵 **Library scanning** — every subfolder of the music root with MP3s becomes a
  playlist; loose MP3s become an "all tracks" playlist.
- ▶️ **Playback** — play/pause, next/previous, seek, and volume, with a snapshot
  playback queue so navigating the UI mid-song never changes what plays next.
- ❤️ **Likes** — a built-in "Liked music" playlist, toggled from any track.
- 📂 **Playlists** — create, rename and delete playlists; add tracks via the
  "⋮" menu or right-click. Deleting a playlist never touches files on disk.
- 🖱️ **Drag-and-drop** — drop a folder (becomes a playlist) or files (added to
  the open playlist) onto the window.
- 🎤 **Synced lyrics** — pulled from the MP3's `SYLT` tag, or fetched online from
  lrclib and NetEase, with an in-memory cache.
- ⌨️ **Global media hotkeys** — play/pause, next, previous, volume and like work
  even when the window is unfocused (e.g. while gaming).
- 🌍 **Localization** — Russian and Ukrainian, switchable at runtime.
- ⬆️ **Self-update** — checks GitHub for a newer release and can update in place.

---

## Building and running

Requires a recent stable Rust toolchain.

```sh
# from the music_player/ directory
cargo run --release
```

By default the app scans the folder **`../DownloadedMusic`** (i.e. a
`DownloadedMusic` folder sitting *next to* the `music_player` directory, not
inside it). Newly created playlists also get a folder there. To change the
location, edit `MUSIC_ROOT` in [`src/app/mod.rs`](src/app/mod.rs).

### Where data is stored

All persistent state lives in a single `config.json`:

- Windows: `%APPDATA%\Elysium\config.json`
- Linux: `~/.config/Elysium/config.json`

It holds liked tracks, user playlists, deleted-playlist names, the chosen
language and key bindings. Legacy `.txt` state files (from older versions) are
migrated into it automatically on first launch.

---

## Project structure

```
src/
├── main.rs            Entry point: fonts, window setup, launches App.
├── app/               The application itself.
│   ├── mod.rs         App struct, startup threads, per-frame update loop.
│   ├── library.rs     Likes, playlist membership, deletion, drag-and-drop import.
│   ├── playback.rs    Track stepping, transport actions, the playback queue.
│   ├── input.rs       Keyboard handling and global media hotkeys.
│   └── ui/            All on-screen drawing (one module per region).
│       ├── bottom_bar.rs    Transport controls, progress, volume.
│       ├── sidebar.rs       Navigation, New playlist, Liked music, playlist list.
│       ├── central.rs       Search row + dispatch to the active page.
│       ├── home_page.rs     "Listen again" card grid.
│       ├── playlist_page.rs A playlist / the Liked music page.
│       ├── settings.rs      Language + hotkey settings overlay.
│       ├── modals.rs        Update prompt, New playlist, Rename dialogs.
│       ├── now_playing.rs   Full cover + synced-lyrics view (adaptive background).
│       └── lyrics.rs        Drag-and-drop hint overlay.
├── player.rs          rodio wrapper: play, pause, seek (background), volume.
├── scanner.rs         Folder scanning + lyrics lookup (embedded + online).
├── meta.rs            Per-track metadata: title, artist, cover texture.
├── audio.rs           Audio-file detection + recursive collection.
├── config.rs          config.json load/save + legacy .txt migration.
├── lang.rs            Languages and the UI string tables.
├── shortcuts.rs       Hotkey actions, key bindings, rdev↔egui key mapping.
└── theme.rs           Shared palette (ACCENT, TEXT_MUTED, BG_MAIN) + helpers.
```

The UI never blocks: scanning, metadata/cover loading, lyrics fetching, seeking
and the update check all run on background threads and hand results back to the
UI thread over channels.

---

## How to extend

### Add a language
1. Add a variant to `Lang` in [`src/lang.rs`](src/lang.rs) and update its small
   `match`es (`all`, `native_name`, `code`, `from_code`).
2. Add a matching arm to `strings()` filling in **every** field of `Strings`.

### Add or change a hotkey action
1. Add a variant to `Shortcut` in [`src/shortcuts.rs`](src/shortcuts.rs) and
   update `all`, `code`, `from_code`, `label`, and `default_shortcuts`.
2. Handle it in `App::do_shortcut` in [`src/app/playback.rs`](src/app/playback.rs).

### Make a key bindable
Add the `egui::Key` to `BINDABLE_KEYS` in [`src/shortcuts.rs`](src/shortcuts.rs)
(and, if it should also work as a *global* hotkey, add the `rdev::Key` mapping in
`rdev_to_egui`).

### Add a lyrics source
Add a provider function in [`src/scanner.rs`](src/scanner.rs) and call it in the
chain inside `fetch_lyrics_from_internet` (the first provider to return a result
wins).

---

## Notes & caveats

- The **"Liked music"** playlist is identified by an exact name string
  (`LIKED_PLAYLIST_NAME`) that is also its on-disk key. It is always stored in
  Russian regardless of UI language; the display layer swaps in the localized
  label. Do not rename it without a config migration.
- Global hotkeys use `rdev::grab`, which on some platforms needs accessibility
  permissions to capture keys system-wide.
