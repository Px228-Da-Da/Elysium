//! Full "Now Playing" view: a large cover on the left, synced lyrics on the
//! right, over a background color derived from the cover art.
//!
//! It replaces the central panel while [`App::show_now_playing`] is set (the
//! sidebar and bottom bar stay visible). Opened by clicking the bottom-bar
//! cover; closed by the chevron in its top-right corner.

use crate::app::App;
use crate::lang::strings;
use eframe::egui;
use egui::{pos2, vec2, Align, Color32, FontId, Layout, Rect, RichText, Rounding, Stroke};

/// Fallback background when the current track has no cover (the olive tone from
/// the reference design).
const FALLBACK_BG: Color32 = Color32::from_rgb(73, 73, 30);

impl App {
    /// Draws the Now Playing view into `ui` (the central panel area).
    pub(in crate::app) fn ui_now_playing(&mut self, ui: &mut egui::Ui) {
        // Read what we need up front so no borrow on `self` is held while drawing.
        let (bg, cover_id) = {
            let meta = self.track_meta.get(&self.current_song);
            let bg = meta.and_then(|m| m.bg).unwrap_or(FALLBACK_BG);
            let cover_id = meta.and_then(|m| m.cover.as_ref()).map(|t| t.id());
            (bg, cover_id)
        };
        let lyrics = self.current_lyrics.clone();
        let time_ms = self.current_playback_time_ms;
        // Whether a background lyrics fetch is still in progress (so we can tell
        // "searching" apart from "genuinely not found").
        let searching = self.lyrics_receiver.is_some();
        let s = strings(self.language);

        let area = ui.max_rect();
        ui.painter().rect_filled(area, Rounding::ZERO, bg);

        // --- Close chevron (top-right) ---
        let chevron_center = pos2(area.right() - 40.0, area.top() + 40.0);
        let chevron_rect = Rect::from_center_size(chevron_center, vec2(40.0, 40.0));
        let chevron = ui.interact(chevron_rect, ui.id().with("now_playing_close"), egui::Sense::click());
        if chevron.hovered() {
            ui.painter().circle_filled(chevron_center, 18.0, Color32::from_black_alpha(60));
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        // Draw the downward chevron with two line segments (font-independent).
        let stroke = Stroke::new(2.5, Color32::WHITE);
        let c = chevron_center;
        ui.painter().line_segment([pos2(c.x - 7.0, c.y - 3.0), pos2(c.x, c.y + 4.0)], stroke);
        ui.painter().line_segment([pos2(c.x, c.y + 4.0), pos2(c.x + 7.0, c.y - 3.0)], stroke);
        if chevron.clicked() {
            self.show_now_playing = false;
        }

        // --- Cover (left) ---
        let cover_size = (area.height() * 0.5).clamp(180.0, 380.0);
        let cover_center = pos2(area.left() + area.width() * 0.28, area.center().y);
        let cover_rect = Rect::from_center_size(cover_center, vec2(cover_size, cover_size));
        ui.painter().rect_filled(cover_rect, Rounding::same(10.0), Color32::from_black_alpha(60));
        match cover_id {
            Some(id) => {
                ui.painter().image(
                    id,
                    cover_rect,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            None => {
                ui.painter().text(
                    cover_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "🎵",
                    FontId::proportional(80.0),
                    Color32::from_white_alpha(120),
                );
            }
        }

        // --- Lyrics (right) ---
        let lyrics_rect = Rect::from_min_max(
            pos2(area.left() + area.width() * 0.5, area.top() + 70.0),
            pos2(area.right() - 40.0, area.bottom() - 30.0),
        );
        let mut lyr_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(lyrics_rect)
                .layout(Layout::top_down(Align::Min)),
        );
        lyr_ui.set_clip_rect(lyrics_rect);

        let Some(lyrics) = lyrics else {
            // No lyrics: tell the user we are still looking, or that none were
            // found, instead of showing bare dots.
            let message = if searching { s.lyrics_searching } else { s.lyrics_not_found };
            lyr_ui.add_space(lyrics_rect.height() / 2.0 - 20.0);
            lyr_ui.label(RichText::new(message).size(22.0).color(Color32::from_white_alpha(150)));
            return;
        };

        // The active line is the last one whose timestamp has passed.
        let active_idx = lyrics
            .iter()
            .rposition(|line| time_ms >= line.time_ms)
            .map(|i| i as i32)
            .unwrap_or(-1);

        egui::ScrollArea::vertical()
            .id_salt("now_playing_lyrics")
            .auto_shrink([false, false])
            .show(&mut lyr_ui, |ui| {
                // Pad so the first/last lines can scroll to the vertical center.
                ui.add_space(lyrics_rect.height() / 2.0);
                for (i, line) in lyrics.iter().enumerate() {
                    let is_active = i as i32 == active_idx;
                    let (color, size) = if is_active {
                        (Color32::WHITE, 30.0)
                    } else {
                        // Fade lines by their distance from the active one.
                        let dist = (i as i32 - active_idx).unsigned_abs();
                        let alpha = 190u8.saturating_sub((dist * 35).min(150) as u8).max(40);
                        (Color32::from_white_alpha(alpha), 22.0)
                    };
                    let resp = ui.add(
                        egui::Label::new(RichText::new(&line.text).size(size).strong().color(color)),
                    );
                    // Keep the active line centered (karaoke-style auto-follow).
                    if is_active {
                        resp.scroll_to_me(Some(Align::Center));
                    }
                    ui.add_space(10.0);
                }
                ui.add_space(lyrics_rect.height() / 2.0);
            });
    }
}
