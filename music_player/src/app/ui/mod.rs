//! All on-screen drawing, one module per region.
//!
//! Each submodule adds methods to [`crate::app::App`] (via `impl` blocks) that
//! draw a single panel, modal or overlay. [`crate::app::App::update`] calls
//! them in a fixed order every frame:
//!
//! * [`bottom_bar`]    — transport controls, progress and volume.
//! * [`sidebar`]       — navigation, "New playlist", Liked music, playlist list.
//! * [`central`]       — search row plus the Home / playlist / Liked views.
//! * [`settings`]      — full-screen settings overlay (language + hotkeys).
//! * [`modals`]        — update prompt, "New playlist" and "Rename" dialogs.
//! * [`lyrics`]        — floating lyrics window and the drag-and-drop hint.
//!
//! Drawing methods recompute the localized [`crate::lang::Strings`] table and
//! reuse the shared palette in [`crate::theme`] (`ACCENT`, `TEXT_MUTED`,
//! `BG_MAIN`) so colors stay consistent.

mod bottom_bar;
mod central;
mod home_page;
mod lyrics;
mod modals;
mod now_playing;
mod playlist_page;
mod settings;
mod sidebar;
