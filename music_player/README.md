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
- 🔎 **Online search ("ЮБ" tab)** — search YouTube Music for tracks and play their
  audio, fetched on demand with [`yt-dlp`] (and transcoded with `ffmpeg`). See the
  [Disclaimer](#disclaimer) below.
- 🌍 **Localization** — Russian, Ukrainian and English, switchable at runtime.
- ⬆️ **Self-update** — checks GitHub for a newer release and can update in place.

[`yt-dlp`]: https://github.com/yt-dlp/yt-dlp

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

### Runtime dependencies (online search)

The local library, playback and lyrics work out of the box. The optional **"ЮБ"**
search tab additionally uses two external command-line tools, `yt-dlp` and
`ffmpeg`. If they are not found on `PATH`, the app downloads them into its data
folder (`%APPDATA%/Elysium/tools`) on first use. The downloads are Windows builds;
on other platforms install `yt-dlp` and `ffmpeg` yourself.

This tab makes network requests to YouTube; the rest of the app only goes online
for lyrics and the update check.

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

---

## Disclaimer

Elysium is a personal hobby project, provided **as-is** for personal and
educational use. The optional "ЮБ" tab retrieves audio through the third-party
tool [`yt-dlp`]; this project is not affiliated with, endorsed by, or sponsored
by YouTube or Google. You are solely responsible for using it in accordance with
the terms of service of any platform you access and with the copyright laws of
your jurisdiction. The authors accept no liability for misuse.

If you are a rights holder and have a concern, please open an issue.

---

## Licenses

Elysium's own source code is released under the **MIT License** (see
[`LICENSE`](../LICENSE)). The bundled Noto fonts in
[`src/fonts/`](src/fonts/) are licensed separately under the **SIL Open Font
License 1.1** (see [`src/fonts/OFL.txt`](src/fonts/OFL.txt)).
