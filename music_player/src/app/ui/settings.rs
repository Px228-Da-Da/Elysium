//! Full-screen, modal settings overlay: language and hotkey configuration.
//!
//! Drawn in a foreground `Area` that covers the whole screen and swallows
//! clicks, so widgets behind it are unreachable while it is open.

use crate::app::App;
use crate::lang::{save_language, strings, Lang};
use crate::shortcuts::{key_label, save_shortcuts, Shortcut};
use crate::theme::{ACCENT, TEXT_MUTED};
use eframe::egui;
use egui::{vec2, Color32, RichText, Rounding};

impl App {
    /// Draws the settings overlay when [`App::show_settings`] is set.
    pub(in crate::app) fn ui_settings(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }

        let s = strings(self.language);
        let screen = ctx.screen_rect();

        egui::Area::new(egui::Id::new("settings_overlay"))
            .order(egui::Order::Foreground) // above every panel
            .interactable(true)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                ui.set_clip_rect(screen);

                // Opaque full-screen background that also intercepts all clicks.
                let _ = ui.allocate_rect(screen, egui::Sense::click_and_drag());
                ui.painter().rect_filled(screen, Rounding::same(0.0), Color32::from_rgb(18, 18, 18));

                // Settings content, inset from the edges.
                let mut content = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(screen.shrink(40.0))
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );

                // Header: title left, close button right.
                content.horizontal(|ui| {
                    ui.label(RichText::new(format!("⚙  {}", s.settings)).size(28.0).strong().color(Color32::WHITE));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let close = ui.add(
                            egui::Button::new(RichText::new("✖").size(18.0).color(Color32::WHITE))
                                .rounding(18.0)
                                .fill(Color32::from_rgb(30, 30, 30)),
                        );
                        if close.clicked() {
                            self.show_settings = false;
                        }
                    });
                });

                // --- Language ---
                content.add_space(28.0);
                content.label(RichText::new(s.language).size(18.0).strong().color(Color32::WHITE));
                content.add_space(12.0);
                content.horizontal(|ui| {
                    for &lang in Lang::all() {
                        let active = self.language == lang;
                        let bg = if active { ACCENT } else { Color32::from_rgb(35, 35, 35) };
                        let fg = if active { Color32::BLACK } else { Color32::WHITE };
                        let btn = ui.add(
                            egui::Button::new(RichText::new(lang.native_name()).size(15.0).color(fg))
                                .min_size(vec2(160.0, 42.0))
                                .rounding(10.0)
                                .fill(bg),
                        );
                        if btn.clicked() {
                            self.language = lang;
                            save_language(lang);
                        }
                        ui.add_space(12.0);
                    }
                });

                // --- Hotkeys ---
                content.add_space(40.0);
                content.label(RichText::new(s.shortcuts).size(18.0).strong().color(Color32::WHITE));
                content.add_space(12.0);
                for &action in Shortcut::all() {
                    content.horizontal(|ui| {
                        // Action name (fixed width so the controls line up).
                        ui.allocate_ui_with_layout(
                            vec2(240.0, 30.0),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.label(RichText::new(action.label(self.language)).size(15.0).color(Color32::WHITE));
                            },
                        );

                        // Current-key button; clicking it waits for a key press.
                        let listening = self.rebinding == Some(action);
                        let key_text = if listening {
                            s.press_key.to_string()
                        } else {
                            match self.shortcuts.get(&action) {
                                Some(&key) => key_label(key),
                                None => s.not_set.to_string(),
                            }
                        };
                        let bg = if listening { ACCENT } else { Color32::from_rgb(35, 35, 35) };
                        let fg = if listening { Color32::BLACK } else { Color32::WHITE };
                        let key_btn = ui.add(
                            egui::Button::new(RichText::new(key_text).size(14.0).color(fg))
                                .min_size(vec2(190.0, 30.0))
                                .rounding(8.0)
                                .fill(bg),
                        );
                        if key_btn.clicked() {
                            self.rebinding = if listening { None } else { Some(action) };
                        }

                        ui.add_space(8.0);

                        // Clear this binding.
                        let clear = ui.add(
                            egui::Button::new(RichText::new("🗑").size(14.0).color(TEXT_MUTED))
                                .min_size(vec2(34.0, 30.0))
                                .rounding(8.0)
                                .fill(Color32::from_rgb(30, 30, 30)),
                        );
                        if clear.clicked() {
                            self.shortcuts.remove(&action);
                            save_shortcuts(&self.shortcuts);
                            if self.rebinding == Some(action) {
                                self.rebinding = None;
                            }
                        }
                    });
                    content.add_space(8.0);
                }

                // --- Placeholder for future settings ---
                content.add_space(28.0);
                content.horizontal(|ui| {
                    ui.label(RichText::new("🛠").size(20.0));
                    ui.add_space(8.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(s.settings_in_dev).size(14.0).strong().color(Color32::WHITE));
                        ui.label(RichText::new(s.settings_in_dev_sub).size(12.0).color(TEXT_MUTED));
                    });
                });
            });
    }
}
