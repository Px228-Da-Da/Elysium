# Elysium

A lightweight desktop **music player** written in Rust with an immediate-mode UI
([`egui`] / [`eframe`]) and a Spotify-like dark theme. It scans a local music
folder into playlists, plays audio with [`rodio`], shows cover art and
time-synced ("karaoke") lyrics, and has an optional online-search tab.

> The application lives in the [`music_player/`](music_player/) directory.
> See [`music_player/README.md`](music_player/README.md) for full documentation,
> architecture notes and contribution guides.

[`egui`]: https://github.com/emilk/egui
[`eframe`]: https://docs.rs/eframe
[`rodio`]: https://docs.rs/rodio

---

## Features

- 🎵 **Library scanning** into playlists, with embedded cover art.
- ▶️ **Playback** — play/pause, next/previous, seek, volume, snapshot queue.
- ❤️ **Likes** and 📂 **playlists** (create / rename / delete; drag-and-drop import).
- 🎤 **Synced lyrics** from the file's `SYLT` tag or online (lrclib / NetEase).
- ⌨️ **Global media hotkeys** that work even when the window is unfocused.
- 🔎 **Online search ("ЮБ" tab)** — search for tracks and play their audio,
  fetched on demand via [`yt-dlp`] (and transcoded with `ffmpeg`). See the
  [Disclaimer](#disclaimer).
- 🌍 **Localization** — Russian, Ukrainian and English.

## Building and running

Requires a recent stable Rust toolchain.

```sh
cd music_player
cargo run --release
```

The local library, playback and lyrics work out of the box. The optional online
search additionally uses `yt-dlp` and `ffmpeg`; if they are not on `PATH`, the
app downloads them into its data folder on first use (Windows builds). See
[`music_player/README.md`](music_player/README.md) for details.

---

## Disclaimer

Elysium is a personal hobby project, provided **as-is** for personal and
educational use. The optional "ЮБ" tab retrieves audio through the third-party
tool [`yt-dlp`]; this project is not affiliated with, endorsed by, or sponsored
by YouTube or Google. You are solely responsible for using it in accordance with
the terms of service of any platform you access and with the copyright laws of
your jurisdiction. The authors accept no liability for misuse. If you are a
rights holder with a concern, please open an issue.

## Licenses

Elysium's own source code is released under the **MIT License** (see
[`LICENSE`](LICENSE)). The bundled Noto fonts are licensed separately under the
**SIL Open Font License 1.1** (see
[`music_player/src/fonts/OFL.txt`](music_player/src/fonts/OFL.txt)).

[`yt-dlp`]: https://github.com/yt-dlp/yt-dlp
