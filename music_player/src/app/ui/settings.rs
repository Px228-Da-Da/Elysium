//! Full-screen, modal settings overlay: language and hotkey configuration.
//!
//! Drawn in a foreground `Area` that covers the whole screen and swallows
//! clicks, so widgets behind it are unreachable while it is open.

use crate::app::App;
use crate::lang::{save_language, strings, Lang};
use crate::shortcuts::{key_label, save_shortcuts, Shortcut};
use crate::theme::{accent, bg_main, lerp_color, line, surface, surface_2, text, text_muted};
use eframe::egui;
use egui::{pos2, vec2, Align2, FontId, Rect, RichText, Rounding, Stroke, Vec2};

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
                ui.painter().rect_filled(screen, Rounding::same(0.0), bg_main());

                // Settings content, inset from the edges.
                let mut content = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(screen.shrink(40.0))
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );

                // Header: title left, close button right.
                content.horizontal(|ui| {
                    let (gear_rect, _) = ui.allocate_exact_size(vec2(28.0, 28.0), egui::Sense::hover());
                    crate::icons::paint(ui, gear_rect, crate::icons::Icon::Gear, text());
                    ui.add_space(8.0);
                    ui.label(RichText::new(s.settings).size(28.0).strong().color(text()));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (close_rect, close) = ui.allocate_exact_size(vec2(38.0, 38.0), egui::Sense::click());
                        ui.painter().rect_filled(close_rect, Rounding::same(18.0), surface_2());
                        crate::icons::paint(ui, close_rect.shrink(11.0), crate::icons::Icon::Close, text());
                        if close.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if close.clicked() {
                            self.show_settings = false;
                        }
                    });
                });

                // --- Language ---
                content.add_space(28.0);
                content.label(RichText::new(s.language).size(18.0).strong().color(text()));
                content.add_space(12.0);
                content.horizontal(|ui| {
                    for &lang in Lang::all() {
                        let active = self.language == lang;
                        // Selected: soft accent-tinted fill + accent text; others:
                        // plain surface with a hairline border and normal text.
                        let bg = if active { lerp_color(surface(), accent(), 0.22) } else { surface() };
                        let fg = if active { accent() } else { text() };
                        let border = if active { accent() } else { line() };
                        let btn = ui.add(
                            egui::Button::new(RichText::new(lang.native_name()).size(15.0).color(fg))
                                .min_size(vec2(160.0, 42.0))
                                .rounding(10.0)
                                .fill(bg)
                                .stroke(Stroke::new(1.0, border)),
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
                content.label(RichText::new(s.shortcuts).size(18.0).strong().color(text()));
                content.add_space(12.0);
                let actions = Shortcut::all();
                for (i, &action) in actions.iter().enumerate() {
                    // One full-width row: label on the left, a key chip in a
                    // right-aligned column, a trash icon (only when a key is set),
                    // and a hairline separator under all but the last row.
                    let row_h = 56.0;
                    let (row, _) =
                        content.allocate_exact_size(vec2(content.available_width(), row_h), egui::Sense::hover());

                    // Action name.
                    content.painter().text(
                        pos2(row.left(), row.center().y),
                        Align2::LEFT_CENTER,
                        action.label(self.language),
                        FontId::proportional(15.0),
                        text(),
                    );

                    let assigned = self.shortcuts.get(&action).is_some();
                    let listening = self.rebinding == Some(action);

                    // Trash icon at the far right (shown only for assigned keys).
                    let trash_rect = Rect::from_center_size(pos2(row.right() - 16.0, row.center().y), Vec2::splat(30.0));
                    let clear = content.interact(trash_rect, content.id().with(("hk_clear", action.code())), egui::Sense::click());
                    if assigned {
                        let tcol = if clear.hovered() { text() } else { text_muted() };
                        crate::icons::paint(&content, trash_rect.shrink(6.0), crate::icons::Icon::Delete, tcol);
                        if clear.hovered() {
                            content.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                    }

                    // Key chip, right-aligned to a fixed column left of the trash.
                    let (chip_w, chip_h) = (210.0, 40.0);
                    let chip_rect = Rect::from_min_size(
                        pos2(trash_rect.left() - 14.0 - chip_w, row.center().y - chip_h / 2.0),
                        vec2(chip_w, chip_h),
                    );
                    let key_resp = content.interact(chip_rect, content.id().with(("hk_key", action.code())), egui::Sense::click());
                    let bg = if listening {
                        lerp_color(surface(), accent(), 0.22)
                    } else if key_resp.hovered() {
                        lerp_color(surface_2(), text(), 0.06)
                    } else {
                        surface_2()
                    };
                    let fg = if listening {
                        accent()
                    } else if assigned {
                        text()
                    } else {
                        text_muted()
                    };
                    let key_text = if listening {
                        s.press_key.to_string()
                    } else {
                        match self.shortcuts.get(&action) {
                            Some(&key) => key_label(key),
                            None => s.not_set.to_string(),
                        }
                    };
                    content.painter().rect_filled(chip_rect, Rounding::same(12.0), bg);
                    content.painter().text(chip_rect.center(), Align2::CENTER_CENTER, &key_text, FontId::monospace(14.0), fg);
                    if key_resp.hovered() {
                        content.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if key_resp.clicked() {
                        self.rebinding = if listening { None } else { Some(action) };
                    }
                    if clear.clicked() && assigned {
                        self.shortcuts.remove(&action);
                        save_shortcuts(&self.shortcuts);
                        if self.rebinding == Some(action) {
                            self.rebinding = None;
                        }
                    }

                    // Hairline separator between rows.
                    if i + 1 < actions.len() {
                        content.painter().line_segment(
                            [pos2(row.left(), row.bottom()), pos2(row.right(), row.bottom())],
                            Stroke::new(1.0, line()),
                        );
                    }
                }

                // --- Placeholder for future settings (a bordered card) ---
                content.add_space(28.0);
                egui::Frame::none()
                    .fill(surface())
                    .stroke(Stroke::new(1.0, line()))
                    .rounding(Rounding::same(14.0))
                    .inner_margin(egui::Margin::symmetric(20.0, 16.0))
                    .show(&mut content, |ui| {
                        ui.set_width(ui.available_width().min(820.0));
                        ui.horizontal(|ui| {
                            let (tools_rect, _) = ui.allocate_exact_size(vec2(22.0, 22.0), egui::Sense::hover());
                            crate::icons::paint(ui, tools_rect, crate::icons::Icon::Tools, text_muted());
                            ui.add_space(8.0);
                            ui.vertical(|ui| {
                                ui.label(RichText::new(s.settings_in_dev).size(14.0).strong().color(text()));
                                ui.label(RichText::new(s.settings_in_dev_sub).size(12.0).color(text_muted()));
                            });
                        });
                    });
            });
    }
}
