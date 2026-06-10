//! Central panel: the top search row plus a dispatch to the active view.
//!
//! The actual page bodies live in sibling modules:
//! * [`super::playlist_page`] — a specific playlist or the Liked music page.
//! * [`super::home_page`]     — the "Listen again" card grid.

use crate::app::App;
use crate::lang::strings;
use crate::theme::{BG_MAIN, TEXT_MUTED};
use eframe::egui;
use egui::{Color32, RichText, Rounding};

impl App {
    /// Draws the central panel: search/profile row, then the active page.
    pub(in crate::app) fn ui_central(&mut self, ctx: &egui::Context) {
        let s = strings(self.language);

        // The Now Playing view replaces the whole central panel (its own
        // adaptive background, no search row).
        if self.show_now_playing {
            egui::CentralPanel::default()
                .frame(egui::Frame::none().fill(BG_MAIN))
                .show(ctx, |ui| {
                    self.ui_now_playing(ui);
                });
            return;
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(BG_MAIN).inner_margin(24.0))
            .show(ctx, |ui| {
                // Top row: search field on the left, profile button on the right.
                ui.horizontal(|ui| {
                    // Search box: custom rounded background with an icon + field.
                    let search_rect = ui
                        .allocate_exact_size(egui::vec2(400.0, 32.0), egui::Sense::hover())
                        .0;
                    ui.painter()
                        .rect_filled(search_rect, Rounding::same(16.0), Color32::from_rgb(30, 30, 30));

                    let mut search_ui = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(search_rect.shrink(8.0))
                            .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    );
                    search_ui.add_space(8.0);
                    search_ui.label(RichText::new("🔍").size(14.0).color(TEXT_MUTED));

                    let response = search_ui.add(
                        egui::TextEdit::singleline(&mut self.search_query)
                            .frame(false)
                            .hint_text(RichText::new(s.search_hint).color(TEXT_MUTED))
                            .text_color(Color32::WHITE)
                            .desired_width(340.0),
                    );

                    // Focusing or typing in search jumps back to Home, where the
                    // results grid lives.
                    if response.gained_focus() || response.changed() {
                        self.selected_playlist_idx = None;
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let user_btn = ui.add(
                            egui::Button::new(RichText::new(s.user).size(13.0))
                                .rounding(15.0)
                                .fill(Color32::from_rgb(10, 10, 10)),
                        );
                        if user_btn.clicked() {
                            self.show_settings = true;
                        }
                    });
                });
                ui.add_space(20.0);

                // Dispatch to the active page.
                if let Some(idx) = self.selected_playlist_idx {
                    self.ui_playlist_page(ui, idx);
                } else {
                    self.ui_home_page(ui);
                }
            });
    }
}
