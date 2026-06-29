//! The "ЮБ" tab: an ad-free YouTube Music search.
//!
//! A search box queries YouTube through `yt-dlp` (see [`crate::ytdl`]); results
//! are drawn as cover-art cards (like the Home grid). Clicking one streams its
//! audio — no web player, so there are no ads of any kind.

use crate::app::App;
use crate::lang::strings;
use crate::theme::{ACCENT, TEXT_MUTED};
use eframe::egui;
use egui::{pos2, vec2, Color32, FontId, Rect, RichText, Rounding, Vec2};

/// Card geometry (logical pixels), matching the Home grid.
const CARD_W: f32 = 160.0;
const CARD_H: f32 = 232.0;
const GAP_X: f32 = 18.0;
const GAP_Y: f32 = 24.0;

/// Truncates a string to at most `max` characters, adding an ellipsis.
fn clip(text: &str, max: usize) -> String {
    if text.chars().count() > max {
        format!("{}…", text.chars().take(max.saturating_sub(1)).collect::<String>())
    } else {
        text.to_string()
    }
}

impl App {
    /// Draws the YouTube search page.
    pub(in crate::app) fn ui_youtube_page(&mut self, ui: &mut egui::Ui) {
        let s = strings(self.language);
        let ctx = ui.ctx().clone();

        // --- Search row: rounded field + accent button. ---
        let mut do_search = false;
        ui.horizontal(|ui| {
            let box_rect = ui.allocate_exact_size(vec2(420.0, 36.0), egui::Sense::hover()).0;
            ui.painter()
                .rect_filled(box_rect, Rounding::same(18.0), Color32::from_rgb(30, 30, 30));

            let mut inner = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(box_rect.shrink(8.0))
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            inner.add_space(8.0);
            inner.label(RichText::new("🔍").size(14.0).color(TEXT_MUTED));
            let response = inner.add(
                egui::TextEdit::singleline(&mut self.yt_query)
                    .frame(false)
                    .hint_text(RichText::new(s.yt_search_hint).color(TEXT_MUTED))
                    .text_color(Color32::WHITE)
                    .desired_width(330.0),
            );
            if response.lost_focus() && inner.input(|i| i.key_pressed(egui::Key::Enter)) {
                do_search = true;
            }

            ui.add_space(10.0);
            let btn = ui.add(
                egui::Button::new(RichText::new(s.yt_search_btn).size(14.0).color(Color32::WHITE))
                    .fill(ACCENT)
                    .rounding(18.0),
            );
            if btn.clicked() {
                do_search = true;
            }
        });
        if do_search {
            self.start_yt_search(&ctx);
        }

        ui.add_space(12.0);

        // --- Status / error line. ---
        let status = self.yt_status.lock().ok().map(|g| g.clone()).unwrap_or_default();
        if self.yt_searching {
            let line = if status.is_empty() { s.yt_searching.to_string() } else { status };
            ui.label(RichText::new(line).color(TEXT_MUTED));
        } else if !status.is_empty() {
            ui.label(RichText::new(status).color(TEXT_MUTED));
        }
        if let Some(err) = &self.yt_error {
            ui.label(RichText::new(err).color(Color32::from_rgb(230, 90, 90)));
        }

        // --- Empty state: hint before the first search, "nothing found" after. ---
        if self.yt_results.is_empty() && !self.yt_searching && self.yt_error.is_none() {
            let msg = if self.yt_searched { s.yt_no_results } else { s.yt_empty };
            ui.add_space(8.0);
            ui.label(RichText::new(msg).color(TEXT_MUTED));
            return;
        }

        // --- Results: cover-art card grid (like the Home page). ---
        ui.add_space(8.0);
        let mut to_play: Option<crate::ytdl::YtTrack> = None;
        let row_h = CARD_H + GAP_Y;
        egui::ScrollArea::vertical()
            .id_salt("youtube_results_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let avail_w = ui.available_width();
                let cols = (((avail_w + GAP_X) / (CARD_W + GAP_X)).floor() as usize).max(1);
                let n = self.yt_results.len();
                let rows = n.div_ceil(cols);

                let (reserved, _) =
                    ui.allocate_exact_size(vec2(avail_w, rows as f32 * row_h), egui::Sense::hover());
                let origin = reserved.min;

                for i in 0..n {
                    let (row, col) = (i / cols, i % cols);
                    let x = origin.x + col as f32 * (CARD_W + GAP_X);
                    let y = origin.y + row as f32 * row_h;
                    let rect = Rect::from_min_size(pos2(x, y), vec2(CARD_W, CARD_H));

                    let track = &self.yt_results[i];
                    let loading =
                        self.yt_loading_track.as_ref().map(|t| t.id.as_str()) == Some(track.id.as_str());
                    if self.draw_yt_card(ui, rect, track, loading, &s) {
                        to_play = Some(track.clone());
                    }
                }
            });

        if let Some(track) = to_play {
            self.start_yt_play(&ctx, track);
        }
    }

    /// Draws one result card (cover, title, artist) and returns `true` when it
    /// was clicked to start playing (ignored while that track is loading).
    fn draw_yt_card(
        &self,
        ui: &mut egui::Ui,
        rect: Rect,
        track: &crate::ytdl::YtTrack,
        loading: bool,
        s: &crate::lang::Strings,
    ) -> bool {
        let is_active = self.current_song == format!("yt:{}", track.id);

        let response =
            ui.interact(rect, ui.make_persistent_id(("yt_card", &track.id)), egui::Sense::click());
        let hovered = response.hovered();

        let bg = if hovered { Color32::from_rgb(40, 40, 40) } else { Color32::from_rgb(24, 24, 24) };
        ui.painter().rect_filled(rect, Rounding::same(8.0), bg);

        // Cover (132×132), filled with the thumbnail center-cropped to a square.
        let cover_size = 132.0;
        let cover_rect = Rect::from_min_size(rect.min + Vec2::new(14.0, 14.0), Vec2::splat(cover_size));
        ui.painter().rect_filled(cover_rect, Rounding::same(6.0), Color32::from_rgb(50, 50, 50));

        let tex = self
            .yt_thumbs
            .lock()
            .ok()
            .and_then(|t| t.get(&track.id).map(|(tex, _)| tex.clone()));
        if let Some(tex) = tex {
            // Thumbnail is already cropped to a square, so draw it whole.
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            ui.painter().image(tex.id(), cover_rect, uv, Color32::WHITE);
        } else {
            ui.painter().text(
                cover_rect.center(),
                egui::Align2::CENTER_CENTER,
                "🎵",
                FontId::proportional(40.0),
                Color32::from_rgb(90, 90, 90),
            );
        }

        // Hover/loading play overlay on the cover.
        if hovered || loading {
            ui.painter()
                .rect_filled(cover_rect, Rounding::same(6.0), Color32::from_black_alpha(110));
            let icon = if loading { "⏳" } else { "▶" };
            ui.painter().text(
                cover_rect.center(),
                egui::Align2::CENTER_CENTER,
                icon,
                FontId::proportional(34.0),
                Color32::WHITE,
            );
        }

        // Title + artist below the cover.
        let text_pos = cover_rect.left_bottom() + Vec2::new(0.0, 12.0);
        let title_color = if is_active { ACCENT } else { Color32::WHITE };
        ui.painter().text(
            text_pos,
            egui::Align2::LEFT_TOP,
            clip(&track.title, 16),
            FontId::proportional(14.0),
            title_color,
        );
        let sub = if loading { s.yt_loading } else { track.artist.as_str() };
        ui.painter().text(
            text_pos + Vec2::new(0.0, 18.0),
            egui::Align2::LEFT_TOP,
            clip(sub, 18),
            FontId::proportional(12.0),
            TEXT_MUTED,
        );

        response.clicked() && !loading
    }
}
