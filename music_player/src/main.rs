mod player;
mod scanner;

use player::Player;
use scanner::{scan_music, Playlist};
use eframe::egui;
use egui::{Color32, FontId, RichText, Rounding, Stroke, Vec2};
use std::time::{Duration, Instant};
use std::collections::HashMap;

// ============================================================
// Вспомогательные функции
// ============================================================

fn format_duration(d: Duration) -> String {
    let total_secs = d.as_secs();
    let mins = total_secs / 60;
    let secs = total_secs % 60;
    format!("{}:{:02}", mins, secs)
}

fn apply_spotify_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    let bg_main = Color32::from_rgb(18, 18, 18);
    let spotify_green = Color32::from_rgb(29, 185, 84);

    visuals.panel_fill = bg_main;
    visuals.window_fill = bg_main;
    visuals.selection.bg_fill = Color32::WHITE;

    visuals.widgets.inactive.bg_fill = Color32::from_rgb(77, 77, 77);
    visuals.widgets.inactive.bg_stroke = Stroke::NONE;
    visuals.widgets.inactive.fg_stroke.color = Color32::from_rgb(179, 179, 179);

    visuals.widgets.hovered.bg_fill = spotify_green;
    visuals.widgets.hovered.fg_stroke.color = Color32::WHITE;

    visuals.widgets.active.bg_fill = spotify_green;
    visuals.widgets.active.fg_stroke.color = Color32::WHITE;

    ctx.set_visuals(visuals);
    // Запрещаем выделение и копирование текста во всём интерфейсе
    ctx.style_mut(|style| {
        style.interaction.selectable_labels = false;
        style.interaction.multi_widget_text_select = false;
    });
}

// ============================================================
// App
// ============================================================

struct TrackMeta {
    title: String,
    artist: Option<String>,
    cover: Option<egui::TextureHandle>,
}

// Читает название, исполнителя и встроенную обложку из ID3-тегов mp3
fn read_track_meta(ctx: &egui::Context, path: &str) -> TrackMeta {
    use id3::TagLike;

    // По умолчанию: название = имя файла без расширения, исполнитель неизвестен
    let fallback_title = std::path::Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "Unknown".to_string());

    let mut title = fallback_title;
    let mut artist: Option<String> = None;
    let mut cover: Option<egui::TextureHandle> = None;

    if let Ok(tag) = id3::Tag::read_from_path(path) {
        if let Some(t) = tag.title() {
            if !t.trim().is_empty() {
                title = t.trim().to_string();
            }
        }
        if let Some(a) = tag.artist() {
            if !a.trim().is_empty() {
                artist = Some(a.trim().to_string());
            }
        }

        // Встроенная обложка (кадр APIC). Берём первую найденную картинку.
        if let Some(pic) = tag.pictures().next() {
            if let Ok(img) = image::load_from_memory(&pic.data) {
                // Карточки маленькие — уменьшаем обложку ради экономии памяти
                let img = img.thumbnail(300, 300);
                let rgba = img.to_rgba8();
                let (w, h) = rgba.dimensions();
                let color = egui::ColorImage::from_rgba_unmultiplied(
                    [w as usize, h as usize],
                    rgba.as_raw(),
                );
                cover = Some(ctx.load_texture(
                    format!("cover:{}", path),
                    color,
                    egui::TextureOptions::LINEAR,
                ));
            }
        }
    }

    TrackMeta { title, artist, cover }
}

struct App {
    playlists: Vec<Playlist>,
    player: Player,
    current_song: String,
    is_playing: bool,
    volume: f32,
    total_duration: Option<Duration>,
    elapsed_duration: Duration,
    last_frame_instant: Instant,
    selected_playlist_idx: Option<usize>,
    track_meta: HashMap<String, TrackMeta>,
}

impl App {
    fn new() -> Self {
        let playlists = scan_music("../DownloadedMusic");
        Self {
            playlists,
            player: Player::new(),
            current_song: String::new(),
            is_playing: false,
            volume: 0.5,
            total_duration: None,
            elapsed_duration: Duration::ZERO,
            last_frame_instant: Instant::now(),
            selected_playlist_idx: None,
            track_meta: HashMap::new(),
        }
    }

    // 🚀 Функция переключения на СЛЕДУЮЩИЙ трек
    fn play_next_track(&mut self) {
        let current_queue = self.get_current_queue();
        if current_queue.is_empty() {
            self.is_playing = false;
            return;
        }

        if let Some(current_idx) = current_queue.iter().position(|s| s == &self.current_song) {
            let next_idx = current_idx + 1;
            if next_idx < current_queue.len() {
                let next_song = current_queue[next_idx].clone();
                self.current_song = next_song.clone();
                self.total_duration = self.player.play(&next_song);
                self.elapsed_duration = Duration::ZERO;
                self.is_playing = true;
            } else {
                // Плейлист закончился
                self.is_playing = false;
                self.elapsed_duration = Duration::ZERO;
            }
        } else {
            // Если текущий трек почему-то не найден, просто включаем самый первый из списка
            let first_song = current_queue[0].clone();
            self.current_song = first_song.clone();
            self.total_duration = self.player.play(&first_song);
            self.elapsed_duration = Duration::ZERO;
            self.is_playing = true;
        }
    }

    // 🚀 Функция возврата на ПРЕДЫДУЩИЙ трек
    fn play_previous_track(&mut self) {
        let current_queue = self.get_current_queue();
        if current_queue.is_empty() { return; }

        if let Some(current_idx) = current_queue.iter().position(|s| s == &self.current_song) {
            if current_idx > 0 {
                let prev_song = current_queue[current_idx - 1].clone();
                self.current_song = prev_song.clone();
                self.total_duration = self.player.play(&prev_song);
                self.elapsed_duration = Duration::ZERO;
                self.is_playing = true;
            } else {
                // Если это первый трек, просто перематываем его в начало
                self.player.seek(&self.current_song, Duration::ZERO);
                self.elapsed_duration = Duration::ZERO;
            }
        }
    }

    // Вспомогательный метод для сборки списка песен, которые сейчас на экране
    fn get_current_queue(&self) -> Vec<String> {
        let mut queue = Vec::new();
        let filtered_playlists: Vec<&Playlist> = match self.selected_playlist_idx {
            Some(idx) => vec![&self.playlists[idx]],
            None => self.playlists.iter().collect(),
        };
        for playlist in filtered_playlists {
            for song in &playlist.songs {
                queue.push(song.clone());
            }
        }
        queue
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        apply_spotify_theme(ctx);
        // Один раз подгружаем метаданные (название, исполнитель, обложка) всех треков
        let total_songs: usize = self.playlists.iter().map(|p| p.songs.len()).sum();
        if self.track_meta.len() < total_songs {
            let all_paths: Vec<String> = self
                .playlists
                .iter()
                .flat_map(|p| p.songs.iter().cloned())
                .collect();
            for path in all_paths {
                if !self.track_meta.contains_key(&path) {
                    let meta = read_track_meta(ctx, &path);
                    self.track_meta.insert(path, meta);
                }
            }
        }

        // --- Логика отсчёта времени ---
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame_instant);
        self.last_frame_instant = now;

        if self.is_playing {
            self.elapsed_duration += dt;
            if let Some(total) = self.total_duration {
                if self.elapsed_duration >= total {
                    self.play_next_track();
                }
            }
        }

        // Общие цвета для использования во всем интерфейсе (панели, карточки):
        let text_muted = Color32::from_rgb(167, 167, 167);
        let spotify_green = Color32::from_rgb(29, 185, 84); // <-- Добавили сюда!

        // =============================================================
        // 🎶 НИЖНЯЯ ПАНЕЛЬ УПРАВЛЕНИЯ
        // =============================================================
        egui::TopBottomPanel::bottom("bottom_bar")
            .resizable(false)
            .min_height(90.0) // Уменьшили высоту панели
            // .max_height(90.0)
            .frame(
                egui::Frame::none()
                    .fill(Color32::from_rgb(0, 0, 0))
                    .inner_margin(16.0),
            )
            .show(ctx, |ui| {
                let total_w = ui.available_width();
                let col_w = total_w / 3.0; 

                ui.horizontal(|ui| {
                    // --- ЛЕВАЯ ЧАСТЬ: Обложка и название ---
                    ui.allocate_ui_with_layout(
                        egui::vec2(col_w, ui.available_height()),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.set_width(col_w);
                            ui.horizontal(|ui| {
                                let meta = self.track_meta.get(&self.current_song);

                                // Обложка текущего трека (56x56): картинка из mp3 или значок ноты
                                let (rect, _) =
                                    ui.allocate_exact_size(egui::vec2(56.0, 56.0), egui::Sense::hover());
                                match meta.and_then(|m| m.cover.as_ref()) {
                                    Some(tex) => {
                                        ui.painter().image(
                                            tex.id(),
                                            rect,
                                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                            Color32::WHITE,
                                        );
                                    }
                                    None => {
                                        ui.painter()
                                            .rect_filled(rect, 4.0, Color32::from_rgb(40, 40, 40));
                                        ui.painter().text(
                                            rect.center(),
                                            egui::Align2::CENTER_CENTER,
                                            "🎵",
                                            FontId::proportional(24.0),
                                            text_muted,
                                        );
                                    }
                                }

                                ui.vertical(|ui| {
                                    ui.add_space(8.0);

                                    // Название и исполнитель из ID3-тегов
                                    let (title, artist) = if self.current_song.is_empty() {
                                        ("No track selected".to_string(), String::new())
                                    } else {
                                        match meta {
                                            Some(m) => (
                                                m.title.clone(),
                                                m.artist.clone().unwrap_or_else(|| "Unknown Artist".to_string()),
                                            ),
                                            None => {
                                                let fname = std::path::Path::new(&self.current_song)
                                                    .file_stem()
                                                    .map(|s| s.to_string_lossy().to_string())
                                                    .unwrap_or_default();
                                                (fname, "Unknown Artist".to_string())
                                            }
                                        }
                                    };

                                    let max_chars = ((col_w / 12.0) as usize).clamp(15, 30);
                                    let display_name = if title.chars().count() > max_chars {
                                        format!("{}...", title.chars().take(max_chars - 3).collect::<String>())
                                    } else {
                                        title.clone()
                                    };

                                    ui.label(
                                        RichText::new(display_name)
                                            .size(14.0)
                                            .strong()
                                            .color(Color32::WHITE),
                                    );
                                    if !artist.is_empty() {
                                        ui.label(RichText::new(artist).size(12.0).color(text_muted));
                                    }
                                });
                            });
                        },
                    );

                    // --- ЦЕНТРАЛЬНАЯ ЧАСТЬ: Кнопки и таймлайн ---
                    ui.allocate_ui_with_layout(
                        egui::vec2(col_w, ui.available_height()),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_width(col_w);

                            // ВЕРХНИЙ ОТСТУП: чем меньше — тем выше кнопки ⏮ ▶ ⏭ от верха панели
                            ui.add_space(8.0);
                            
                            // 1. Идеальное выравнивание по высоте (горизонтальная линия)
                            ui.horizontal(|ui| {
                                ui.add_space(4.0);
                                
                                // Вычисляем общую ширину блока кнопок, чтобы отцентровать их
                                let buttons_width = 190.0;
                                let available_space = ui.available_width();
                                if available_space > buttons_width {
                                    // Пружина слева — толкает кнопки к центру
                                    ui.add_space((available_space - buttons_width) / 2.0);
                                }

                                ui.spacing_mut().item_spacing.x = 18.0;

                                // КНОПКА СЛЕДУЮЩЕГО ТРЕКА (Теперь это кнопка, а не текст!)
                                let prev_btn = ui.add(
                                    egui::Button::new(
                                        RichText::new("⏮")
                                            .size(16.0)
                                            .color(Color32::WHITE),
                                    )
                                    .fill(Color32::from_rgb(30, 30, 30)) // Темный аккуратный фон кнопки
                                    .rounding(100.0)
                                    .min_size(Vec2::new(32.0, 32.0)),
                                );
                                
                                if prev_btn.clicked() && !self.current_song.is_empty() {
                                    self.play_previous_track();
                                }

                                let play_pause_symbol = if self.is_playing { "⏸" } else { "▶" };
                                let play_btn = ui.add(
                                    egui::Button::new(
                                        RichText::new(play_pause_symbol)
                                            .size(18.0)
                                            .color(Color32::BLACK),
                                    )
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

                                // КНОПКА СЛЕДУЮЩЕГО ТРЕКА (Теперь это кнопка, а не текст!)
                                let next_btn = ui.add(
                                    egui::Button::new(
                                        RichText::new("⏭")
                                            .size(16.0)
                                            .color(Color32::WHITE),
                                    )
                                    .fill(Color32::from_rgb(30, 30, 30)) // Темный аккуратный фон кнопки
                                    .rounding(100.0)
                                    .min_size(Vec2::new(32.0, 32.0)),
                                );
                                
                                if next_btn.clicked() && !self.current_song.is_empty() {
                                    self.play_next_track();
                                }
                            });

                            // ui.add_space(12.0);

                            // --- ТАЙМЛАЙН ---
                            ui.add_space(8.0); // Зазор между кнопками и ползунком (положительный = есть воздух)
                            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;

                                let current_time_str = format_duration(self.elapsed_duration);
                                ui.label(RichText::new(current_time_str).size(11.0).color(text_muted));

                                let total_secs_f32 =
                                    self.total_duration.map(|d| d.as_secs_f32()).unwrap_or(0.0);
                                let mut current_secs = self.elapsed_duration.as_secs_f32();

                                ui.style_mut().spacing.slider_width = (col_w - 100.0).max(150.0);

                                if total_secs_f32 > 0.0 {
                                    // Создаем слайдер
                                    let slider = ui.add(
                                        egui::Slider::new(&mut current_secs, 0.0..=total_secs_f32)
                                            .show_value(false)
                                            .trailing_fill(true),
                                    );

                                    if slider.changed() {
                                        self.elapsed_duration = Duration::from_secs_f32(current_secs);
                                    }

                                    if slider.drag_stopped() && !self.current_song.is_empty() {
                                        let new_pos = Duration::from_secs_f32(current_secs);
                                        self.player.seek(&self.current_song, new_pos);
                                        self.elapsed_duration = new_pos;
                                        self.is_playing = true;
                                    }

                                    // Вызываем подсказку в самом конце, когда slider больше не нужен для условий!
                                    slider.on_hover_text_at_pointer(format_duration(
                                        Duration::from_secs_f32(current_secs)
                                    ));
                                } else {
                                    let mut dummy = 0.0;
                                    ui.add_enabled(
                                        false,
                                        egui::Slider::new(&mut dummy, 0.0..=1.0).show_value(false),
                                    );
                                }

                                let total_time_str = self
                                    .total_duration
                                    .map(format_duration)
                                    .unwrap_or_else(|| "0:00".to_string());
                                ui.label(RichText::new(total_time_str).size(11.0).color(text_muted));
                            });
                        },
                    );

                    // --- ПРАВАЯ ЧАСТЬ: Громкость ---
                    ui.allocate_ui_with_layout(
                        egui::vec2(col_w, ui.available_height()),
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            ui.set_width(col_w);
                            ui.add_space(10.0);
                            ui.style_mut().spacing.slider_width = 80.0;
                            let vol_slider = ui.add(
                                egui::Slider::new(&mut self.volume, 0.0..=1.0)
                                    .show_value(false)
                                    .trailing_fill(true),
                            );
                            if vol_slider.changed() {
                                self.player.set_volume(self.volume);
                            }
                            ui.label(RichText::new("").size(14.0).color(text_muted));
                            ui.label(RichText::new("🔊").size(14.0).color(text_muted));
                        },
                    );
                });
            });

        // =============================================================
        // 📁 ЛЕВАЯ ПАНЕЛЬ (Sidebar)
        // =============================================================
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
                    ui.label(
                        RichText::new("Spotify Premium")
                            .size(20.0)
                            .strong()
                            .color(Color32::WHITE),
                    );
                    ui.add_space(25.0);

                    let nav_items = [("🏠", "Home"), ("🔍", "Search"), ("📚", "Your Library")];
                    for (icon, name) in nav_items.iter() {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(*icon).size(18.0));
                            ui.add_space(10.0);
                            ui.label(
                                RichText::new(*name).size(14.0).strong().color(text_muted),
                            );
                        });
                        ui.add_space(12.0);
                    }

                    ui.add_space(15.0);
                    ui.separator();
                    ui.add_space(10.0);

                    let all_tracks_selected = self.selected_playlist_idx.is_none();
                    let all_color =
                        if all_tracks_selected { Color32::WHITE } else { text_muted };
                    if ui
                        .selectable_label(
                            all_tracks_selected,
                            RichText::new("🎵 All Tracks").size(14.0).color(all_color),
                        )
                        .clicked()
                    {
                        self.selected_playlist_idx = None;
                    }
                    ui.add_space(10.0);

                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for (idx, playlist) in self.playlists.iter().enumerate() {
                            let is_selected = self.selected_playlist_idx == Some(idx);
                            let color = if is_selected { Color32::WHITE } else { text_muted };
                            let label =
                                RichText::new(&playlist.name).size(14.0).color(color);

                            if ui.selectable_label(is_selected, label).clicked() {
                                self.selected_playlist_idx = Some(idx);
                            }
                            ui.add_space(4.0);
                        }
                    });
                });
            });

        // =============================================================
        // 🔥 ЦЕНТРАЛЬНАЯ ЧАСТЬ (Музыкальные карточки)
        // =============================================================
        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(Color32::from_rgb(18, 18, 18))
                    .inner_margin(24.0),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Button::new(RichText::new("  <  ").size(16.0))
                            .rounding(15.0)
                            .fill(Color32::from_rgb(10, 10, 10)),
                    );
                    ui.add_space(8.0);
                    ui.add(
                        egui::Button::new(RichText::new("  >  ").size(16.0))
                            .rounding(15.0)
                            .fill(Color32::from_rgb(10, 10, 10)),
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add(
                            egui::Button::new(RichText::new("👤 User").size(13.0))
                                .rounding(15.0)
                                .fill(Color32::from_rgb(10, 10, 10)),
                        );
                    });
                });
                ui.add_space(20.0);

                let title = match self.selected_playlist_idx {
                    Some(idx) => &self.playlists[idx].name,
                    None => "Good evening",
                };
                ui.heading(RichText::new(title).size(30.0).strong().color(Color32::WHITE));
                ui.add_space(20.0);

                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(18.0, 24.0);

                        let filtered_playlists: Vec<&Playlist> = match self.selected_playlist_idx {
                            Some(idx) => vec![&self.playlists[idx]],
                            None => self.playlists.iter().collect(),
                        };

                        for playlist in filtered_playlists {
                            for song in &playlist.songs {
                                let file_name = std::path::Path::new(song)
                                    .file_name()
                                    .unwrap()
                                    .to_string_lossy()
                                    .to_string();

                                // Имя файла без .mp3 — запасной вариант названия
                                let clean_name = file_name.replace(".mp3", "");

                                // Метаданные трека (название, исполнитель, обложка) из ID3
                                let meta = self.track_meta.get(song);

                                let is_active = self.current_song == *song;
                                let card_size = Vec2::new(160.0, 240.0);
                                let (rect, response) =
                                    ui.allocate_exact_size(card_size, egui::Sense::click());
                                let is_hovered = response.hovered();

                                let bg_color = if is_hovered {
                                    Color32::from_rgb(40, 40, 40)
                                } else {
                                    Color32::from_rgb(24, 24, 24)
                                };
                                ui.painter()
                                    .rect_filled(rect, Rounding::same(8.0), bg_color);

                                let cover_size = 132.0;
                                let cover_pos = rect.min + Vec2::new(14.0, 14.0);
                                let cover_rect =
                                    egui::Rect::from_min_size(cover_pos, Vec2::new(cover_size, cover_size));

                                // Подложка под обложку
                                ui.painter().rect_filled(
                                    cover_rect,
                                    Rounding::same(6.0),
                                    Color32::from_rgb(50, 50, 50),
                                );

                                // Есть обложка в mp3 — рисуем её, иначе значок ноты
                                match meta.and_then(|m| m.cover.as_ref()) {
                                    Some(tex) => {
                                        ui.painter().image(
                                            tex.id(),
                                            cover_rect,
                                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                            Color32::WHITE,
                                        );
                                    }
                                    None => {
                                        ui.painter().text(
                                            cover_rect.center(),
                                            egui::Align2::CENTER_CENTER,
                                            "🎵",
                                            FontId::proportional(40.0),
                                            Color32::from_rgb(90, 90, 90),
                                        );
                                    }
                                }

                                let text_pos = cover_rect.left_bottom() + Vec2::new(0.0, 12.0);
                                let text_color =
                                    if is_active { spotify_green } else { Color32::WHITE };

                                // Название: из тега, иначе имя файла
                                let track_title = meta
                                    .map(|m| m.title.clone())
                                    .unwrap_or_else(|| clean_name.clone());
                                let display_name = if track_title.chars().count() > 16 {
                                    format!("{}...", track_title.chars().take(13).collect::<String>())
                                } else {
                                    track_title.clone()
                                };

                                ui.painter().text(
                                    text_pos,
                                    egui::Align2::LEFT_TOP,
                                    display_name,
                                    FontId::proportional(14.0),
                                    text_color,
                                );

                                // Подпись: исполнитель, иначе "Track"
                                let subtitle = meta
                                    .and_then(|m| m.artist.clone())
                                    .unwrap_or_else(|| "Track".to_string());
                                let subtitle = if subtitle.chars().count() > 18 {
                                    format!("{}...", subtitle.chars().take(15).collect::<String>())
                                } else {
                                    subtitle
                                };
                                let subtext_pos = text_pos + Vec2::new(0.0, 18.0);
                                ui.painter().text(
                                    subtext_pos,
                                    egui::Align2::LEFT_TOP,
                                    subtitle,
                                    FontId::proportional(12.0),
                                    text_muted,
                                );

                                if is_hovered || is_active {
                                    let btn_radius = 22.0;
                                    let btn_center =
                                        cover_rect.max - Vec2::new(btn_radius + 4.0, btn_radius + 4.0);

                                    ui.painter().circle_filled(
                                        btn_center + Vec2::new(0.0, 2.0),
                                        btn_radius,
                                        Color32::from_black_alpha(100),
                                    );
                                    ui.painter()
                                        .circle_filled(btn_center, btn_radius, spotify_green);

                                    let icon =
                                        if is_active && self.is_playing { "⏸" } else { "▶" };
                                    ui.painter().text(
                                        btn_center,
                                        egui::Align2::CENTER_CENTER,
                                        icon,
                                        FontId::proportional(20.0),
                                        Color32::BLACK,
                                    );
                                }

                                if response.clicked() {
                                    if is_active {
                                        if self.is_playing {
                                            self.player.pause();
                                            self.is_playing = false;
                                        } else {
                                            self.player.resume();
                                            self.is_playing = true;
                                        }
                                    } else {
                                        self.current_song = song.clone();
                                        self.total_duration = self.player.play(song);
                                        self.elapsed_duration = Duration::ZERO;
                                        self.is_playing = true;
                                    }
                                }
                            }
                        }
                    });
                });
            });

        ctx.request_repaint();
    }
}

fn main() {
    let mut options = eframe::NativeOptions::default();
    options.viewport = egui::ViewportBuilder::default()
        .with_inner_size([1100.0, 750.0])
        .with_min_inner_size([800.0, 600.0]);
    let _ = eframe::run_native(
        "Spotify Desktop",
        options,
        Box::new(|_| Ok(Box::new(App::new()))),
    );
}