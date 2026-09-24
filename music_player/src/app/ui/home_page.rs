//! Home page: the "Listen again" grid of track cards.
//!
//! Cards are de-duplicated across playlists, optionally filtered by the search
//! query, and form the playback queue when one is played. Each card has a hover
//! play button, a ❤ like toggle, and a "⋮" / right-click menu for adding the
//! track to a playlist.

use crate::app::{App, LIKED_PLAYLIST_NAME};
use crate::lang::{strings, Lang};
use crate::theme::{
    accent, cover_label, gen_glyph, gen_gradient, gradient_rrect, line, style_menu, surface, surface_2, text, text_muted,
};
use eframe::egui;
use egui::{pos2, vec2, Align2, Color32, FontId, Rect, RichText, Rounding, Stroke, Vec2};
use std::collections::HashSet;

/// Card geometry (logical pixels). The grid shows between `MIN_COLS` and
/// `MAX_COLS` cards per row: a wide window fits `MAX_COLS`, and as it shrinks the
/// column count steps down (never below `MIN_COLS`, after which the cards simply
/// get smaller). Cards are always stretched to divide the row width exactly, so
/// there is never an empty gutter on the right. The card height follows the
/// (square) cover width plus a fixed footer (title + artist + padding).
const MIN_COLS: usize = 3;
const MAX_COLS: usize = 5;
/// Nominal card width used to decide how many columns fit before stepping down.
/// Kept small so a maximized window reaches `MAX_COLS` even under OS display
/// scaling (which narrows the logical width).
const NOMINAL_CARD_W: f32 = 175.0;
/// Height added below the square cover for the title + artist + bottom padding.
const CARD_FOOTER: f32 = 54.0;
const GAP_X: f32 = 24.0;
const GAP_Y: f32 = 28.0;
/// Corner radius of a cover, relative to its width.
const COVER_ROUND: f32 = 0.07;

impl App {
    /// Returns the cached Home track list, rebuilding it only when its inputs
    /// changed (playlists, loaded metadata or the search query).
    ///
    /// The list is deduplicated across playlists and filtered by the search
    /// query; Liked music is excluded because its tracks already appear via
    /// their source playlists. For a large library this avoids rebuilding a
    /// multi-thousand-entry list on every frame.
    fn home_track_list(&mut self) -> &[String] {
        let total_songs: usize = self
            .playlists
            .iter()
            .filter(|p| p.name != LIKED_PLAYLIST_NAME)
            .map(|p| p.songs.len())
            .sum();
        // Loaded-metadata count only affects the result while searching (the
        // filter reads titles/artists). When not searching it is held at 0 so
        // streaming covers in at startup does not trigger needless rebuilds.
        let meta_sig = if self.search_query.is_empty() {
            0
        } else {
            self.track_meta.len()
        };
        let sig = (
            self.playlists.len(),
            total_songs,
            meta_sig,
            self.search_query.clone(),
        );

        if sig != self.home_cache_sig {
            let query = self.search_query.to_lowercase();
            let mut seen_songs = HashSet::new();
            self.home_cache = self
                .playlists
                .iter()
                .filter(|p| p.name != LIKED_PLAYLIST_NAME)
                .flat_map(|p| p.songs.iter().cloned())
                .filter(|song| {
                    // De-duplicate by file name (case-insensitive), so the *same*
                    // track sitting in several playlists/folders — with different
                    // paths — is shown only once.
                    let dedup_key = std::path::Path::new(song)
                        .file_name()
                        .map(|s| s.to_string_lossy().to_lowercase())
                        .unwrap_or_else(|| song.to_lowercase());
                    if !seen_songs.insert(dedup_key) {
                        return false; // already shown (same file elsewhere)
                    }
                    if query.is_empty() {
                        return true;
                    }
                    let meta = self.track_meta.get(song);
                    let title = meta.map(|m| m.title.to_lowercase()).unwrap_or_default();
                    let artist =
                        meta.and_then(|m| m.artist.clone()).unwrap_or_default().to_lowercase();
                    // Always match the file name too, so a track is findable even
                    // before its (streamed) metadata has loaded.
                    let file_name = std::path::Path::new(song)
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_lowercase())
                        .unwrap_or_default();
                    title.contains(&query)
                        || artist.contains(&query)
                        || file_name.contains(&query)
                })
                .collect();
            self.home_cache_sig = sig;
        }

        &self.home_cache
    }

    /// Draws the Home page card grid.
    ///
    /// The grid is **virtualized**: only the cards inside the visible viewport
    /// are built and drawn. With thousands of tracks this is the difference
    /// between processing ~30 widgets per frame and processing all of them.
    pub(in crate::app) fn ui_home_page(&mut self, ui: &mut egui::Ui) {
        let s = strings(self.language);

        // Small gap below the search bar, then the heading row.
        ui.add_space(2.0);
        // Use the current cursor (below the search row), not the panel top.
        let top = ui.cursor().min;
        let full_w = ui.available_width();
        // Single-line heading, as in the mockup.
        ui.painter().text(
            pos2(top.x, top.y + 4.0),
            Align2::LEFT_TOP,
            s.listen_again,
            FontId::proportional(34.0),
            text(),
        );
        ui.painter().text(
            pos2(top.x + full_w, top.y + 16.0),
            Align2::RIGHT_TOP,
            all_history(self.language),
            FontId::proportional(13.0),
            text_muted(),
        );
        // Reserve the single-line heading (~44px tall) plus a small gap before
        // the card grid.
        ui.add_space(56.0);

        // Refresh (or reuse) the cached list.
        self.home_track_list();
        if self.home_cache.is_empty() {
            return;
        }

        let mut to_play: Option<String> = None;

        egui::ScrollArea::vertical()
            .id_salt("main_page_vertical_scroll")
            .auto_shrink([false, false])
            .show_viewport(ui, |ui, viewport| {
                let n = self.home_cache.len();
                let avail_w = ui.available_width();
                // How many nominal-width cards fit, clamped to [MIN_COLS, MAX_COLS]:
                // 5 on a wide window, stepping down to 3 as it narrows. The cards
                // are then stretched to divide the row width exactly (no gutter).
                let fit = ((avail_w + GAP_X) / (NOMINAL_CARD_W + GAP_X)).floor() as usize;
                let cols = fit.clamp(MIN_COLS, MAX_COLS);
                let card_w = (avail_w - (cols as f32 - 1.0) * GAP_X) / cols as f32;
                let card_h = card_w + CARD_FOOTER;
                let row_h = card_h + GAP_Y;

                let rows = n.div_ceil(cols);
                let total_h = rows as f32 * row_h;

                // Reserve the full virtual height so the scrollbar is correct;
                // `origin` is the top-left of the (scrolling) content.
                let (reserved, _) = ui.allocate_exact_size(vec2(avail_w, total_h), egui::Sense::hover());
                let origin = reserved.min;

                // Only the rows intersecting the viewport need to be drawn.
                let first_row = (viewport.min.y / row_h).floor().max(0.0) as usize;
                let last_row = ((viewport.max.y / row_h).ceil() as usize).min(rows);

                for row in first_row..last_row {
                    for col in 0..cols {
                        let idx = row * cols + col;
                        if idx >= n {
                            break;
                        }
                        let x = origin.x + col as f32 * (card_w + GAP_X);
                        let y = origin.y + row as f32 * row_h;
                        let rect = Rect::from_min_size(pos2(x, y), vec2(card_w, card_h));

                        // Clone just this one (visible) song so the `self` borrow
                        // is free for the &mut call below.
                        let song = self.home_cache[idx].clone();
                        if self.draw_home_card(ui, rect, &song, &s) {
                            to_play = Some(song);
                        }
                    }
                }
            });

        // Start a new track outside the draw loop: install the full Home list as
        // the playback queue (so next/prev walk it), then play.
        if let Some(song) = to_play {
            self.playback_queue = self.home_cache.clone();
            self.play_track(&song);
        }
    }

    /// Draws a single Home card for `song` at the given `rect` and handles its
    /// interactions. Returns `true` when the card was clicked to start playing a
    /// new track (the caller installs the queue and starts playback); play/pause
    /// of the already-active track is handled inline.
    fn draw_home_card(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        song: &str,
        s: &crate::lang::Strings,
    ) -> bool {
        let meta = self.track_meta.get(song);
        let is_active = self.current_song == *song;

        let response = ui.interact(rect, ui.make_persistent_id(("home_card", song)), egui::Sense::click());
        let is_hovered = response.hovered();

        // Card panel — a subtle bordered rounded surface that lifts on hover.
        let dy = ui.ctx().animate_bool_with_time(ui.make_persistent_id(("card_lift", song)), is_hovered, 0.13) * 4.0;
        let rect = rect.translate(vec2(0.0, -dy));
        let panel_fill = if is_hovered { surface_2() } else { surface() };
        ui.painter().rect(rect, Rounding::same(14.0), panel_fill, Stroke::new(1.0, line()));

        // Cover: inset within the panel (padding all around), square and rounded.
        let pad = 12.0;
        let cover_side = rect.width() - 2.0 * pad;
        let cover_rect = Rect::from_min_size(rect.min + vec2(pad, pad), vec2(cover_side, cover_side));
        let title = meta
            .map(|m| m.title.clone())
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| {
                std::path::Path::new(song)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| s.unknown_title.to_string())
            });
        draw_cover(ui, cover_rect, song, meta, &title);

        // Title + artist below the cover (inside the panel padding). Font sizes
        // scale with the card width, and each line is truncated to the *measured*
        // pixel width (leaving room for the heart on the right) so nothing spills
        // out of the card when the window (and cards) get small.
        let text_left = rect.left() + pad;
        let ty = cover_rect.bottom() + 14.0;
        let text_color = if is_active { accent() } else { text() };
        let title_font = (rect.width() * 0.068).clamp(11.0, 16.0);
        let artist_font = (rect.width() * 0.055).clamp(9.0, 13.0);
        // Width available for text: card minus left pad and the heart's column.
        let text_w = (rect.width() - pad - 34.0).max(24.0);
        let title_galley = fit_line(ui, &title, FontId::proportional(title_font), text_color, text_w);
        ui.painter().galley(pos2(text_left, ty), title_galley, text_color);

        let artist = meta.and_then(|m| m.artist.clone()).unwrap_or_else(|| s.unknown_artist.to_string());
        let artist_galley = fit_line(ui, &artist, FontId::proportional(artist_font), text_muted(), text_w);
        ui.painter().galley(pos2(text_left, ty + 24.0), artist_galley, text_muted());

        // ❤ Like toggle (right of the title/artist).
        let liked = self.is_liked(song);
        let heart_color = if liked { accent() } else { Color32::from_rgb(120, 120, 128) };
        let heart_rect = Rect::from_center_size(pos2(rect.right() - pad - 2.0, ty + 18.0), vec2(28.0, 28.0));
        let heart_click = ui.interact(heart_rect, ui.make_persistent_id(("home_heart", song)), egui::Sense::click());
        crate::icons::paint(ui, heart_rect.shrink(6.0), crate::icons::Icon::Heart, heart_color);
        if heart_click.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if heart_click.clicked() {
            self.toggle_like(song);
        }

        // ⋮ Three vertical dots in the cover's top-right corner. The clickable
        // zone is invisible (no background/border) so it does not look like a
        // button; the dots are painted manually to avoid missing-glyph boxes.
        let dots_rect = Rect::from_min_size(pos2(cover_rect.right() - 30.0, cover_rect.top() + 8.0), vec2(28.0, 28.0));
        let dots_id = ui.make_persistent_id(("dots_btn", song));
        let dots_resp = ui.interact(dots_rect, dots_id, egui::Sense::click());

        if dots_resp.hovered() {
            ui.painter().rect_filled(dots_rect, Rounding::same(6.0), Color32::from_black_alpha(70));
        }

        let dot_color = if dots_resp.hovered() { Color32::WHITE } else { Color32::from_gray(210) };
        let dot_shadow = Color32::from_black_alpha(130);
        let dc = dots_rect.center();
        let dot_r = 2.0;
        let dot_gap = 6.0;
        for dy in [-dot_gap, 0.0, dot_gap] {
            let p = pos2(dc.x, dc.y + dy);
            // Shadow so the dots read over a bright cover.
            ui.painter().circle_filled(p + vec2(0.0, 1.0), dot_r + 0.4, dot_shadow);
            ui.painter().circle_filled(p, dot_r, dot_color);
        }

        // Click on the dots toggles the "add to playlist" popup.
        let dots_popup_id = ui.make_persistent_id(("dots_popup", song));
        if dots_resp.clicked() {
            ui.ctx().memory_mut(|mem| mem.toggle_popup(dots_popup_id));
        }

        let mut dots_clicked = false;
        egui::popup::popup_below_widget(
            ui,
            dots_popup_id,
            &dots_resp,
            egui::popup::PopupCloseBehavior::CloseOnClickOutside,
            |ui| {
                self.draw_add_to_playlist_menu(ui, song, &mut dots_clicked, true);
            },
        );

        // Right-click on the card opens the same "add to playlist" menu.
        response.context_menu(|ui| {
            let mut ignored = false;
            self.draw_add_to_playlist_menu(ui, song, &mut ignored, false);
        });

        // Large green play button on hover / while active.
        if (is_hovered || is_active) && !dots_resp.hovered() {
            let btn_radius = 22.0;
            let btn_center = cover_rect.max - Vec2::new(btn_radius + 4.0, btn_radius + 4.0);
            ui.painter().circle_filled(btn_center + Vec2::new(0.0, 2.0), btn_radius, Color32::from_black_alpha(100));
            ui.painter().circle_filled(btn_center, btn_radius, accent());
            let icon = if is_active && self.is_playing { crate::icons::Icon::Pause } else { crate::icons::Icon::Play };
            crate::icons::paint_at(ui, btn_center, 22.0, icon, Color32::BLACK);
        }

        // Card click plays the track, but only if no control/menu was clicked.
        if response.clicked() && !heart_click.clicked() && !dots_resp.clicked() && !dots_clicked {
            if is_active {
                // Toggle the already-playing track in place (no queue needed).
                if self.is_playing {
                    self.player.pause();
                    self.is_playing = false;
                } else {
                    self.player.resume();
                    self.is_playing = true;
                }
            } else {
                // Signal the caller to start this track with the Home queue.
                return true;
            }
        }
        false
    }

    /// Draws the shared "add this track to a playlist" menu body (used by both
    /// the "⋮" popup and the right-click context menu). Sets `clicked` to `true`
    /// when an item is chosen. A green check is drawn beside playlists that
    /// already contain the track.
    ///
    /// `via_popup` selects the correct dismissal: the "⋮" popup closes via
    /// `close_popup`, while the right-click context menu closes via `close_menu`.
    fn draw_add_to_playlist_menu(
        &mut self,
        ui: &mut egui::Ui,
        song: &str,
        clicked: &mut bool,
        via_popup: bool,
    ) {
        style_menu(ui);
        ui.set_min_width(210.0);
        for p_idx in 0..self.playlists.len() {
            if self.playlists[p_idx].name == LIKED_PLAYLIST_NAME {
                continue;
            }
            let p_name = self.playlists[p_idx].name.clone();
            let already_in = self.playlists[p_idx].songs.iter().any(|s| s == song);
            let text_color = if already_in { accent() } else { text() };

            let btn = ui.add(
                egui::Button::new(
                    RichText::new(format!("      {}", p_name)) // leading space leaves room for the check
                        .size(14.0)
                        .color(text_color),
                )
                .min_size(vec2(202.0, 32.0))
                .rounding(6.0),
            );

            // Draw the check mark with line segments (font-independent).
            if already_in {
                let r = btn.rect;
                let cx = r.left() + 15.0;
                let cy = r.center().y;
                let stroke = Stroke::new(2.0, accent());
                ui.painter().line_segment([pos2(cx - 5.0, cy + 1.0), pos2(cx - 1.0, cy + 5.0)], stroke);
                ui.painter().line_segment([pos2(cx - 1.0, cy + 5.0), pos2(cx + 6.0, cy - 5.0)], stroke);
            }

            if btn.clicked() {
                *clicked = true;
                if !already_in {
                    self.playlists[p_idx].songs.push(song.to_string());
                    self.save_playlists();
                }
                if via_popup {
                    ui.ctx().memory_mut(|m| m.close_popup());
                } else {
                    ui.close_menu();
                }
            }
        }
    }
}

/// Draws a track cover into `rect`. When the file embeds album art, the real
/// cover image is shown (rounded to the card shape); otherwise a generated
/// gradient with the title's leading characters is drawn as a fallback.
fn draw_cover(ui: &mut egui::Ui, rect: Rect, song: &str, meta: Option<&crate::meta::TrackMeta>, title: &str) {
    let round = rect.width() * COVER_ROUND;

    // Real embedded cover art, if the loader has decoded one for this track.
    if let Some(tex) = meta.and_then(|m| m.cover.as_ref()) {
        egui::Image::new((tex.id(), rect.size()))
            .rounding(Rounding::same(round))
            .paint_at(ui, rect);
        return;
    }

    // Fallback: a generated gradient with the title's leading glyph.
    let (c1, c2) = gen_gradient(song);
    gradient_rrect(ui.painter(), rect, round, c1, c2);
    let label = cover_label(title);
    if !label.is_empty() {
        // Bright colored glyph (a light tint of the cover's hue), like the mockup.
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            &label,
            FontId::proportional(rect.width() * 0.33),
            gen_glyph(song),
        );
    }
}

/// Lays out `text` on a single line in `font`/`color`, truncated with an
/// ellipsis to fit within `max_w` pixels (measuring the *actual* glyph widths,
/// so wide Cyrillic caps never spill out of the card). Returns a ready-to-paint
/// galley for `Painter::galley`.
fn fit_line(
    ui: &egui::Ui,
    text: &str,
    font: FontId,
    color: Color32,
    max_w: f32,
) -> std::sync::Arc<egui::Galley> {
    let make = |s: String| ui.fonts(|f| f.layout_no_wrap(s, font.clone(), color));
    let full = make(text.to_string());
    if full.rect.width() <= max_w {
        return full;
    }
    // Binary-search the longest prefix that still fits once an ellipsis is added.
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        let candidate: String = chars[..mid].iter().collect::<String>() + "…";
        if make(candidate).rect.width() <= max_w {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    let candidate: String = chars[..lo].iter().collect::<String>() + "…";
    make(candidate)
}

/// Localized "all history" link shown at the top-right of the Home page.
fn all_history(lang: Lang) -> &'static str {
    match lang {
        Lang::Ru => "ВСЯ ИСТОРИЯ  →",
        Lang::Uk => "УСЯ ІСТОРІЯ  →",
        Lang::En => "ALL HISTORY  →",
    }
}
