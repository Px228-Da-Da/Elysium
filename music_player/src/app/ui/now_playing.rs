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

impl App {
    /// Draws the Now Playing view into `ui` (the central panel area).
    pub(in crate::app) fn ui_now_playing(&mut self, ui: &mut egui::Ui) {
        // Read what we need up front so no borrow on `self` is held while drawing.
        let song = self.current_song.clone();
        let (bg, cover_id, title) = {
            let meta = self.track_meta.get(&self.current_song);
            // Background tone: from the real cover when there is one, otherwise a
            // dark version of the *generated* cover's gradient, so the backdrop
            // still matches the on-screen cover instead of a fixed fallback.
            let bg = meta.and_then(|m| m.bg).unwrap_or_else(|| {
                let (_, deep) = crate::theme::gen_gradient(&song);
                crate::theme::lerp_color(deep, Color32::BLACK, 0.45)
            });
            let cover_id = meta.and_then(|m| m.cover.as_ref()).map(|t| t.id());
            // Title used for the generated cover's letters when there's no art.
            let title = meta
                .map(|m| m.title.clone())
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| {
                    std::path::Path::new(&song)
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default()
                });
            (bg, cover_id, title)
        };
        let lyrics = self.current_lyrics.clone();
        let time_ms = self.current_playback_time_ms;
        // Whether a background lyrics fetch is still in progress (so we can tell
        // "searching" apart from "genuinely not found").
        let searching = self.lyrics_receiver.is_some();
        let s = strings(self.language);

        let area = ui.max_rect();
        // Living background: a gradient built from the cover's tone that slowly
        // flows/shimmers while the track plays. Frozen (still a gradient) when
        // paused. Repaint continuously only while playing so it actually moves.
        let time = ui.input(|i| i.time) as f32;
        paint_flowing_bg(ui.painter(), area, bg, time);
        if self.is_playing {
            ui.ctx().request_repaint();
        }

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

        // --- "Лаб" button (left of the chevron) ---
        // Same style as the main header: an accent-outlined pill that fills with
        // the user's accent color (short fade) on hover, label turning white.
        let lab_rect = Rect::from_center_size(
            pos2(area.right() - 110.0, area.top() + 40.0),
            vec2(72.0, 30.0),
        );
        let lab_resp = ui.interact(lab_rect, ui.id().with("now_playing_lab"), egui::Sense::click());
        let accent = crate::theme::accent();
        let hovered = lab_resp.hovered();
        let t = ui
            .ctx()
            .animate_bool_with_time(ui.id().with("np_lab_hover"), hovered, 0.12);
        let fill =
            Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), (t * 255.0) as u8);
        let text_col = crate::theme::lerp_color(accent, Color32::WHITE, t);
        ui.painter().rect(lab_rect, Rounding::same(15.0), fill, Stroke::new(1.5, accent));
        ui.painter().text(
            lab_rect.center(),
            egui::Align2::CENTER_CENTER,
            s.lab_button,
            FontId::proportional(13.5),
            text_col,
        );
        if hovered {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if lab_resp.clicked() {
            self.show_lab = true;
        }

        // --- Cover (left) ---
        let cover_size = (area.height() * 0.5).clamp(180.0, 380.0);
        let cover_center = pos2(area.left() + area.width() * 0.28, area.center().y);
        let cover_rect = Rect::from_center_size(cover_center, vec2(cover_size, cover_size));
        match cover_id {
            Some(id) => {
                ui.painter().rect_filled(cover_rect, Rounding::same(10.0), Color32::from_black_alpha(60));
                ui.painter().image(
                    id,
                    cover_rect,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            None => {
                // No embedded art: draw the same generated gradient + letters
                // used by the grid/mini-player, so the cover is never blank.
                let (c1, c2) = crate::theme::gen_gradient(&song);
                crate::theme::gradient_rrect(ui.painter(), cover_rect, 10.0, c1, c2);
                let label = crate::theme::cover_label(&title);
                if !label.is_empty() {
                    ui.painter().text(
                        cover_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        &label,
                        FontId::proportional(cover_size * 0.33),
                        crate::theme::gen_glyph(&song),
                    );
                }
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

        // Some tracks only have plain, un-timed lyrics (represented as lines all
        // at `time_ms == 0`). Those can't drive the karaoke highlight, so we show
        // them as a static, uniformly styled list with no auto-follow.
        let synced = crate::scanner::is_synced(&lyrics);

        // The active line is the last one whose timestamp has passed (synced only).
        let active_idx = if synced {
            lyrics
                .iter()
                .rposition(|line| time_ms >= line.time_ms)
                .map(|i| i as i32)
                .unwrap_or(-1)
        } else {
            -1
        };

        egui::ScrollArea::vertical()
            .id_salt("now_playing_lyrics")
            .auto_shrink([false, false])
            .show(&mut lyr_ui, |ui| {
                // Pad so the first/last lines can scroll to the vertical center.
                ui.add_space(lyrics_rect.height() / 2.0);
                for (i, line) in lyrics.iter().enumerate() {
                    let is_active = i as i32 == active_idx;
                    let (color, size) = if !synced {
                        // Plain lyrics: every line the same, comfortably readable.
                        (Color32::from_white_alpha(220), 22.0)
                    } else if is_active {
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

/// Paints a slowly flowing gradient across `area`, built from the cover tone
/// `bg`. A rotating linear gradient (from `bg` to a brighter tint of it) is
/// combined with a gentle two-axis sine ripple, so the background subtly
/// shimmers and "переливается" over time. `time` is the current time in seconds;
/// pass a frozen value to hold a still frame.
fn paint_flowing_bg(painter: &egui::Painter, area: Rect, bg: Color32, time: f32) {
    // A brighter, same-hue tint of the base tone for the light end of the sweep.
    let bright = Color32::from_rgb(
        (bg.r() as f32 * 1.8 + 14.0).min(255.0) as u8,
        (bg.g() as f32 * 1.8 + 14.0).min(255.0) as u8,
        (bg.b() as f32 * 1.8 + 14.0).min(255.0) as u8,
    );

    // Slowly rotating gradient direction.
    let ang = time * 0.20;
    let dir = vec2(ang.cos(), ang.sin());
    let center = area.center();
    let half_diag = (area.size().length() * 0.5).max(1.0);
    let (aw, ah) = (area.width().max(1.0), area.height().max(1.0));

    let col_at = |p: egui::Pos2| {
        // Projection onto the (rotating) gradient axis, in -1..1.
        let d = ((p - center).dot(dir) / half_diag).clamp(-1.0, 1.0);
        // A soft flowing ripple so it does not read as a flat linear ramp.
        let wave = 0.16
            * (((p.x / aw) * 2.4 + time * 0.7).sin() + ((p.y / ah) * 2.4 - time * 0.5).sin());
        let m = ((0.5 + 0.5 * d) * 0.7 + wave).clamp(0.0, 1.0);
        crate::theme::lerp_color(bg, bright, m)
    };

    // A fine grid of colored vertices approximates the smooth gradient + ripple.
    let (cols, rows) = (14usize, 14usize);
    let mut mesh = egui::epaint::Mesh::default();
    for iy in 0..=rows {
        for ix in 0..=cols {
            let x = area.left() + aw * ix as f32 / cols as f32;
            let y = area.top() + ah * iy as f32 / rows as f32;
            let p = pos2(x, y);
            mesh.colored_vertex(p, col_at(p));
        }
    }
    let stride = (cols + 1) as u32;
    for iy in 0..rows as u32 {
        for ix in 0..cols as u32 {
            let i0 = iy * stride + ix;
            mesh.add_triangle(i0, i0 + 1, i0 + stride);
            mesh.add_triangle(i0 + 1, i0 + stride + 1, i0 + stride);
        }
    }
    painter.add(mesh);
}
