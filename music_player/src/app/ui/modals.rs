//! Modal dialogs: the update prompt, "New playlist" and "Rename playlist".
//!
//! Each is a foreground overlay that dims the screen and intercepts clicks
//! behind it. The two playlist dialogs share the same shape (centered card,
//! single text field with a focus-once request, Enter/Esc handling, and
//! Create-or-Cancel buttons).

use crate::app::{App, MUSIC_ROOT};
use crate::config::{load_deleted_playlists, save_deleted_playlists};
use crate::lang::{strings, Lang};
use crate::scanner::Playlist;
use crate::theme::{accent, text, text_muted};
use eframe::egui;
use egui::{pos2, vec2, Color32, Rect, RichText, Rounding};

impl App {
    /// Draws the "update available" prompt and drives the download when the
    /// user accepts. Shown only while the shared update state is marked
    /// available. Styled as a centered modal card (dimmed backdrop) to match the
    /// rest of the app rather than egui's default window.
    pub(in crate::app) fn ui_update_modal(&mut self, ctx: &egui::Context) {
        let update_info = self.update.lock().unwrap().clone();
        if !update_info.available {
            return;
        }

        let s = strings(self.language);
        let screen = ctx.screen_rect();
        // The card grows a little when an error message has to be shown.
        let win_h = if update_info.error.is_some() { 280.0 } else { 230.0 };
        let win_rect = Rect::from_center_size(screen.center(), vec2(440.0, win_h));

        egui::Area::new(egui::Id::new("update_overlay"))
            .order(egui::Order::Foreground)
            .interactable(true)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                ui.set_clip_rect(screen);

                // Dim the background and intercept clicks behind the dialog.
                let _ = ui.allocate_rect(screen, egui::Sense::click_and_drag());
                ui.painter().rect_filled(screen, Rounding::ZERO, Color32::from_black_alpha(170));

                // Card with a subtle accent border.
                ui.painter().rect_filled(win_rect, Rounding::same(16.0), crate::theme::surface_2());
                ui.painter().rect_stroke(
                    win_rect,
                    Rounding::same(16.0),
                    egui::Stroke::new(1.0, crate::theme::line()),
                );

                let mut content = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(win_rect.shrink(26.0))
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );

                // Header: accent download glyph + title.
                content.horizontal(|ui| {
                    let (dl_rect, _) = ui.allocate_exact_size(vec2(26.0, 26.0), egui::Sense::hover());
                    crate::icons::paint(ui, dl_rect, crate::icons::Icon::Update, accent());
                    ui.add_space(8.0);
                    ui.label(RichText::new(s.update_title).size(22.0).strong().color(text()));
                });
                content.add_space(14.0);

                // Version line.
                content.label(
                    RichText::new(s.update_available.replace("{v}", &update_info.latest_version))
                        .size(15.0)
                        .color(text_muted()),
                );

                // Previous attempt's error, if any.
                if let Some(ref err) = update_info.error {
                    content.add_space(8.0);
                    content.label(
                        RichText::new(format!("{}{}", s.update_error, err))
                            .size(13.0)
                            .color(Color32::from_rgb(240, 110, 110)),
                    );
                }

                // Push the actions to the bottom of the card.
                let remaining = content.available_height();
                content.add_space((remaining - 44.0).max(16.0));

                if update_info.updating {
                    // Download in progress: spinner instead of buttons.
                    content.horizontal(|ui| {
                        ui.spinner();
                        ui.add_space(8.0);
                        ui.label(RichText::new(s.update_downloading).size(14.0).color(text()));
                    });
                } else {
                    content.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Primary action: update now.
                        let update_btn = ui.add(
                            egui::Button::new(RichText::new(s.update_now).size(15.0).strong().color(Color32::BLACK))
                                .fill(accent())
                                .rounding(20.0)
                                .min_size(vec2(170.0, 42.0)),
                        );
                        if update_btn.clicked() {
                            self.start_self_update();
                        }

                        ui.add_space(10.0);

                        // Secondary action: dismiss for this session.
                        let later_btn = ui.add(
                            egui::Button::new(RichText::new(s.update_later).size(15.0).color(text()))
                                .fill(crate::theme::line())
                                .rounding(20.0)
                                .min_size(vec2(120.0, 42.0)),
                        );
                        if later_btn.clicked() {
                            // Hide until the next launch (state is in-memory only).
                            self.update.lock().unwrap().available = false;
                        }
                    });
                }
            });
    }

    /// Kicks off the in-place self-update on a background thread.
    ///
    /// Flips the shared state to "downloading", then downloads and replaces the
    /// binary. On success the process exits so the new version runs next launch;
    /// on failure the error is stored for the modal to display.
    fn start_self_update(&self) {
        let update_clone = self.update.clone();
        {
            let mut state = update_clone.lock().unwrap();
            state.updating = true;
            state.error = None;
        }

        std::thread::spawn(move || {
            let result = self_update::backends::github::Update::configure()
                .repo_owner("Px228-Da-Da")
                .repo_name("Elysium")
                .bin_name("Elysium")
                .show_download_progress(false) // important: false for GUI apps
                .current_version(env!("CARGO_PKG_VERSION"))
                .build()
                .unwrap()
                .update();

            let mut state = update_clone.lock().unwrap();
            state.updating = false;

            match result {
                // Success: the binary was replaced; exit so the new version runs
                // on the next manual launch.
                Ok(_) => std::process::exit(0),
                Err(e) => state.error = Some(e.to_string()),
            }
        });
    }

    /// Draws the "New playlist" dialog when [`App::show_new_playlist`] is set.
    ///
    /// Creating a playlist makes its folder under [`MUSIC_ROOT`], un-deletes the
    /// name if it had been deleted, appends it, opens it, and persists.
    pub(in crate::app) fn ui_new_playlist(&mut self, ctx: &egui::Context) {
        if !self.show_new_playlist {
            return;
        }

        let s = strings(self.language);
        let screen = ctx.screen_rect();
        let win_rect = Rect::from_center_size(screen.center(), vec2(360.0, 190.0));

        egui::Area::new(egui::Id::new("new_playlist_overlay"))
            .order(egui::Order::Foreground)
            .interactable(true)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                ui.set_clip_rect(screen);

                // Dim the background and intercept clicks behind the dialog.
                let _ = ui.allocate_rect(screen, egui::Sense::click_and_drag());
                ui.painter().rect_filled(screen, Rounding::same(0.0), Color32::from_black_alpha(160));

                // Esc closes the dialog.
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    self.new_playlist_name.clear();
                    self.show_new_playlist = false;
                }

                // Card.
                ui.painter().rect_filled(win_rect, Rounding::same(14.0), crate::theme::surface_2());

                let mut content = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(win_rect.shrink(20.0))
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );

                content.label(RichText::new(s.new_playlist_title).size(20.0).strong().color(text()));
                content.add_space(16.0);

                // Input field with its own rounded background + frameless TextEdit.
                let field_h = 44.0;
                let (field_rect, _) =
                    content.allocate_exact_size(vec2(content.available_width(), field_h), egui::Sense::hover());
                content.painter().rect_filled(field_rect, Rounding::same(10.0), crate::theme::surface());

                let mut field_ui = content.new_child(
                    egui::UiBuilder::new()
                        .max_rect(field_rect.shrink2(vec2(14.0, 0.0)))
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );
                let resp = field_ui.add(
                    egui::TextEdit::singleline(&mut self.new_playlist_name)
                        .hint_text(RichText::new(s.new_playlist_hint).color(text_muted()))
                        .frame(false)
                        .desired_width(f32::INFINITY),
                );

                if self.focus_new_playlist {
                    resp.request_focus();
                    self.focus_new_playlist = false;
                }

                // Accent underline while focused.
                if resp.has_focus() {
                    let underline = Rect::from_min_max(
                        pos2(field_rect.left() + 6.0, field_rect.bottom() - 3.0),
                        pos2(field_rect.right() - 6.0, field_rect.bottom() - 1.0),
                    );
                    content.painter().rect_filled(underline, Rounding::same(2.0), accent());
                }

                let enter_pressed = resp.lost_focus() && content.input(|i| i.key_pressed(egui::Key::Enter));
                content.add_space(22.0);

                let mut do_create = enter_pressed;
                let mut do_cancel = false;
                content.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(
                            egui::Button::new(RichText::new(s.create).size(15.0).color(Color32::BLACK))
                                .fill(accent())
                                .rounding(18.0)
                                .min_size(vec2(120.0, 36.0)),
                        )
                        .clicked()
                    {
                        do_create = true;
                    }

                    ui.add_space(10.0);

                    if ui
                        .add(
                            egui::Button::new(RichText::new(s.cancel).size(15.0).color(text()))
                                .fill(crate::theme::line())
                                .rounding(18.0)
                                .min_size(vec2(120.0, 36.0)),
                        )
                        .clicked()
                    {
                        do_cancel = true;
                    }
                });

                if do_create {
                    let name = self.new_playlist_name.trim().to_string();
                    if !name.is_empty() {
                        // Create the playlist's folder under MUSIC_ROOT.
                        let path = std::path::Path::new(MUSIC_ROOT).join(&name);
                        if let Err(e) = std::fs::create_dir_all(&path) {
                            eprintln!("Failed to create folder: {}", e);
                        }

                        // If this name was previously deleted, un-delete it so it
                        // does not vanish again on the next restart.
                        let mut deleted = load_deleted_playlists();
                        if deleted.remove(&name) {
                            save_deleted_playlists(&deleted);
                        }

                        self.playlists.push(Playlist { name, songs: Vec::new() });
                        self.selected_playlist_idx = Some(self.playlists.len() - 1);
                        self.save_playlists();
                    }
                    self.new_playlist_name.clear();
                    self.show_new_playlist = false;
                } else if do_cancel {
                    self.new_playlist_name.clear();
                    self.show_new_playlist = false;
                }
            });
    }

    /// Draws the "Rename playlist" dialog when [`App::rename_playlist_idx`] is set.
    pub(in crate::app) fn ui_rename_playlist(&mut self, ctx: &egui::Context) {
        if self.rename_playlist_idx.is_none() {
            return;
        }

        let screen = ctx.screen_rect();
        let win_rect = Rect::from_center_size(screen.center(), vec2(400.0, 210.0));
        let lang = self.language;

        egui::Area::new(egui::Id::new("rename_playlist_overlay"))
            .order(egui::Order::Foreground)
            .interactable(true)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                ui.set_clip_rect(screen);

                let _ = ui.allocate_rect(screen, egui::Sense::click_and_drag());
                ui.painter().rect_filled(screen, Rounding::same(0.0), Color32::from_black_alpha(160));

                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    self.rename_playlist_idx = None;
                    self.rename_playlist_name.clear();
                }

                ui.painter().rect_filled(win_rect, Rounding::same(14.0), crate::theme::surface_2());

                let mut content = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(win_rect.shrink(20.0))
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );

                let title_label = match lang {
                    Lang::Ru => "Переименовать плейлист",
                    Lang::Uk => "Перейменувати плейлист",
                    Lang::En => "Rename playlist",
                };
                content.label(RichText::new(title_label).size(20.0).strong().color(text()));
                content.add_space(16.0);

                let field_h = 44.0;
                let (field_rect, _) =
                    content.allocate_exact_size(vec2(content.available_width(), field_h), egui::Sense::hover());
                content.painter().rect_filled(field_rect, Rounding::same(10.0), crate::theme::surface());

                let mut field_ui = content.new_child(
                    egui::UiBuilder::new()
                        .max_rect(field_rect.shrink2(vec2(14.0, 0.0)))
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );
                let hint = match lang {
                    Lang::Ru => "Новое название",
                    Lang::Uk => "Нова назва",
                    Lang::En => "New name",
                };
                let resp = field_ui.add(
                    egui::TextEdit::singleline(&mut self.rename_playlist_name)
                        .hint_text(RichText::new(hint).color(text_muted()))
                        .frame(false)
                        .desired_width(f32::INFINITY),
                );
                if self.focus_rename_playlist {
                    resp.request_focus();
                    self.focus_rename_playlist = false;
                }
                if resp.has_focus() {
                    let underline = Rect::from_min_max(
                        pos2(field_rect.left() + 6.0, field_rect.bottom() - 3.0),
                        pos2(field_rect.right() - 6.0, field_rect.bottom() - 1.0),
                    );
                    content.painter().rect_filled(underline, Rounding::same(2.0), accent());
                }

                let enter_pressed = resp.lost_focus() && content.input(|i| i.key_pressed(egui::Key::Enter));
                content.add_space(16.0);

                let mut do_rename = enter_pressed;
                let mut do_cancel = false;
                content.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let save_label = match lang {
                        Lang::Ru => "Сохранить",
                        Lang::Uk => "Зберегти",
                        Lang::En => "Save",
                    };
                    if ui
                        .add(
                            egui::Button::new(RichText::new(save_label).size(15.0).color(Color32::BLACK))
                                .fill(accent())
                                .rounding(18.0)
                                .min_size(vec2(120.0, 36.0)),
                        )
                        .clicked()
                    {
                        do_rename = true;
                    }

                    ui.add_space(10.0);

                    let cancel_label = match lang {
                        Lang::Ru => "Отмена",
                        Lang::Uk => "Скасувати",
                        Lang::En => "Cancel",
                    };
                    if ui
                        .add(
                            egui::Button::new(RichText::new(cancel_label).size(15.0).color(text()))
                                .fill(crate::theme::line())
                                .rounding(18.0)
                                .min_size(vec2(120.0, 36.0)),
                        )
                        .clicked()
                    {
                        do_cancel = true;
                    }
                });

                if do_rename {
                    let new_name = self.rename_playlist_name.trim().to_string();
                    if !new_name.is_empty() {
                        if let Some(pl_idx) = self.rename_playlist_idx {
                            if pl_idx < self.playlists.len() {
                                self.playlists[pl_idx].name = new_name;
                                self.save_playlists();
                            }
                        }
                    }
                    self.rename_playlist_idx = None;
                    self.rename_playlist_name.clear();
                }
                if do_cancel {
                    self.rename_playlist_idx = None;
                    self.rename_playlist_name.clear();
                }
            });
    }

    /// Confirmation dialog shown before a playlist is actually deleted
    /// (triggered by the header trash button). Deletes only on "Delete".
    pub(in crate::app) fn ui_confirm_delete_playlist(&mut self, ctx: &egui::Context) {
        let Some(idx) = self.confirm_delete_playlist else {
            return;
        };
        // Resolve the name now; bail out if the index went stale.
        let Some(name) = self.playlists.get(idx).map(|p| p.name.clone()) else {
            self.confirm_delete_playlist = None;
            return;
        };
        let lang = self.language;
        let screen = ctx.screen_rect();
        let win_rect = Rect::from_center_size(screen.center(), vec2(420.0, 200.0));

        egui::Area::new(egui::Id::new("confirm_delete_overlay"))
            .order(egui::Order::Foreground)
            .interactable(true)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                ui.set_clip_rect(screen);
                let _ = ui.allocate_rect(screen, egui::Sense::click_and_drag());
                ui.painter().rect_filled(screen, Rounding::same(0.0), Color32::from_black_alpha(160));

                let mut do_delete = false;
                let mut do_cancel = ui.input(|i| i.key_pressed(egui::Key::Escape));

                ui.painter().rect_filled(win_rect, Rounding::same(14.0), crate::theme::surface_2());

                let mut content = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(win_rect.shrink(22.0))
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );

                let title = match lang {
                    Lang::Ru => "Удалить плейлист?",
                    Lang::Uk => "Видалити плейлист?",
                    Lang::En => "Delete playlist?",
                };
                content.label(RichText::new(title).size(20.0).strong().color(text()));
                content.add_space(10.0);

                let msg = match lang {
                    Lang::Ru => format!("«{}» будет удалён безвозвратно.", name),
                    Lang::Uk => format!("«{}» буде видалено безповоротно.", name),
                    Lang::En => format!("\u{201C}{}\u{201D} will be permanently deleted.", name),
                };
                content.label(RichText::new(msg).size(14.0).color(text_muted()));
                content.add_space(24.0);

                content.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let del_label = match lang {
                        Lang::Ru => "Удалить",
                        Lang::Uk => "Видалити",
                        Lang::En => "Delete",
                    };
                    if ui
                        .add(
                            egui::Button::new(RichText::new(del_label).size(15.0).color(Color32::WHITE))
                                .fill(Color32::from_rgb(220, 70, 70))
                                .rounding(18.0)
                                .min_size(vec2(120.0, 36.0)),
                        )
                        .clicked()
                    {
                        do_delete = true;
                    }

                    ui.add_space(10.0);

                    let cancel_label = match lang {
                        Lang::Ru => "Отмена",
                        Lang::Uk => "Скасувати",
                        Lang::En => "Cancel",
                    };
                    if ui
                        .add(
                            egui::Button::new(RichText::new(cancel_label).size(15.0).color(text()))
                                .fill(crate::theme::line())
                                .rounding(18.0)
                                .min_size(vec2(120.0, 36.0)),
                        )
                        .clicked()
                    {
                        do_cancel = true;
                    }
                });

                if do_delete {
                    self.delete_playlist(idx);
                    self.confirm_delete_playlist = None;
                }
                if do_cancel {
                    self.confirm_delete_playlist = None;
                }
            });
    }
}
