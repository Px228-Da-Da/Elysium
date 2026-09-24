//! Left sidebar: brand, navigation, "New playlist", Liked music and the list
//! of ordinary playlists.

use crate::app::{App, LIKED_PAGE_IDX, LIKED_PLAYLIST_NAME};
use crate::lang::{strings, Lang};
use crate::theme::{accent, gen_gradient, gradient_rrect, line, surface, surface_2, text, text_faint, text_muted};
use eframe::egui;
use egui::{pos2, vec2, Align2, Color32, FontId, Rect, Rounding};

impl App {
    /// Draws the left navigation sidebar.
    pub(in crate::app) fn ui_sidebar(&mut self, ctx: &egui::Context) {
        let s = strings(self.language);

        egui::SidePanel::left("sidebar_panel")
            .resizable(false)
            .exact_width(248.0)
            .frame(
                egui::Frame::none()
                    .fill(self.bg_tint)
                    .stroke(egui::Stroke::new(1.0, line()))
                    .inner_margin(20.0),
            )
            .show(ctx, |ui| {
                ui.vertical(|ui| {
                    // Brand + accent dot.
                    let (logo_rect, _) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 30.0), egui::Sense::hover());
                    let end = ui.painter().text(
                        pos2(logo_rect.left(), logo_rect.center().y),
                        Align2::LEFT_CENTER,
                        "Elysium",
                        FontId::proportional(22.0),
                        text(),
                    );
                    ui.painter().circle_filled(pos2(end.right() + 10.0, end.center().y), 4.0, accent());
                    ui.add_space(22.0);

                    // Navigation. Currently a single "Home" pill; kept as a loop
                    // so more entries can be added later.
                    let nav_items = [(crate::icons::Icon::Home, s.home)];
                    for (i, (icon, name)) in nav_items.iter().enumerate() {
                        let is_active =
                            self.selected_playlist_idx.is_none() && !self.show_youtube && i == 0;

                        // Full-width clickable pill.
                        let (rect, response) =
                            ui.allocate_exact_size(vec2(ui.available_width(), 44.0), egui::Sense::click());

                        if is_active {
                            ui.painter().rect_filled(rect, Rounding::same(12.0), surface());
                        } else if response.hovered() {
                            ui.painter().rect_filled(rect, Rounding::same(12.0), surface());
                        }

                        // Bright white when active, muted otherwise.
                        let content_color = if is_active { text() } else { text_muted() };

                        // Icon (inset from the left, vertically centered).
                        let icon_rect = Rect::from_center_size(pos2(rect.min.x + 26.0, rect.center().y), vec2(20.0, 20.0));
                        crate::icons::paint(ui, icon_rect, *icon, content_color);

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
                    // TODO: temporarily hidden — re-enable once the tab is finished.
                    /*
                    {
                        let is_active = self.show_youtube;
                        let (rect, response) = ui
                            .allocate_exact_size(vec2(ui.available_width(), 44.0), egui::Sense::click());

                        if is_active {
                            ui.painter().rect_filled(rect, Rounding::same(12.0), surface());
                        } else if response.hovered() {
                            ui.painter().rect_filled(rect, Rounding::same(12.0), surface());
                        }

                        let content_color = if is_active { text() } else { text_muted() };
                        let icon_pos = pos2(rect.min.x + 16.0, rect.center().y);
                        ui.painter().text(icon_pos, egui::Align2::LEFT_CENTER, "📺", FontId::proportional(18.0), content_color);
                        let text_pos = pos2(rect.min.x + 48.0, rect.center().y);
                        ui.painter().text(text_pos, egui::Align2::LEFT_CENTER, s.yt_tab, FontId::proportional(15.0), content_color);

                        if response.clicked() {
                            self.show_youtube = true;
                            self.show_now_playing = false;
                        }
                    }
                    */

                    ui.add_space(15.0);
                    ui.separator();
                    ui.add_space(15.0);

                    // "New playlist" — a purple accent pill with an add icon + label.
                    let (btn_rect, add_btn) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 44.0), egui::Sense::click());
                    let fill = if add_btn.hovered() {
                        crate::theme::lerp_color(accent(), Color32::WHITE, 0.12)
                    } else {
                        accent()
                    };
                    ui.painter().rect_filled(btn_rect, Rounding::same(22.0), fill);
                    // Center the icon + label as a group.
                    let add_label = s.new_playlist.trim_start_matches(|c: char| !c.is_alphanumeric()).trim();
                    let font = FontId::proportional(15.0);
                    let galley = ui.fonts(|f| f.layout_no_wrap(add_label.to_string(), font, Color32::WHITE));
                    let (icon_sz, gap) = (16.0, 8.0);
                    let total_w = icon_sz + gap + galley.rect.width();
                    let start_x = btn_rect.center().x - total_w / 2.0;
                    let icon_rect = Rect::from_min_size(pos2(start_x, btn_rect.center().y - icon_sz / 2.0), vec2(icon_sz, icon_sz));
                    crate::icons::paint(ui, icon_rect, crate::icons::Icon::Add, Color32::WHITE);
                    ui.painter().galley(
                        pos2(start_x + icon_sz + gap, btn_rect.center().y - galley.rect.height() / 2.0),
                        galley,
                        Color32::WHITE,
                    );
                    if add_btn.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if add_btn.clicked() {
                        self.show_new_playlist = true;
                        self.focus_new_playlist = true;
                        self.new_playlist_name.clear();
                    }

                    ui.add_space(24.0);

                    // "LIBRARY" section header.
                    ui.painter().text(
                        ui.cursor().min + vec2(2.0, 0.0),
                        Align2::LEFT_TOP,
                        library_header(self.language),
                        FontId::proportional(11.0),
                        text_faint(),
                    );
                    ui.add_space(22.0);

                    // Library: Liked music, then user playlists — each an avatar row.
                    let mut clicked: Option<usize> = None;
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        // Liked music (accent avatar with a heart).
                        let liked_sel =
                            self.selected_playlist_idx == Some(LIKED_PAGE_IDX) && !self.show_youtube;
                        if lib_row(ui, None, s.liked_music, s.auto_created, liked_sel, true) {
                            clicked = Some(LIKED_PAGE_IDX);
                        }

                        for (idx, playlist) in self.playlists.iter().enumerate() {
                            if playlist.name == LIKED_PLAYLIST_NAME {
                                continue; // shown above
                            }
                            let sel = self.selected_playlist_idx == Some(idx) && !self.show_youtube;
                            let sub = format!("{} · User", playlist_word(self.language));
                            if lib_row(ui, Some(&playlist.name), &playlist.name, &sub, sel, false) {
                                clicked = Some(idx);
                            }
                        }
                    });
                    if let Some(idx) = clicked {
                        self.selected_playlist_idx = Some(idx);
                        self.search_query.clear();
                        self.show_now_playing = false;
                        self.show_youtube = false;
                    }
                });
            });
    }
}

/// Draws one library row (avatar + name + subtitle) and returns whether it was
/// clicked. `seed`/`name` drive the generated gradient avatar; `is_liked` draws
/// an accent avatar with a heart instead.
fn lib_row(
    ui: &mut egui::Ui,
    seed: Option<&str>,
    name: &str,
    subtitle: &str,
    selected: bool,
    is_liked: bool,
) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 56.0), egui::Sense::click());
    if selected {
        ui.painter().rect_filled(rect, Rounding::same(10.0), surface_2());
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, Rounding::same(10.0), surface());
    }

    // Avatar (rounded gradient square).
    let av = Rect::from_min_size(rect.min + vec2(6.0, 6.0), vec2(44.0, 44.0));
    if is_liked {
        let light = Color32::from_rgb(
            accent().r().saturating_add(30),
            accent().g().saturating_add(30),
            accent().b().saturating_add(30),
        );
        gradient_rrect(ui.painter(), av, 12.0, light, accent());
        crate::icons::paint(ui, Rect::from_center_size(av.center(), vec2(20.0, 20.0)), crate::icons::Icon::Heart, Color32::WHITE);
    } else {
        let (a, b) = gen_gradient(seed.unwrap_or(name));
        gradient_rrect(ui.painter(), av, 12.0, a, b);
        let letter: String = name
            .trim()
            .chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_default();
        ui.painter().text(av.center(), Align2::CENTER_CENTER, letter, FontId::proportional(20.0), Color32::WHITE);
    }

    // Name + subtitle, each truncated to the width left of the avatar so longer
    // translations (uk/ru) can't spill past the row's right edge.
    let tx = av.right() + 12.0;
    let text_w = (rect.right() - tx - 6.0).max(20.0);
    let name_clip = crate::theme::fit_text(ui, name, FontId::proportional(15.0), text_w);
    let sub_clip = crate::theme::fit_text(ui, subtitle, FontId::proportional(12.0), text_w);
    ui.painter().text(pos2(tx, rect.center().y - 9.0), Align2::LEFT_CENTER, name_clip, FontId::proportional(15.0), text());
    ui.painter().text(pos2(tx, rect.center().y + 11.0), Align2::LEFT_CENTER, sub_clip, FontId::proportional(12.0), text_muted());

    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    ui.add_space(6.0);
    resp.clicked()
}

/// Localized "LIBRARY" section header.
fn library_header(lang: Lang) -> &'static str {
    match lang {
        Lang::Ru => "БИБЛИОТЕКА",
        Lang::Uk => "БІБЛІОТЕКА",
        Lang::En => "LIBRARY",
    }
}

/// Localized word "Playlist" used in a row's subtitle.
fn playlist_word(lang: Lang) -> &'static str {
    match lang {
        Lang::Ru => "Плейлист",
        Lang::Uk => "Плейлист",
        Lang::En => "Playlist",
    }
}
