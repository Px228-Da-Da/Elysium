//! Left sidebar: brand, navigation, "New playlist", Liked music and the list
//! of ordinary playlists.

use crate::app::{App, LIKED_PAGE_IDX, LIKED_PLAYLIST_NAME};
use crate::lang::strings;
use crate::theme::TEXT_MUTED;
use eframe::egui;
use egui::{pos2, vec2, Color32, FontId, RichText, Rounding};

impl App {
    /// Draws the left navigation sidebar.
    pub(in crate::app) fn ui_sidebar(&mut self, ctx: &egui::Context) {
        let s = strings(self.language);

        egui::SidePanel::left("sidebar_panel")
            .resizable(false)
            .exact_width(240.0)
            .frame(
                egui::Frame::none()
                    .fill(Color32::from_rgb(0, 0, 0))
                    .inner_margin(20.0),
            )
            .show(ctx, |ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new("Elysium").size(20.0).strong().color(Color32::WHITE));
                    ui.add_space(25.0);

                    // Navigation. Currently a single "Home" pill; kept as a loop
                    // so more entries can be added later.
                    let nav_items = [("🏠", s.home)];
                    for (i, (icon, name)) in nav_items.iter().enumerate() {
                        let is_active =
                            self.selected_playlist_idx.is_none() && !self.show_youtube && i == 0;

                        // Full-width clickable pill.
                        let (rect, response) =
                            ui.allocate_exact_size(vec2(ui.available_width(), 44.0), egui::Sense::click());

                        if is_active {
                            ui.painter().rect_filled(rect, Rounding::same(12.0), Color32::from_rgb(32, 32, 32));
                        } else if response.hovered() {
                            ui.painter().rect_filled(rect, Rounding::same(12.0), Color32::from_rgb(20, 20, 20));
                        }

                        // Bright white when active, muted otherwise.
                        let content_color = if is_active { Color32::WHITE } else { TEXT_MUTED };

                        // Icon (16px in from the left, vertically centered).
                        let icon_pos = pos2(rect.min.x + 16.0, rect.center().y);
                        ui.painter().text(icon_pos, egui::Align2::LEFT_CENTER, *icon, FontId::proportional(18.0), content_color);

                        // Label beside the icon.
                        let text_pos = pos2(rect.min.x + 48.0, rect.center().y);
                        ui.painter().text(text_pos, egui::Align2::LEFT_CENTER, *name, FontId::proportional(15.0), content_color);

                        if response.clicked() && i == 0 {
                            self.selected_playlist_idx = None;
                            self.search_query.clear();
                            self.show_now_playing = false;
                            self.show_youtube = false;
                        }
                        ui.add_space(12.0);
                    }

                    // "ЮБ" — ad-free YouTube Music search tab, sitting right under Home.
                    {
                        let is_active = self.show_youtube;
                        let (rect, response) = ui
                            .allocate_exact_size(vec2(ui.available_width(), 44.0), egui::Sense::click());

                        if is_active {
                            ui.painter().rect_filled(rect, Rounding::same(12.0), Color32::from_rgb(32, 32, 32));
                        } else if response.hovered() {
                            ui.painter().rect_filled(rect, Rounding::same(12.0), Color32::from_rgb(20, 20, 20));
                        }

                        let content_color = if is_active { Color32::WHITE } else { TEXT_MUTED };
                        let icon_pos = pos2(rect.min.x + 16.0, rect.center().y);
                        ui.painter().text(icon_pos, egui::Align2::LEFT_CENTER, "📺", FontId::proportional(18.0), content_color);
                        let text_pos = pos2(rect.min.x + 48.0, rect.center().y);
                        ui.painter().text(text_pos, egui::Align2::LEFT_CENTER, s.yt_tab, FontId::proportional(15.0), content_color);

                        if response.clicked() {
                            self.show_youtube = true;
                            self.show_now_playing = false;
                        }
                    }

                    ui.add_space(15.0);
                    ui.separator();
                    ui.add_space(15.0);

                    // "New playlist" — opens the creation modal.
                    let add_btn = ui.add_sized(
                        [ui.available_width(), 40.0],
                        egui::Button::new(RichText::new(s.new_playlist).size(16.0).strong())
                            .fill(Color32::from_rgb(30, 30, 30))
                            .rounding(20.0),
                    );
                    if add_btn.clicked() {
                        self.show_new_playlist = true;
                        self.focus_new_playlist = true;
                        self.new_playlist_name.clear();
                    }

                    ui.add_space(25.0);

                    // Liked music entry (addressed by the LIKED_PAGE_IDX sentinel).
                    let is_liked_selected =
                        self.selected_playlist_idx == Some(LIKED_PAGE_IDX) && !self.show_youtube;
                    let (rect, response) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 50.0), egui::Sense::click());

                    if is_liked_selected {
                        ui.painter().rect_filled(rect, Rounding::same(6.0), Color32::from_rgb(40, 40, 40));
                    } else if response.hovered() {
                        ui.painter().rect_filled(rect, Rounding::same(6.0), Color32::from_rgb(30, 30, 30));
                    }

                    let text_pos = rect.min + vec2(10.0, 8.0);
                    ui.painter().text(text_pos, egui::Align2::LEFT_TOP, s.liked_music, FontId::proportional(15.0), Color32::WHITE);
                    ui.painter().text(text_pos + vec2(0.0, 20.0), egui::Align2::LEFT_TOP, s.auto_created, FontId::proportional(12.0), TEXT_MUTED);

                    if response.clicked() {
                        self.selected_playlist_idx = Some(LIKED_PAGE_IDX);
                        self.search_query.clear();
                        self.show_now_playing = false;
                        self.show_youtube = false;
                    }

                    ui.add_space(20.0);

                    // Scrollable list of ordinary playlists.
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for (idx, playlist) in self.playlists.iter().enumerate() {
                            // Liked music has its own button above — skip it here.
                            if playlist.name == LIKED_PLAYLIST_NAME {
                                continue;
                            }
                            let is_selected =
                                self.selected_playlist_idx == Some(idx) && !self.show_youtube;

                            let (rect, response) =
                                ui.allocate_exact_size(vec2(ui.available_width(), 50.0), egui::Sense::click());

                            if is_selected {
                                ui.painter().rect_filled(rect, Rounding::same(6.0), Color32::from_rgb(40, 40, 40));
                            } else if response.hovered() {
                                ui.painter().rect_filled(rect, Rounding::same(6.0), Color32::from_rgb(30, 30, 30));
                            }

                            let text_pos = rect.min + vec2(10.0, 8.0);
                            ui.painter().text(text_pos, egui::Align2::LEFT_TOP, &playlist.name, FontId::proportional(15.0), Color32::WHITE);
                            ui.painter().text(text_pos + vec2(0.0, 20.0), egui::Align2::LEFT_TOP, "User", FontId::proportional(12.0), TEXT_MUTED);

                            if response.clicked() {
                                self.selected_playlist_idx = Some(idx);
                                self.search_query.clear();
                                self.show_now_playing = false;
                                self.show_youtube = false;
                            }

                            ui.add_space(4.0);
                        }
                    });
                });
            });
    }
}
