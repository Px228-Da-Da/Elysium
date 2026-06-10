# AGENTS.md — architecture & gotchas for AI agents

This file orients an AI agent working on **Elysium** quickly. Read it before
editing. It documents the non-obvious model, invariants and traps that are easy
to break. For a human-facing overview see [`README.md`](README.md); every module
and public item also has rustdoc (`cargo doc --open`).

## What this is

A single-binary desktop music player. Stack: `eframe`/`egui` (immediate-mode UI),
`rodio` (audio), `id3`/`image` (tags + covers), `reqwest`/`ureq` (lyrics + update),
`rdev` (global hotkeys), `serde_json` (config). Edition 2021.

## Module map (where things live)

| Concern | File |
|---|---|
| Startup, `App` struct, frame loop | `src/app/mod.rs` |
| Likes / playlists / deletion / drop import | `src/app/library.rs` |
| Queue, next/prev, transport, hotkey actions | `src/app/playback.rs` |
| Keyboard + global hotkey wiring | `src/app/input.rs` |
| Drawing (one file per UI region) | `src/app/ui/*` |
| Audio engine (play/pause/seek/volume) | `src/player.rs` |
| Folder scan + lyrics (embedded + online) | `src/scanner.rs` |
| Track metadata (title/artist/cover) | `src/meta.rs` |
| Config persistence + legacy migration | `src/config.rs` |
| Languages + string tables | `src/lang.rs` |
| Hotkey enum + bindings + key mapping | `src/shortcuts.rs` |
| Palette + theme + small helpers | `src/theme.rs` |

The UI methods are spread across `src/app/ui/*` as additional `impl App` blocks.
This is legal Rust: those modules are descendants of `app`, so they can access
`App`'s private fields. UI methods use visibility `pub(in crate::app)` so both
`App::update` (in `mod.rs`) and sibling UI modules can call them.

## The frame loop (the heartbeat)

`<App as eframe::App>::update` in `src/app/mod.rs` runs once per frame and is the
single orchestrator. Order matters and is intentional:

1. advance the lyrics clock; 2. drain the lyrics channel; 3. apply theme;
4. drain the loader channel (`LoaderMsg::Playlists` / `Meta`); 5. handle dropped
files; 6. advance playback time + auto-advance at end of track; 7. keyboard +
global hotkeys; 8. draw panels, then overlays (`ui_update_modal`, `ui_bottom_bar`,
`ui_sidebar`, `ui_central`, `ui_settings`, `ui_new_playlist`, `ui_rename_playlist`,
`ui_drop_hint`); 9. schedule the next repaint **only when
there is something to animate** — `request_repaint_after(100ms)` while playing or
while a lyrics fetch is pending; otherwise the UI sleeps until the next input or
background event. Do not add an unconditional repaint here: idle sleeping is the
main CPU saving on weak machines.

**Nothing blocks the UI thread.** All slow work runs on background threads spawned
in `App::new` (hotkey grab, library scan + metadata stream, update check) or
per-action (lyrics fetch in `play_track`, seek in `Player::seek`, metadata for
dropped files). Results return via `mpsc` channels and `ctx.request_repaint()`.

## Invariants — do not break these

- **`LIKED_PLAYLIST_NAME` (`src/app/mod.rs`) is a magic key.** The "Liked music"
  playlist is a normal `Playlist` whose `name` equals this exact string. The same
  string is the on-disk identity. It is **always Russian**, never localized in
  storage; the display layer substitutes `strings(lang).liked_music`. Renaming it
  requires a config migration. Always compare against the constant, never a literal.

- **`LIKED_PAGE_IDX == usize::MAX` is a sentinel page index**, not a real list
  position. `selected_playlist_idx` is: `None` = Home, `Some(LIKED_PAGE_IDX)` =
  Liked music, `Some(i)` = `playlists[i]`. Code that indexes `playlists` must
  exclude or special-case `LIKED_PAGE_IDX` (it uses `.get(idx)` defensively).

- **`playback_queue` is a snapshot.** It is captured (`get_current_queue`) at the
  moment a track starts and is what next/prev walk. Do **not** recompute "what
  plays next" from the currently-open page — switching pages mid-song must not
  change playback order. This is deliberate.

- **Liking inserts at the front and shifts indices.** `toggle_like` may create
  the Liked playlist at index 0, which shifts every other playlist's index by +1.
  It already bumps `selected_playlist_idx` to compensate; preserve that when
  touching it.

- **Likes vs ordinary playlists persist separately.** `save_liked` writes
  `cfg.liked`; `save_playlists` writes `cfg.playlists` (everything *except* Liked
  music). After mutating membership, call the matching one. The track "⋮" delete
  in `playlist_page.rs` picks the right saver based on whether the open page is
  the Liked page.

- **Deleting a playlist keeps files.** `delete_playlist` only removes it from the
  list and records the name in `deleted_playlists` so the next folder scan does
  not resurrect it. It never deletes MP3s or folders.

## Subtle traps

- **Popup vs context-menu dismissal.** In `home_page.rs`, the shared "add to
  playlist" menu body (`draw_add_to_playlist_menu`) must close differently
  depending on the host: the "⋮" popup uses `memory.close_popup()`, the
  right-click `context_menu` uses `ui.close_menu()`. The `via_popup` flag selects
  the correct one — they are not interchangeable.

- **Context-menu first-frame guard.** The manual track "⋮" popup in
  `playlist_page.rs` uses `context_menu_just_opened` to skip the very first
  frame's outside-click check; otherwise the same click that opens it closes it.

- **Seek correctness via operation id.** `Player::seek` has no native seek: it
  decodes/discards samples on a background thread. An `AtomicU64`
  (`current_operation_id`) lets a newer play/seek invalidate an older in-flight
  seek so a stale thread won't resume at the wrong position. `play` also bumps it.

- **Lyrics caching.** `play_track` checks `lyrics_cache` before spawning a fetch;
  successful fetches are cached in `update` step 2. Don't refetch on cache hit.

- **The Home grid and playlist list are virtualized — keep them that way.**
  egui is immediate-mode: drawing all cards/rows every frame is O(library size)
  and dominates CPU on large libraries (thousands of tracks). `home_page.rs`
  draws only the cards intersecting the viewport (`ScrollArea::show_viewport`
  with manual row math) and caches the deduped/filtered list in `App::home_cache`
  (rebuilt only when `home_cache_sig` changes). `playlist_page.rs` uses
  `ScrollArea::show_rows`. Do **not** revert to a plain `for song in … { draw }`
  loop over the whole list, and keep `draw_home_card` taking an explicit `rect`
  (it is positioned manually, not via `allocate_exact_size`).

- **`MUSIC_ROOT` is relative** (`../DownloadedMusic`), resolved against the
  working directory. Tests/launches from a different CWD will scan a different
  place.

- **Fonts must be installed before `App::new`** (see `main.rs`); the bundled Noto
  fonts provide Cyrillic + emoji coverage. Many "icons" (dots, checkmarks) are
  drawn with primitives rather than glyphs specifically to avoid missing-glyph
  boxes — keep that approach when adding similar affordances.

## Conventions

- Comments and rustdoc are in **English**. Keep them accurate when changing code.
- Reuse the palette in `src/theme.rs` (`ACCENT`, `TEXT_MUTED`, `BG_MAIN`) instead
  of hardcoding those colors.
- UI drawing methods recompute `strings(self.language)` locally rather than
  threading it through — match that pattern.

## Verify after changes

```sh
cargo check       # type/borrow correctness
cargo clippy      # currently clean — keep it that way
cargo run --release
```

There is no automated test suite; the app is GUI-driven, so verify behavior by
running it. The codebase compiles warning-free — treat new warnings as regressions.
