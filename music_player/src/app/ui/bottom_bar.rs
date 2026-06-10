//! Bottom transport bar: now-playing info, controls, progress and volume.
//!
//! Laid out in three equal columns: track info + like on the left, play
//! controls and the seek slider in the center, and the volume slider on the
//! right.

use crate::app::App;
use crate::lang::strings;
use crate::theme::{format_duration, ACCENT, TEXT_MUTED};
use eframe::egui;
use egui::{pos2, Color32, FontId, Rect, RichText, Vec2};
use std::time::Duration;

/// Truncates `text` to at most `max_chars` characters, appending "…" (as "...")
/// when it had to be cut. Counts by `char`, so multi-byte text is handled safely.
fn truncate_ellipsis(text: &str, max_chars: usize) -> String {
    if text.chars().count() > max_chars {
        let keep = max_chars.saturating_sub(3);
        format!("{}...", text.chars().take(keep).collect::<String>())
    } else {
        text.to_string()
    }
}

impl App {
    /// Draws the bottom control bar.
    pub(in crate::app) fn ui_bottom_bar(&mut self, ctx: &egui::Context) {
        let s = strings(self.language);

        egui::TopBottomPanel::bottom("bottom_bar")
            .resizable(false)
            .min_height(90.0)
            .frame(
                egui::Frame::none()
                    .fill(Color32::from_rgb(0, 0, 0))
                    .inner_margin(16.0),
            )
            .show(ctx, |ui| {
                let total_w = ui.available_width();
                let col_w = total_w / 3.0;

                // Clicking anywhere on the bar toggles the full Now Playing view
                // (like YouTube Music). Registered first, so the controls below
                // (buttons/sliders) sit on top and capture their own clicks;
                // only clicks on empty areas fall through to here.
                if !self.current_song.is_empty() {
                    let bar_rect = ui.max_rect();
                    let bar = ui.interact(bar_rect, ui.id().with("now_playing_bar"), egui::Sense::click());
                    if bar.clicked() {
                        self.show_now_playing = !self.show_now_playing;
                    }
                    if bar.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                }

                ui.horizontal(|ui| {
                    // --- LEFT: cover + track info + like button ---
                    ui.allocate_ui_with_layout(
                        egui::vec2(col_w, ui.available_height()),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.set_width(col_w);
                            let meta = self.track_meta.get(&self.current_song);
                            let (rect, _) =
                                ui.allocate_exact_size(egui::vec2(56.0, 56.0), egui::Sense::hover());

                            match meta.and_then(|m| m.cover.as_ref()) {
                                Some(tex) => {
                                    ui.painter().image(
                                        tex.id(),
                                        rect,
                                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                                        Color32::WHITE,
                                    );
                                }
                                None => {
                                    ui.painter().rect_filled(rect, 4.0, Color32::from_rgb(40, 40, 40));
                                    ui.painter().text(
                                        rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        "🎵",
                                        FontId::proportional(24.0),
                                        TEXT_MUTED,
                                    );
                                }
                            }

                            ui.vertical(|ui| {
                                ui.add_space(8.0);
                                let title = meta
                                    .map(|m| m.title.clone())
                                    .unwrap_or_else(|| "No track selected".to_string());
                                let artist = meta
                                    .and_then(|m| m.artist.clone())
                                    .unwrap_or_else(|| "Unknown Artist".to_string());

                                // Truncate both lines to a single line so a long
                                // title or a long list of artists cannot wrap and
                                // push the rest of the bar out of place. The artist
                                // line uses a smaller font, so it fits a few more
                                // characters than the title.
                                let title_max = ((col_w / 12.0) as usize).clamp(15, 30);
                                let artist_max = ((col_w / 9.0) as usize).clamp(20, 40);
                                let display_name = truncate_ellipsis(&title, title_max);
                                let display_artist = truncate_ellipsis(&artist, artist_max);

                                // Never wrap: keep each on exactly one line.
                                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                                ui.label(RichText::new(display_name).size(14.0).strong().color(Color32::WHITE));
                                if !display_artist.is_empty() {
                                    ui.label(RichText::new(display_artist).size(12.0).color(TEXT_MUTED));
                                }
                            });

                            // ❤ Heart button: toggles the current track's like.
                            // Green = liked, gray = not.
                            if !self.current_song.is_empty() {
                                ui.add_space(12.0);
                                let song = self.current_song.clone();
                                let liked = self.is_liked(&song);
                                let heart_color = if liked { ACCENT } else { TEXT_MUTED };
                                let heart = ui
                                    .add(
                                        egui::Button::new(RichText::new("❤").size(20.0).color(heart_color))
                                            .fill(Color32::TRANSPARENT)
                                            .frame(false)
                                            .min_size(Vec2::new(34.0, 34.0)),
                                    )
                                    .on_hover_text(if liked { s.unlike_hint } else { s.like_hint });
                                if heart.clicked() {
                                    self.toggle_like(&song);
                                }
                            }
                        },
                    );

                    // --- CENTER: play controls + seek slider ---
                    ui.allocate_ui_with_layout(
                        egui::vec2(col_w, ui.available_height()),
                        egui::Layout::top_down(egui::Align::Center),
                        |ui| {
                            ui.set_width(col_w);
                            ui.add_space(8.0);

                            ui.horizontal(|ui| {
                                // Center the row of buttons within the column.
                                let buttons_width = 190.0;
                                let available_space = ui.available_width();
                                if available_space > buttons_width {
                                    ui.add_space((available_space - buttons_width) / 2.0);
                                }
                                ui.spacing_mut().item_spacing.x = 18.0;

                                let prev_btn = ui.add(
                                    egui::Button::new(RichText::new("⏮").size(16.0).color(Color32::WHITE))
                                        .fill(Color32::from_rgb(30, 30, 30))
                                        .rounding(100.0)
                                        .min_size(Vec2::new(32.0, 32.0)),
                                );
                                if prev_btn.clicked() {
                                    self.play_previous_track();
                                }

                                let play_icon = if self.is_playing { "⏸" } else { "▶" };
                                let play_btn = ui.add(
                                    egui::Button::new(RichText::new(play_icon).size(18.0).color(Color32::BLACK))
                                        .fill(Color32::WHITE)
                                        .rounding(100.0)
                                        .min_size(Vec2::new(32.0, 32.0)),
                                );
                                if play_btn.clicked() && !self.current_song.is_empty() {
                                    if self.is_playing {
                                        self.player.pause();
                                        self.is_playing = false;
                                    } else {
                                        self.player.resume();
                                        self.is_playing = true;
                                    }
                                }

                                let next_btn = ui.add(
                                    egui::Button::new(RichText::new("⏭").size(16.0).color(Color32::WHITE))
                                        .fill(Color32::from_rgb(30, 30, 30))
                                        .rounding(100.0)
                                        .min_size(Vec2::new(32.0, 32.0)),
                                );
                                if next_btn.clicked() {
                                    self.play_next_track();
                                }
                            });

                            ui.add_space(8.0);
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;
                                ui.label(
                                    RichText::new(format_duration(self.elapsed_duration))
                                        .size(11.0)
                                        .color(TEXT_MUTED),
                                );

                                let total_secs_f32 =
                                    self.total_duration.map(|d| d.as_secs_f32()).unwrap_or(0.0);
                                let mut current_secs = self.elapsed_duration.as_secs_f32();
                                ui.style_mut().spacing.slider_width = (col_w - 100.0).max(150.0);

                                if total_secs_f32 > 0.0 {
                                    let slider = ui.add(
                                        egui::Slider::new(&mut current_secs, 0.0..=total_secs_f32)
                                            .show_value(false)
                                            .trailing_fill(true),
                                    );
                                    // Update the visible position live while dragging...
                                    if slider.changed() {
                                        self.elapsed_duration = Duration::from_secs_f32(current_secs);
                                    }
                                    // ...but only perform the (expensive) seek on release.
                                    if slider.drag_stopped() && !self.current_song.is_empty() {
                                        let new_pos = Duration::from_secs_f32(current_secs);
                                        self.player.seek(&self.current_song, new_pos);
                                        self.is_playing = true;
                                    }
                                } else {
                                    // No track loaded: show a disabled placeholder slider.
                                    let mut dummy = 0.0;
                                    ui.add_enabled(
                                        false,
                                        egui::Slider::new(&mut dummy, 0.0..=1.0).show_value(false),
                                    );
                                }

                                ui.label(
                                    RichText::new(
                                        self.total_duration
                                            .map(format_duration)
                                            .unwrap_or_else(|| "0:00".to_string()),
                                    )
                                    .size(11.0)
                                    .color(TEXT_MUTED),
                                );
                            });
                        },
                    );

                    // --- RIGHT: volume ---
                    ui.allocate_ui_with_layout(
                        egui::vec2(col_w, ui.available_height()),
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            ui.set_width(col_w);
                            ui.add_space(10.0);
                            ui.style_mut().spacing.slider_width = 80.0;
                            if ui
                                .add(
                                    egui::Slider::new(&mut self.volume, 0.0..=1.0)
                                        .show_value(false)
                                        .trailing_fill(true),
                                )
                                .changed()
                            {
                                self.player.set_volume(self.volume);
                            }
                            ui.label(RichText::new("🔊").size(14.0).color(TEXT_MUTED));
                        },
                    );
                });
            });
    }
}
