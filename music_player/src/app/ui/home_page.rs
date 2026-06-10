//! Home page: the "Listen again" grid of track cards.
//!
//! Cards are de-duplicated across playlists, optionally filtered by the search
//! query, and form the playback queue when one is played. Each card has a hover
//! play button, a ❤ like toggle, and a "⋮" / right-click menu for adding the
//! track to a playlist.

use crate::app::{App, LIKED_PLAYLIST_NAME};
use crate::lang::strings;
use crate::theme::{style_menu, ACCENT, TEXT_MUTED};
use eframe::egui;
use egui::{pos2, vec2, Color32, FontId, Rect, RichText, Rounding, Stroke, Vec2};
use std::collections::HashSet;

/// Card geometry (logical pixels). Kept as constants because the virtualized
/// grid needs them up front to compute which rows are visible.
const CARD_W: f32 = 160.0;
const CARD_H: f32 = 240.0;
const GAP_X: f32 = 18.0;
const GAP_Y: f32 = 24.0;

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
                    if !seen_songs.insert(song.clone()) {
                        return false; // already shown via another playlist
                    }
                    if query.is_empty() {
                        return true;
                    }
                    let meta = self.track_meta.get(song);
                    let title = meta.map(|m| m.title.to_lowercase()).unwrap_or_default();
                    let artist =
                        meta.and_then(|m| m.artist.clone()).unwrap_or_default().to_lowercase();
                    title.contains(&query) || artist.contains(&query)
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

        // Fixed heading above the scroll area.
        ui.label(RichText::new(s.listen_again).size(26.0).strong().color(Color32::WHITE));
        ui.add_space(20.0);

        // Refresh (or reuse) the cached list.
        self.home_track_list();
        if self.home_cache.is_empty() {
            return;
        }

        let row_h = CARD_H + GAP_Y;
        let mut to_play: Option<String> = None;

        egui::ScrollArea::vertical()
            .id_salt("main_page_vertical_scroll")
            .auto_shrink([false, false])
            .show_viewport(ui, |ui, viewport| {
                let n = self.home_cache.len();
                // Columns that fit the actual content width; rows follow.
                let avail_w = ui.available_width();
                let cols = (((avail_w + GAP_X) / (CARD_W + GAP_X)).floor() as usize).max(1);
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
                        let x = origin.x + col as f32 * (CARD_W + GAP_X);
                        let y = origin.y + row as f32 * row_h;
                        let rect = Rect::from_min_size(pos2(x, y), vec2(CARD_W, CARD_H));

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

        let bg_color = if is_hovered { Color32::from_rgb(40, 40, 40) } else { Color32::from_rgb(24, 24, 24) };
        ui.painter().rect_filled(rect, Rounding::same(8.0), bg_color);

        // Cover.
        let cover_size = 132.0;
        let cover_pos = rect.min + Vec2::new(14.0, 14.0);
        let cover_rect = Rect::from_min_size(cover_pos, Vec2::new(cover_size, cover_size));
        ui.painter().rect_filled(cover_rect, Rounding::same(6.0), Color32::from_rgb(50, 50, 50));
        if let Some(tex) = meta.and_then(|m| m.cover.as_ref()) {
            ui.painter().image(
                tex.id(),
                cover_rect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        } else {
            ui.painter().text(cover_rect.center(), egui::Align2::CENTER_CENTER, "🎵", FontId::proportional(40.0), Color32::from_rgb(90, 90, 90));
        }

        // Title + artist below the cover (truncated to the card width).
        let text_pos = cover_rect.left_bottom() + Vec2::new(0.0, 12.0);
        let text_color = if is_active { ACCENT } else { Color32::WHITE };
        let max_chars_title = 15;
        let max_chars_artist = 18;

        let title = meta.map(|m| m.title.clone()).unwrap_or_else(|| s.unknown_title.to_string());
        let display_name = if title.chars().count() > max_chars_title {
            format!("{}...", title.chars().take(max_chars_title - 3).collect::<String>())
        } else {
            title
        };
        ui.painter().text(text_pos, egui::Align2::LEFT_TOP, display_name, FontId::proportional(14.0), text_color);

        let artist = meta.and_then(|m| m.artist.clone()).unwrap_or_else(|| s.unknown_artist.to_string());
        let subtitle = if artist.chars().count() > max_chars_artist {
            format!("{}...", artist.chars().take(max_chars_artist - 3).collect::<String>())
        } else {
            artist
        };
        let subtext_pos = text_pos + Vec2::new(0.0, 18.0);
        ui.painter().text(subtext_pos, egui::Align2::LEFT_TOP, subtitle, FontId::proportional(12.0), TEXT_MUTED);

        // ❤ Like toggle.
        let liked = self.is_liked(song);
        let heart_color = if liked { ACCENT } else { Color32::from_rgb(100, 100, 100) };
        let heart_rect = Rect::from_min_size(pos2(rect.right() - 36.0, rect.bottom() - 36.0), vec2(28.0, 28.0));
        let mut heart_ui = ui.new_child(egui::UiBuilder::new().max_rect(heart_rect));
        let heart_click = heart_ui.add(
            egui::Button::new(RichText::new("❤").size(16.0).color(heart_color))
                .fill(Color32::TRANSPARENT)
                .frame(false),
        );
        if heart_click.clicked() {
            self.toggle_like(song);
        }

        // ⋮ Three vertical dots in the cover's top-right corner. The clickable
        // zone is invisible (no background/border) so it does not look like a
        // button; the dots are painted manually to avoid missing-glyph boxes.
        let dots_rect = Rect::from_min_size(pos2(rect.right() - 34.0, rect.min.y + 10.0), vec2(28.0, 28.0));
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
            ui.painter().circle_filled(btn_center, btn_radius, ACCENT);
            let icon = if is_active && self.is_playing { "⏸" } else { "▶" };
            ui.painter().text(btn_center, egui::Align2::CENTER_CENTER, icon, FontId::proportional(20.0), Color32::BLACK);
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
            let text_color = if already_in { ACCENT } else { Color32::WHITE };

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
                let stroke = Stroke::new(2.0, ACCENT);
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
