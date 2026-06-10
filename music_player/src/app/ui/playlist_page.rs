//! Playlist page: cover/info column on the left, track list on the right.
//!
//! Handles both a normal playlist and the special Liked music page (selected
//! via [`LIKED_PAGE_IDX`]), including the empty-state shown when nothing has
//! been liked yet. The per-track "⋮" menu is drawn as a manual popup after the
//! list so it renders above the rows.

use crate::app::{App, LIKED_PAGE_IDX, LIKED_PLAYLIST_NAME};
use crate::lang::{strings, Lang};
use crate::scanner::Playlist;
use crate::theme::{ACCENT, TEXT_MUTED};
use eframe::egui;
use egui::{pos2, vec2, Color32, FontId, Rect, RichText, Rounding, Stroke};

impl App {
    /// Draws the page for the playlist identified by `idx`.
    pub(in crate::app) fn ui_playlist_page(&mut self, ui: &mut egui::Ui, idx: usize) {
        let s = strings(self.language);

        // Resolve which playlist to show. LIKED_PAGE_IDX is the virtual Liked
        // music page; its tracks live in an ordinary playlist of the same name.
        let playlist: Playlist = if idx == LIKED_PAGE_IDX {
            self.playlists
                .iter()
                .find(|p| p.name == LIKED_PLAYLIST_NAME)
                .cloned()
                .unwrap_or_else(|| Playlist {
                    name: LIKED_PLAYLIST_NAME.to_string(),
                    songs: Vec::new(),
                })
        } else {
            self.playlists[idx].clone()
        };

        // Empty-state for Liked music (nothing liked yet).
        if idx == LIKED_PAGE_IDX && playlist.songs.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() / 3.0);
                ui.label(RichText::new("🤍").size(64.0));
                ui.add_space(20.0);
                ui.label(RichText::new(s.liked_music).size(28.0).strong().color(Color32::WHITE));
                ui.add_space(10.0);
                ui.label(RichText::new(s.liked_empty).size(16.0).color(TEXT_MUTED));
            });
            return;
        }

        let remaining_height = ui.available_height();

        ui.horizontal_top(|ui| {
            // ---- LEFT COLUMN: cover, title, action buttons ----
            ui.allocate_ui_with_layout(
                vec2(240.0, remaining_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.add_space(10.0);

                    // Cover = first track's cover, or a placeholder.
                    let first_meta = playlist.songs.first().and_then(|s| self.track_meta.get(s));
                    let cover_rect = ui.allocate_exact_size(vec2(240.0, 240.0), egui::Sense::hover()).0;
                    ui.painter().rect_filled(cover_rect, Rounding::same(8.0), Color32::from_rgb(40, 40, 40));
                    match first_meta.and_then(|m| m.cover.as_ref()) {
                        Some(tex) => {
                            ui.painter().image(
                                tex.id(),
                                cover_rect,
                                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                                Color32::WHITE,
                            );
                        }
                        None => {
                            ui.painter().text(
                                cover_rect.center(),
                                egui::Align2::CENTER_CENTER,
                                "🎵",
                                FontId::proportional(60.0),
                                Color32::from_rgb(90, 90, 90),
                            );
                        }
                    }

                    ui.add_space(16.0);
                    // The Liked playlist stores its name as the Russian key, so
                    // swap in the localized label for display.
                    let playlist_title: &str = if playlist.name == LIKED_PLAYLIST_NAME {
                        s.liked_music
                    } else {
                        playlist.name.as_str()
                    };
                    ui.label(RichText::new(playlist_title).size(24.0).strong().color(Color32::WHITE));
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(s.playlist_tracks.replace("{n}", &playlist.songs.len().to_string()))
                            .size(13.0)
                            .color(TEXT_MUTED),
                    );
                    ui.add_space(12.0);

                    // Action row: rename (✏), play (▶) and delete (🗑). Rename and
                    // delete are hidden for the Liked music page.
                    ui.horizontal(|ui| {
                        if idx != LIKED_PAGE_IDX && playlist.name != LIKED_PLAYLIST_NAME {
                            let rename_btn = ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("✏").size(16.0).color(Color32::from_rgb(180, 180, 180)),
                                    )
                                    .fill(Color32::from_rgb(45, 45, 45))
                                    .rounding(20.0)
                                    .min_size(vec2(40.0, 36.0)),
                                )
                                .on_hover_text(match self.language {
                                    Lang::Ru => "Переименовать плейлист",
                                    Lang::Uk => "Перейменувати плейлист",
                                    Lang::En => "Rename playlist",
                                });
                            if rename_btn.clicked() {
                                self.rename_playlist_idx = Some(idx);
                                self.rename_playlist_name = playlist.name.clone();
                                self.focus_rename_playlist = true;
                            }
                            ui.add_space(8.0);
                        }

                        // ▶ Play the whole playlist.
                        if ui
                            .add(
                                egui::Button::new(RichText::new(s.play).size(15.0).color(Color32::BLACK))
                                    .fill(ACCENT)
                                    .rounding(20.0)
                                    .min_size(vec2(100.0, 36.0)),
                            )
                            .clicked()
                            && !playlist.songs.is_empty()
                        {
                            self.playback_queue = self.get_current_queue();
                            self.play_track(&playlist.songs[0]);
                        }

                        if idx != LIKED_PAGE_IDX && playlist.name != LIKED_PLAYLIST_NAME {
                            ui.add_space(8.0);
                            let del = ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("🗑").size(16.0).color(Color32::from_rgb(240, 110, 110)),
                                    )
                                    .fill(Color32::from_rgb(45, 45, 45))
                                    .rounding(20.0)
                                    .min_size(vec2(40.0, 36.0)),
                                )
                                .on_hover_text(s.delete_playlist);
                            if del.clicked() {
                                self.delete_playlist(idx);
                            }
                        }
                    });
                },
            );

            ui.add_space(24.0);

            // ---- RIGHT COLUMN: track list ----
            ui.allocate_ui_with_layout(
                vec2(ui.available_width(), remaining_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.label(RichText::new(s.sort).size(13.0).color(TEXT_MUTED));
                    ui.add_space(10.0);

                    // Filter tracks by the search query (title or artist).
                    let query = self.search_query.to_lowercase();
                    let filtered_songs: Vec<&String> = playlist
                        .songs
                        .iter()
                        .filter(|song| {
                            if query.is_empty() {
                                return true;
                            }
                            let meta = self.track_meta.get(*song);
                            let title = meta.map(|m| m.title.to_lowercase()).unwrap_or_default();
                            let artist =
                                meta.and_then(|m| m.artist.clone()).unwrap_or_default().to_lowercase();
                            title.contains(&query) || artist.contains(&query)
                        })
                        .collect();

                    // Virtualized list: only the rows visible in the viewport are
                    // built/drawn, so a playlist with thousands of tracks costs
                    // the same per frame as a small one. Rows are a uniform
                    // height, which is exactly what `show_rows` needs.
                    let row_height = 56.0;
                    egui::ScrollArea::vertical()
                        .id_salt("playlist_tracks_scroll")
                        .auto_shrink([false, false])
                        .max_height(remaining_height - 40.0)
                        .show_rows(ui, row_height, filtered_songs.len(), |ui, range| {
                            for i in range {
                                self.draw_track_row(ui, filtered_songs[i], &s);
                            }
                        });

                    // Track "⋮" popup, drawn after the list so it floats on top.
                    self.draw_track_context_menu(ui);
                },
            );
        });
    }

    /// Draws one track row (cover thumbnail, title/artist, ❤ and ⋮ controls)
    /// and handles its clicks.
    fn draw_track_row(&mut self, ui: &mut egui::Ui, song: &str, s: &crate::lang::Strings) {
        let meta = self.track_meta.get(song);
        let is_active = self.current_song == *song;

        let row_height = 56.0;
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width() - 16.0, row_height), egui::Sense::click());

        let is_hovered = response.hovered();
        if is_hovered {
            ui.painter().rect_filled(rect, Rounding::same(6.0), Color32::from_rgb(40, 40, 40));
        }

        // Cover thumbnail with a play/pause overlay on hover or while active.
        let img_size = 40.0;
        let img_pos = rect.min + vec2(8.0, 8.0);
        let img_rect = Rect::from_min_size(img_pos, vec2(img_size, img_size));
        ui.painter().rect_filled(img_rect, Rounding::same(4.0), Color32::from_rgb(50, 50, 50));
        if let Some(tex) = meta.and_then(|m| m.cover.as_ref()) {
            ui.painter().image(
                tex.id(),
                img_rect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        if is_hovered || (is_active && self.is_playing) {
            ui.painter().rect_filled(img_rect, Rounding::same(4.0), Color32::from_black_alpha(150));
            let icon = if is_active && self.is_playing { "⏸" } else { "▶" };
            ui.painter().text(img_rect.center(), egui::Align2::CENTER_CENTER, icon, FontId::proportional(16.0), ACCENT);
        }

        // Title + artist, truncated to fit.
        let text_color = if is_active { ACCENT } else { Color32::WHITE };
        let title = meta.map(|m| m.title.clone()).unwrap_or_else(|| s.unknown_title.to_string());
        let artist = meta.and_then(|m| m.artist.clone()).unwrap_or_else(|| s.unknown_artist.to_string());
        let max_text_width = rect.width() - img_size - 80.0;
        let max_chars = ((max_text_width / 8.0) as usize).clamp(20, 60);
        let display_title = if title.chars().count() > max_chars {
            format!("{}...", title.chars().take(max_chars - 3).collect::<String>())
        } else {
            title
        };
        let display_artist = if artist.chars().count() > max_chars + 5 {
            format!("{}...", artist.chars().take(max_chars + 2).collect::<String>())
        } else {
            artist
        };
        ui.painter().text(img_rect.right_top() + vec2(16.0, 4.0), egui::Align2::LEFT_TOP, display_title, FontId::proportional(14.0), text_color);
        ui.painter().text(img_rect.right_top() + vec2(16.0, 22.0), egui::Align2::LEFT_TOP, display_artist, FontId::proportional(12.0), TEXT_MUTED);

        // ❤ Like button.
        let track_liked = self.is_liked(song);
        let heart_color = if track_liked { ACCENT } else { Color32::from_rgb(120, 120, 120) };
        let heart_rect = Rect::from_min_size(pos2(rect.right() - 72.0, rect.center().y - 15.0), vec2(30.0, 30.0));
        let mut child_ui = ui.new_child(egui::UiBuilder::new().max_rect(heart_rect));
        let heart_click = child_ui.add(
            egui::Button::new(RichText::new("❤").size(18.0).color(heart_color))
                .fill(Color32::TRANSPARENT)
                .frame(false),
        );
        if heart_click.clicked() {
            self.toggle_like(song);
        }

        // ⋮ Three-dot menu trigger (dots drawn manually so they never render as
        // missing-glyph boxes).
        let dots_rect = Rect::from_min_size(pos2(rect.right() - 36.0, rect.center().y - 15.0), vec2(28.0, 30.0));
        let dots_click = ui.interact(dots_rect, ui.id().with(song), egui::Sense::click());
        let dots_color = if dots_click.hovered() {
            Color32::WHITE
        } else if is_hovered {
            Color32::from_rgb(180, 180, 180)
        } else {
            Color32::TRANSPARENT
        };
        let cx = dots_rect.center().x;
        let cy = dots_rect.center().y;
        for dy in [-5.5_f32, 0.0, 5.5] {
            ui.painter().circle_filled(pos2(cx, cy + dy), 2.2, dots_color);
        }
        if dots_click.clicked() {
            self.track_context_menu = Some(song.to_string());
            self.context_menu_pos = pos2(dots_rect.left() - 172.0, dots_rect.bottom() + 4.0);
            self.context_menu_just_opened = true;
        }

        // Row click = play / toggle (ignored if a control was clicked instead).
        if response.clicked() && !heart_click.clicked() && !dots_click.clicked() {
            if is_active {
                if self.is_playing {
                    self.player.pause();
                    self.is_playing = false;
                } else {
                    self.player.resume();
                    self.is_playing = true;
                }
            } else {
                self.playback_queue = self.get_current_queue();
                self.play_track(song);
            }
        }
    }

    /// Draws the floating "Remove from playlist" popup for the track stored in
    /// [`App::track_context_menu`], if any, and handles its interactions.
    fn draw_track_context_menu(&mut self, ui: &mut egui::Ui) {
        let Some(ctx_song) = self.track_context_menu.clone() else {
            return;
        };

        let popup_rect = Rect::from_min_size(self.context_menu_pos, vec2(180.0, 44.0));

        // Close on a click outside, but skip the very first frame (the same
        // click that opened it would otherwise close it immediately).
        if self.context_menu_just_opened {
            self.context_menu_just_opened = false;
        } else if ui.input(|i| i.pointer.any_click())
            && !popup_rect.contains(ui.input(|i| i.pointer.interact_pos().unwrap_or_default()))
        {
            self.track_context_menu = None;
        }

        let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("track_ctx_menu"));
        let painter = ui.ctx().layer_painter(layer);

        let remove_label = match self.language {
            Lang::Ru => "Удалить из плейлиста   ",
            Lang::Uk => "Видалити з плейлиста   ",
            Lang::En => "Remove from playlist   ",
        };

        let is_menu_hovered =
            ui.input(|i| i.pointer.hover_pos().map(|p| popup_rect.contains(p)).unwrap_or(false));
        let bg_color = if is_menu_hovered { Color32::from_rgb(45, 45, 45) } else { Color32::from_rgb(32, 32, 32) };
        let text_color = if is_menu_hovered { Color32::from_rgb(255, 130, 130) } else { Color32::from_rgb(240, 110, 110) };

        painter.rect_filled(popup_rect, Rounding::same(8.0), bg_color);
        painter.rect_stroke(popup_rect, Rounding::same(8.0), Stroke::new(1.0, Color32::from_rgb(70, 70, 70)));
        painter.text(pos2(popup_rect.min.x + 14.0, popup_rect.center().y), egui::Align2::LEFT_CENTER, "🗑", FontId::proportional(13.0), text_color);
        painter.text(pos2(popup_rect.min.x + 32.0, popup_rect.center().y), egui::Align2::LEFT_CENTER, remove_label, FontId::proportional(13.0), text_color);

        let btn_resp = ui.interact(popup_rect, egui::Id::new("ctx_menu_delete_btn"), egui::Sense::click());
        if btn_resp.clicked() {
            let song_path = ctx_song.clone();
            if let Some(pl_idx) = self.selected_playlist_idx {
                let pl = if pl_idx == LIKED_PAGE_IDX {
                    self.playlists.iter_mut().find(|p| p.name == LIKED_PLAYLIST_NAME)
                } else {
                    self.playlists.get_mut(pl_idx)
                };
                if let Some(pl) = pl {
                    pl.songs.retain(|s| s != &song_path);
                }
                if pl_idx == LIKED_PAGE_IDX {
                    self.save_liked();
                } else {
                    self.save_playlists();
                }
            }
            self.track_context_menu = None;
        }
    }
}
