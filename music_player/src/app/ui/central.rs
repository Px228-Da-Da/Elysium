//! Central panel: the top search row plus a dispatch to the active view.
//!
//! The actual page bodies live in sibling modules:
//! * [`super::playlist_page`] — a specific playlist or the Liked music page.
//! * [`super::home_page`]     — the "Listen again" card grid.

use crate::app::App;
use crate::lang::strings;
use crate::theme::{bg_main, line, surface, text, text_muted};
use eframe::egui;
use egui::{Align2, Color32, FontId, RichText, Rounding, Stroke};

impl App {
    /// Draws the central panel: search/profile row, then the active page.
    pub(in crate::app) fn ui_central(&mut self, ctx: &egui::Context) {
        let s = strings(self.language);

        // The Now Playing view replaces the whole central panel (its own
        // adaptive background, no search row).
        if self.show_now_playing {
            egui::CentralPanel::default()
                .frame(egui::Frame::none().fill(bg_main()))
                .show(ctx, |ui| {
                    self.ui_now_playing(ui);
                });
            return;
        }

        // The YouTube ("ЮБ") tab has its own search box, so it also replaces the
        // whole central panel (no library search row).
        if self.show_youtube {
            egui::CentralPanel::default()
                .frame(egui::Frame::none().fill(self.bg_tint).inner_margin(24.0))
                .show(ctx, |ui| {
                    self.ui_youtube_page(ui);
                });
            return;
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(self.bg_tint).inner_margin(24.0))
            .show(ctx, |ui| {
                // Top row: search field on the left, profile button on the right.
                ui.horizontal(|ui| {
                    // Search box: custom rounded background with an icon + field.
                    // The whole box is clickable — clicking anywhere in it (not
                    // just on the text) focuses the input, which feels natural.
                    let (search_rect, box_resp) =
                        ui.allocate_exact_size(egui::vec2(460.0, 40.0), egui::Sense::click());
                    ui.painter().rect(
                        search_rect,
                        Rounding::same(14.0),
                        surface(),
                        egui::Stroke::new(1.0, line()),
                    );

                    let mut search_ui = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(search_rect.shrink(8.0))
                            .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    );
                    search_ui.add_space(8.0);
                    let (icon_rect, _) =
                        search_ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                    crate::icons::paint(&search_ui, icon_rect, crate::icons::Icon::Search, text_muted());
                    search_ui.add_space(6.0);

                    let response = search_ui.add(
                        egui::TextEdit::singleline(&mut self.search_query)
                            .frame(false)
                            .hint_text(RichText::new(s.search_hint).color(text_muted()))
                            .text_color(text())
                            .desired_width(340.0),
                    );

                    // A click on the box's padding/icon (anywhere but the field
                    // itself) also focuses the field; show a text cursor on hover.
                    if box_resp.clicked() {
                        response.request_focus();
                    }
                    if box_resp.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
                    }

                    // Focusing or typing in search jumps back to Home, where the
                    // results grid lives.
                    if response.gained_focus() || response.changed() {
                        self.selected_playlist_idx = None;
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // "Лаб" — opens the audio-analysis window. A rounded pill
                        // with just an accent outline at rest; on hover it fills
                        // (with a short fade) with the accent color the user chose
                        // during onboarding, and its label turns white.
                        let accent = crate::theme::accent();
                        let (lab_rect, lab_resp) =
                            ui.allocate_exact_size(egui::vec2(72.0, 36.0), egui::Sense::click());
                        let hovered = lab_resp.hovered();
                        let t = ui
                            .ctx()
                            .animate_bool_with_time(ui.id().with("lab_btn_hover"), hovered, 0.12);
                        let fill = Color32::from_rgba_unmultiplied(
                            accent.r(),
                            accent.g(),
                            accent.b(),
                            (t * 255.0) as u8,
                        );
                        let text_col = crate::theme::lerp_color(accent, Color32::WHITE, t);
                        ui.painter().rect(lab_rect, Rounding::same(18.0), fill, Stroke::new(1.5, accent));
                        ui.painter().text(
                            lab_rect.center(),
                            Align2::CENTER_CENTER,
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

                        ui.add_space(14.0);

                        // "Профіль": a single pill holding the account icon + label
                        // (the glyph in `s.user` is stripped so the SVG icon is used).
                        // On hover it fills with a contrasting pill for feedback.
                        let user_label = s.user.trim_start_matches(|c: char| !c.is_alphanumeric()).trim();
                        let font = egui::FontId::proportional(13.5);
                        let (icon_sz, gap, pad_x) = (16.0, 7.0, 14.0);
                        let label_w = ui
                            .fonts(|f| f.layout_no_wrap(user_label.to_string(), font.clone(), text()))
                            .rect
                            .width();
                        let pill_w = pad_x * 2.0 + icon_sz + gap + label_w;
                        let (pill_rect, user_btn) =
                            ui.allocate_exact_size(egui::vec2(pill_w, 36.0), egui::Sense::click());
                        let hov = user_btn.hovered();
                        let content_col = if hov { crate::theme::bg_main() } else { text_muted() };
                        if hov {
                            ui.painter().rect_filled(pill_rect, Rounding::same(18.0), text());
                        }
                        let icon_rect = egui::Rect::from_min_size(
                            egui::pos2(pill_rect.left() + pad_x, pill_rect.center().y - icon_sz / 2.0),
                            egui::vec2(icon_sz, icon_sz),
                        );
                        crate::icons::paint(ui, icon_rect, crate::icons::Icon::Account, content_col);
                        let galley = ui.fonts(|f| f.layout_no_wrap(user_label.to_string(), font, content_col));
                        ui.painter().galley(
                            egui::pos2(
                                pill_rect.left() + pad_x + icon_sz + gap,
                                pill_rect.center().y - galley.rect.height() / 2.0,
                            ),
                            galley,
                            content_col,
                        );
                        if hov {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if user_btn.clicked() {
                            self.show_settings = true;
                        }
                    });
                });
                ui.add_space(6.0);

                // Dispatch to the active page.
                if let Some(idx) = self.selected_playlist_idx {
                    self.ui_playlist_page(ui, idx);
                } else {
                    self.ui_home_page(ui);
                }
            });
    }
}
