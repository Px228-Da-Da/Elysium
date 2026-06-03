#![windows_subsystem = "windows"]
mod player;
mod scanner;

use player::Player;
use scanner::{scan_music, Playlist};
use eframe::egui;
use egui::{vec2, Color32, FontId, RichText, Rounding, Stroke, Vec2, Rect, pos2};
use std::time::{Duration, Instant};
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver};
use std::thread;

// Файл, в котором хранится список лайкнутых треков (по одному пути на строку).
// Лежит в рабочей папке — там же, откуда приложение ищет "../DownloadedMusic".
const LIKED_FILE: &str = "liked_songs.txt";

// Читает сохранённые лайки с диска. Если файла нет — возвращает пустой список.
fn load_liked_songs() -> Vec<String> {
    std::fs::read_to_string(LIKED_FILE)
        .map(|content| {
            content
                .lines()
                .map(|line| line.trim().to_string())
                .filter(|line| !line.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

// ============================================================
// Вспомогательные функции
// ============================================================

fn format_duration(d: Duration) -> String {
    let total_secs = d.as_secs();
    let mins = total_secs / 60;
    let secs = total_secs % 60;
    format!("{}:{:02}", mins, secs)
}

// 🎨 Возвращаем ваш оригинальный стиль (Темный фон + Зеленый акцент)
fn apply_custom_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    let bg_main = Color32::from_rgb(18, 18, 18);
    let accent_color = Color32::from_rgb(29, 185, 84);

    visuals.panel_fill = Color32::from_rgb(0, 0, 0);
    visuals.window_fill = bg_main;
    visuals.selection.bg_fill = Color32::WHITE;

    visuals.widgets.inactive.bg_fill = Color32::from_rgb(77, 77, 77);
    visuals.widgets.inactive.bg_stroke = Stroke::NONE;
    visuals.widgets.inactive.fg_stroke.color = Color32::from_rgb(179, 179, 179);

    visuals.widgets.hovered.bg_fill = accent_color;
    visuals.widgets.hovered.fg_stroke.color = Color32::WHITE;

    visuals.widgets.active.bg_fill = accent_color;
    visuals.widgets.active.fg_stroke.color = Color32::WHITE;

    ctx.set_visuals(visuals);
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

fn read_track_meta(ctx: &egui::Context, path: &str) -> TrackMeta {
    use id3::TagLike;

    let fallback_title = std::path::Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "Unknown".to_string());

    let mut title = fallback_title;
    let mut artist: Option<String> = None;
    let mut cover: Option<egui::TextureHandle> = None;

    if let Ok(tag) = id3::Tag::read_from_path(path) {
        if let Some(t) = tag.title() {
            if !t.trim().is_empty() { title = t.trim().to_string(); }
        }
        if let Some(a) = tag.artist() {
            if !a.trim().is_empty() { artist = Some(a.trim().to_string()); }
        }

        if let Some(pic) = tag.pictures().next() {
            if let Ok(img) = image::load_from_memory(&pic.data) {
                let img = img.resize_to_fill(300, 300, image::imageops::FilterType::Lanczos3);
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

// Сообщения от фонового загрузчика к UI-потоку
enum LoaderMsg {
    Playlists(Vec<Playlist>),
    Meta(String, TrackMeta),
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
    loader_rx: Receiver<LoaderMsg>,
    search_query: String, // <--- ДОБАВЛЕНО ТУТ
}

// 1. Помещаем функцию на самый верхний уровень, прямо перед main
fn setup_custom_fonts(ctx: &egui::Context) {
    // Добавили mut, так как код ниже теперь активен и изменяет переменную fonts
    let mut fonts = egui::FontDefinitions::default(); 

    fonts.font_data.insert(
        "emoji_font".to_owned(),
        egui::FontData::from_static(include_bytes!("NotoEmoji-VariableFont_wght.ttf")),
    );
    fonts.families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .push("emoji_font".to_owned());

    ctx.set_fonts(fonts);
}

impl App {
    // Проверяет, есть ли песня в плейлисте
    fn is_liked(&self, song_path: &str) -> bool {
        self.playlists.iter()
            .find(|p| p.name == "Понравившаяся музыка")
            .map(|p| p.songs.contains(&song_path.to_string()))
            .unwrap_or(false)
    }

    // Ставит или убирает лайк
    fn toggle_like(&mut self, song_path: &str) {
        let playlist_name = "Понравившаяся музыка";
        
        if let Some(playlist) = self.playlists.iter_mut().find(|p| p.name == playlist_name) {
            if let Some(pos) = playlist.songs.iter().position(|s| s == song_path) {
                playlist.songs.remove(pos); // Если лайк уже стоит - убираем
            } else {
                playlist.songs.insert(0, song_path.to_string()); // <--- Теперь вставляется в самое начало
            }
        } else {
            // Если плейлиста "Понравившаяся музыка" еще не существует — создаем его
            self.playlists.insert(0, Playlist {
                name: playlist_name.to_string(),
                songs: vec![song_path.to_string()],
            });
            // Вставка в начало сдвинула индексы всех остальных плейлистов на +1.
            // Если сейчас открыт обычный плейлист — поправим его индекс,
            // чтобы под пользователем не «подменилась» открытая страница.
            if let Some(idx) = self.selected_playlist_idx {
                if idx != usize::MAX {
                    self.selected_playlist_idx = Some(idx + 1);
                }
            }
        }

        // Любое изменение лайков сразу пишем на диск, чтобы они пережили перезапуск.
        self.save_liked();
    }

    // Сохраняет треки плейлиста «Понравившаяся музыка» в файл (по одному пути на строку).
    fn save_liked(&self) {
        let songs: Vec<String> = self
            .playlists
            .iter()
            .find(|p| p.name == "Понравившаяся музыка")
            .map(|p| p.songs.clone())
            .unwrap_or_default();

        if let Err(e) = std::fs::write(LIKED_FILE, songs.join("\n")) {
            println!("⚠️ Не вдалося зберегти список лайків: {:?}", e);
        }
    }
    fn new(ctx: &egui::Context) -> Self {
        let (tx, loader_rx) = channel();

        // Сканирование папок и чтение метаданных (теги + обложки) — самая тяжёлая
        // часть запуска. Уносим её в отдельный поток, чтобы окно открылось мгновенно,
        // а не после загрузки всей фонотеки.
        let ctx_clone = ctx.clone();
        thread::spawn(move || {
            let mut playlists = scan_music("../DownloadedMusic");

            // Метаданные грузим только для треков библиотеки. Лайки — это копии тех же
            // треков, поэтому их пути берём ДО добавления плейлиста лайков, чтобы не
            // декодировать одни и те же обложки дважды.
            let all_paths: Vec<String> =
                playlists.iter().flat_map(|p| p.songs.iter().cloned()).collect();

            // Восстанавливаем сохранённый плейлист «Понравившаяся музыка» с диска
            // и кладём его в начало — так же, как это делает кнопка-сердечко.
            let liked = load_liked_songs();
            if !liked.is_empty() {
                playlists.insert(0, Playlist {
                    name: "Понравившаяся музыка".to_string(),
                    songs: liked,
                });
            }

            // Сразу отдаём список плейлистов — карточки появятся в окне без обложек.
            if tx.send(LoaderMsg::Playlists(playlists)).is_err() {
                return; // окно уже закрыли
            }

            // Затем по одной подгружаем обложки/теги — они «доезжают» на ходу.
            for path in all_paths {
                let meta = read_track_meta(&ctx_clone, &path);
                if tx.send(LoaderMsg::Meta(path, meta)).is_err() {
                    break; // окно закрыли — выходим из потока
                }
                ctx_clone.request_repaint(); // будим UI, чтобы показать новую карточку
            }
        });

        Self {
            playlists: Vec::new(),
            player: Player::new(),
            current_song: String::new(),
            is_playing: false,
            volume: 0.5,
            total_duration: None,
            elapsed_duration: Duration::ZERO,
            last_frame_instant: Instant::now(),
            selected_playlist_idx: None,
            track_meta: HashMap::new(),
            loader_rx,
            search_query: String::new(), // <--- ДОБАВЛЕНО ТУТ
        }
    }

    fn play_next_track(&mut self) {
        let current_queue = self.get_current_queue();
        if current_queue.is_empty() {
            self.is_playing = false;
            return;
        }

        if let Some(current_idx) = current_queue.iter().position(|s| s == &self.current_song) {
            let next_idx = current_idx + 1;
            if next_idx < current_queue.len() {
                self.play_track(&current_queue[next_idx].clone());
            } else {
                self.is_playing = false;
                self.elapsed_duration = Duration::ZERO;
            }
        } else {
            self.play_track(&current_queue[0].clone());
        }
    }

    fn play_previous_track(&mut self) {
        let current_queue = self.get_current_queue();
        if current_queue.is_empty() { return; }

        if let Some(current_idx) = current_queue.iter().position(|s| s == &self.current_song) {
            if current_idx > 0 {
                self.play_track(&current_queue[current_idx - 1].clone());
            } else {
                self.player.seek(&self.current_song, Duration::ZERO);
                self.elapsed_duration = Duration::ZERO;
            }
        }
    }

    fn play_track(&mut self, path: &str) {
        self.current_song = path.to_string();
        self.total_duration = self.player.play(path);
        self.elapsed_duration = Duration::ZERO;
        self.is_playing = true;
    }

    fn get_current_queue(&self) -> Vec<String> {
        // usize::MAX — это виртуальная страница «Понравившаяся музыка».
        // Её треки лежат в обычном плейлисте с тем же именем, а не по индексу.
        if self.selected_playlist_idx == Some(usize::MAX) {
            return self
                .playlists
                .iter()
                .find(|p| p.name == "Понравившаяся музыка")
                .map(|p| p.songs.clone())
                .unwrap_or_default();
        }

        let mut queue = Vec::new();
        let filtered_playlists: Vec<&Playlist> = match self.selected_playlist_idx {
            // .get() вместо [idx], чтобы устаревший индекс не «уронил» приложение
            Some(idx) => self.playlists.get(idx).into_iter().collect(),
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
        apply_custom_theme(ctx);
        
        // Забираем всё, что успел подготовить фоновый загрузчик.
        // try_recv не блокирует кадр: берём что готово и сразу рисуем.
        while let Ok(msg) = self.loader_rx.try_recv() {
            match msg {
                LoaderMsg::Playlists(playlists) => self.playlists = playlists,
                LoaderMsg::Meta(path, meta) => {
                    self.track_meta.insert(path, meta);
                }
            }
        }

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

        let text_muted = Color32::from_rgb(167, 167, 167);
        let accent_color = Color32::from_rgb(29, 185, 84);

        // =============================================================
        // 🎶 НИЖНЯЯ ПАНЕЛЬ УПРАВЛЕНИЯ (Возвращен ваш дизайн)
        // =============================================================
        egui::TopBottomPanel::bottom("bottom_bar")
            .resizable(false)
            .min_height(90.0)
            .frame(egui::Frame::none().fill(Color32::from_rgb(0, 0, 0)).inner_margin(16.0))
            .show(ctx, |ui| {
                let total_w = ui.available_width();
                let col_w = total_w / 3.0; 

                ui.horizontal(|ui| {
                    // --- ЛЕВО: Обложка и инфо ---
                    ui.allocate_ui_with_layout(egui::vec2(col_w, ui.available_height()), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.set_width(col_w);
                        let meta = self.track_meta.get(&self.current_song);
                        let (rect, _) = ui.allocate_exact_size(egui::vec2(56.0, 56.0), egui::Sense::hover());
                        
                        match meta.and_then(|m| m.cover.as_ref()) {
                            Some(tex) => {
                                ui.painter().image(tex.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                            }
                            None => {
                                ui.painter().rect_filled(rect, 4.0, Color32::from_rgb(40, 40, 40));
                                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "🎵", FontId::proportional(24.0), text_muted);
                            }
                        }

                        ui.vertical(|ui| {
                            ui.add_space(8.0);
                            let title = meta.map(|m| m.title.clone()).unwrap_or_else(|| "No track selected".to_string());
                            let artist = meta.and_then(|m| m.artist.clone()).unwrap_or_else(|| "Unknown Artist".to_string());

                            let max_chars = ((col_w / 12.0) as usize).clamp(15, 30);
                            let display_name = if title.chars().count() > max_chars {
                                format!("{}...", title.chars().take(max_chars - 3).collect::<String>())
                            } else { title };

                            ui.label(RichText::new(display_name).size(14.0).strong().color(Color32::WHITE));
                            if !artist.is_empty() {
                                ui.label(RichText::new(artist).size(12.0).color(text_muted));
                            }
                        });

                        // ❤ Кнопка-сердечко: сохраняет/убирает текущий трек в плейлист
                        // «Понравившаяся музыка». Зелёное — лайкнуто, серое — нет.
                        if !self.current_song.is_empty() {
                            ui.add_space(12.0);
                            let song = self.current_song.clone();
                            let liked = self.is_liked(&song);
                            let heart_color = if liked { accent_color } else { text_muted };
                            let heart = ui
                                .add(
                                    egui::Button::new(RichText::new("❤").size(20.0).color(heart_color))
                                        .fill(Color32::TRANSPARENT)
                                        .frame(false)
                                        .min_size(Vec2::new(34.0, 34.0)),
                                )
                                .on_hover_text(if liked {
                                    "Убрать из «Понравившаяся музыка»"
                                } else {
                                    "Сохранить в «Понравившаяся музыка»"
                                });
                            if heart.clicked() {
                                self.toggle_like(&song);
                            }
                        }
                    });

                    // --- ЦЕНТР: Управление ---
                    ui.allocate_ui_with_layout(egui::vec2(col_w, ui.available_height()), egui::Layout::top_down(egui::Align::Center), |ui| {
                        ui.set_width(col_w);
                        ui.add_space(8.0);
                        
                        ui.horizontal(|ui| {
                            let buttons_width = 190.0;
                            let available_space = ui.available_width();
                            if available_space > buttons_width {
                                ui.add_space((available_space - buttons_width) / 2.0);
                            }
                            ui.spacing_mut().item_spacing.x = 18.0;
                            
                            let prev_btn = ui.add(egui::Button::new(RichText::new("⏮").size(16.0).color(Color32::WHITE))
                                .fill(Color32::from_rgb(30, 30, 30))
                                .rounding(100.0)
                                .min_size(Vec2::new(32.0, 32.0)));
                            if prev_btn.clicked() { self.play_previous_track(); }

                            let play_icon = if self.is_playing { "⏸" } else { "▶" };
                            let play_btn = ui.add(egui::Button::new(RichText::new(play_icon).size(18.0).color(Color32::BLACK))
                                .fill(Color32::WHITE)
                                .rounding(100.0)
                                .min_size(Vec2::new(32.0, 32.0)));
                            if play_btn.clicked() && !self.current_song.is_empty() {
                                if self.is_playing { self.player.pause(); self.is_playing = false; } 
                                else { self.player.resume(); self.is_playing = true; }
                            }

                            let next_btn = ui.add(egui::Button::new(RichText::new("⏭").size(16.0).color(Color32::WHITE))
                                .fill(Color32::from_rgb(30, 30, 30))
                                .rounding(100.0)
                                .min_size(Vec2::new(32.0, 32.0)));
                            if next_btn.clicked() { self.play_next_track(); }
                        });

                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            ui.label(RichText::new(format_duration(self.elapsed_duration)).size(11.0).color(text_muted));

                            let total_secs_f32 = self.total_duration.map(|d| d.as_secs_f32()).unwrap_or(0.0);
                            let mut current_secs = self.elapsed_duration.as_secs_f32();
                            ui.style_mut().spacing.slider_width = (col_w - 100.0).max(150.0);

                            if total_secs_f32 > 0.0 {
                                let slider = ui.add(egui::Slider::new(&mut current_secs, 0.0..=total_secs_f32).show_value(false).trailing_fill(true));
                                if slider.changed() { self.elapsed_duration = Duration::from_secs_f32(current_secs); }
                                if slider.drag_stopped() && !self.current_song.is_empty() {
                                    let new_pos = Duration::from_secs_f32(current_secs);
                                    self.player.seek(&self.current_song, new_pos);
                                    self.is_playing = true;
                                }
                            } else {
                                let mut dummy = 0.0;
                                ui.add_enabled(false, egui::Slider::new(&mut dummy, 0.0..=1.0).show_value(false));
                            }

                            ui.label(RichText::new(self.total_duration.map(format_duration).unwrap_or_else(|| "0:00".to_string())).size(11.0).color(text_muted));
                        });
                    });

                    // --- ПРАВО: Громкость ---
                    ui.allocate_ui_with_layout(egui::vec2(col_w, ui.available_height()), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.set_width(col_w);
                        ui.add_space(10.0);
                        ui.style_mut().spacing.slider_width = 80.0;
                        if ui.add(egui::Slider::new(&mut self.volume, 0.0..=1.0).show_value(false).trailing_fill(true)).changed() {
                            self.player.set_volume(self.volume);
                        }
                        ui.label(RichText::new("🔊").size(14.0).color(text_muted));
                    });
                });
            });

        // =============================================================
        // 📁 БОКОВАЯ ПАНЕЛЬ
        // =============================================================
        egui::SidePanel::left("sidebar_panel")
            .resizable(false)
            .exact_width(240.0)
            .frame(egui::Frame::none().fill(Color32::from_rgb(0, 0, 0)).inner_margin(20.0))
            .show(ctx, |ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new("Music App").size(20.0).strong().color(Color32::WHITE));
                    ui.add_space(25.0);

                    // Навигация (структура YT, но ваш стиль)
                    // let nav_items = [("🏠", "Главная"), ("🧭", "Навигатор"), ("📚", "Библиотека")];
                    // Навигация (в стиле закругленной плашки, как на фото 2)
                    let nav_items = [("🏠", "Главная")];
                    for (i, (icon, name)) in nav_items.iter().enumerate() {
                        let is_active = self.selected_playlist_idx.is_none() && i == 0;
                        
                        // Выделяем фиксированную область для кнопки во всю ширину сайдбара
                        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), egui::Sense::click());
                        
                        // Отрисовываем фон плашки в зависимости от состояния
                        if is_active {
                            ui.painter().rect_filled(rect, Rounding::same(12.0), Color32::from_rgb(32, 32, 32));
                        } else if response.hovered() {
                            ui.painter().rect_filled(rect, Rounding::same(12.0), Color32::from_rgb(20, 20, 20));
                        }

                        // Цвет контента: ярко-белый для активной, приглушенный для неактивной
                        let content_color = if is_active { Color32::WHITE } else { text_muted };
                        
                        // Рисуем иконку (с отступом 16px слева и идеальным центрированием по вертикали)
                        let icon_pos = pos2(rect.min.x + 16.0, rect.center().y);
                        ui.painter().text(icon_pos, egui::Align2::LEFT_CENTER, *icon, FontId::proportional(18.0), content_color);
                        
                        // Рисуем текст рядом
                        let text_pos = pos2(rect.min.x + 48.0, rect.center().y);
                        ui.painter().text(text_pos, egui::Align2::LEFT_CENTER, *name, FontId::proportional(15.0), content_color);

                        if response.clicked() {
                            if i == 0 { self.selected_playlist_idx = None; }
                        }
                        ui.add_space(12.0);
                    }

                    ui.add_space(15.0);
                    ui.separator();
                    ui.add_space(15.0);

                    // Кнопка Новый (растянутая на всю доступную ширину)
                    ui.add_sized(
                        [ui.available_width(), 40.0],
                        egui::Button::new(RichText::new("➕ Новый").size(16.0).strong())
                            .fill(Color32::from_rgb(30, 30, 30))
                            .rounding(20.0)
                    );
                    
                    ui.add_space(25.0);

                    // Кликабельная секция: Понравившаяся музыка
                    let is_liked_selected = self.selected_playlist_idx == Some(usize::MAX);
                    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 50.0), egui::Sense::click());
                    
                    // Эффекты выделения и наведения (как у плейлистов)
                    if is_liked_selected {
                        ui.painter().rect_filled(rect, Rounding::same(6.0), Color32::from_rgb(40, 40, 40));
                    } else if response.hovered() {
                        ui.painter().rect_filled(rect, Rounding::same(6.0), Color32::from_rgb(30, 30, 30));
                    }

                    // Текст
                    let text_pos = rect.min + vec2(10.0, 8.0);
                    ui.painter().text(text_pos, egui::Align2::LEFT_TOP, "Понравившаяся музыка", FontId::proportional(15.0), Color32::WHITE);
                    ui.painter().text(text_pos + vec2(0.0, 20.0), egui::Align2::LEFT_TOP, "📌 Создан автоматически", FontId::proportional(12.0), text_muted);

                    // Обработка клика (используем usize::MAX как специальный ID для этой страницы)
                    if response.clicked() {
                        self.selected_playlist_idx = Some(usize::MAX);
                    }

                    ui.add_space(20.0);

                    // Список обычных плейлистов
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        // ... здесь остается ваш цикл for (idx, playlist) ...
                        for (idx, playlist) in self.playlists.iter().enumerate() {
                            // «Понравившаяся музыка» уже есть отдельной кнопкой выше — не дублируем
                            if playlist.name == "Понравившаяся музыка" { continue; }
                            let is_selected = self.selected_playlist_idx == Some(idx);
                            
                            // Создаем контейнер для плейлиста, похожий на карточку со скриншота
                            let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 50.0), egui::Sense::click());
                            
                            if is_selected {
                                ui.painter().rect_filled(rect, Rounding::same(6.0), Color32::from_rgb(40, 40, 40));
                            } else if response.hovered() {
                                ui.painter().rect_filled(rect, Rounding::same(6.0), Color32::from_rgb(30, 30, 30));
                            }

                            // Текст плейлиста
                            let text_pos = rect.min + vec2(10.0, 8.0);
                            ui.painter().text(text_pos, egui::Align2::LEFT_TOP, &playlist.name, FontId::proportional(15.0), Color32::WHITE);
                            ui.painter().text(text_pos + vec2(0.0, 20.0), egui::Align2::LEFT_TOP, "User", FontId::proportional(12.0), text_muted);

                            if response.clicked() {
                                self.selected_playlist_idx = Some(idx);
                            }
                            ui.add_space(4.0);
                        }
                    });
                });
            });

        // =============================================================
        // 🔥 ЦЕНТРАЛЬНАЯ ПАНЕЛЬ
        // =============================================================
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(Color32::from_rgb(18, 18, 18)).inner_margin(24.0))
            .show(ctx, |ui| {
                
                // Верхний блок: стрелочки и поиск
                ui.horizontal(|ui| {
                    ui.add(egui::Button::new(RichText::new("  <  ").size(16.0)).rounding(15.0).fill(Color32::from_rgb(10, 10, 10)));
                    ui.add_space(8.0);
                    ui.add(egui::Button::new(RichText::new("  >  ").size(16.0)).rounding(15.0).fill(Color32::from_rgb(10, 10, 10)));
                    ui.add_space(16.0);
                    
                    // Настоящий поиск
                    let search_rect = ui.allocate_exact_size(egui::vec2(400.0, 32.0), egui::Sense::hover()).0;
                    ui.painter().rect_filled(search_rect, Rounding::same(16.0), Color32::from_rgb(30, 30, 30));
                    
                    // Создаем дочерний UI поверх нарисованного фона, чтобы разместить иконку и поле ввода
                    let mut search_ui = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(search_rect.shrink(8.0))
                            .layout(egui::Layout::left_to_right(egui::Align::Center))
                    );
                    search_ui.add_space(8.0);
                    search_ui.label(RichText::new("🔍").size(14.0).color(text_muted));
                    
                    // 1. Сохраняем поле ввода в переменную `response`
                    let response = search_ui.add(
                        egui::TextEdit::singleline(&mut self.search_query)
                            .frame(false) // Убираем стандартную рамку
                            .hint_text(RichText::new("Поиск треков, артистов...").color(text_muted))
                            .text_color(Color32::WHITE)
                            .desired_width(340.0)
                    );

                    // 2. ДОБАВЛЕНО ТУТ: Если нажали на поиск или начали вводить текст — перекидываем на Главную
                    if response.gained_focus() || response.changed() {
                        self.selected_playlist_idx = None;
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add(egui::Button::new(RichText::new("👤 User").size(13.0)).rounding(15.0).fill(Color32::from_rgb(10, 10, 10)));
                    });
                });
                ui.add_space(20.0);

                if let Some(idx) = self.selected_playlist_idx {
                    // Определяем, какой плейлист показывать. usize::MAX — это виртуальная
                    // страница «Понравившаяся музыка»; её треки хранятся в обычном
                    // плейлисте с тем же именем (его наполняет кнопка-сердечко).
                    let playlist: Playlist = if idx == usize::MAX {
                        self.playlists
                            .iter()
                            .find(|p| p.name == "Понравившаяся музыка")
                            .cloned()
                            .unwrap_or_else(|| Playlist {
                                name: "Понравившаяся музыка".to_string(),
                                songs: Vec::new(),
                            })
                    } else {
                        self.playlists[idx].clone()
                    };

                    if idx == usize::MAX && playlist.songs.is_empty() {
                        // -------------------------------------------------------------
                        // ❤️ "ПОНРАВИВШАЯСЯ МУЗЫКА" — пустое состояние (лайков ещё нет)
                        // -------------------------------------------------------------
                        ui.vertical_centered(|ui| {
                            ui.add_space(ui.available_height() / 3.0);
                            
                            ui.label(RichText::new("🤍").size(64.0));
                            ui.add_space(20.0);
                            ui.label(RichText::new("Понравившаяся музыка").size(28.0).strong().color(Color32::WHITE));
                            ui.add_space(10.0);
                            ui.label(RichText::new("Здесь пока пусто. Треки, которые вы лайкнете, появятся тут.").size(16.0).color(text_muted));
                        });
                    } else {
                        // -------------------------------------------------------------
                        // 📄 СТРАНИЦА ПЛЕЙЛИСТА
                        // -------------------------------------------------------------
                        let remaining_height = ui.available_height();
                        
                        ui.horizontal_top(|ui| {
                            // ЛЕВАЯ КОЛОНКА (Обложка и инфо)
                            ui.allocate_ui_with_layout(
                                vec2(240.0, remaining_height),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    ui.add_space(10.0);

                                    let first_meta = playlist.songs.first().and_then(|s| self.track_meta.get(s));
                                    let cover_rect = ui.allocate_exact_size(vec2(240.0, 240.0), egui::Sense::hover()).0;
                                    
                                    ui.painter().rect_filled(cover_rect, Rounding::same(8.0), Color32::from_rgb(40, 40, 40));
                                    match first_meta.and_then(|m| m.cover.as_ref()) {
                                        Some(tex) => { ui.painter().image(tex.id(), cover_rect, Rect::from_min_max(pos2(0.0,0.0), pos2(1.0,1.0)), Color32::WHITE); }
                                        None => { ui.painter().text(cover_rect.center(), egui::Align2::CENTER_CENTER, "🎵", FontId::proportional(60.0), Color32::from_rgb(90, 90, 90)); }
                                    }

                                    ui.add_space(16.0);
                                    ui.label(RichText::new(&playlist.name).size(24.0).strong().color(Color32::WHITE));
                                    ui.add_space(4.0);
                                    ui.label(RichText::new(format!("Плейлист • {} треков", playlist.songs.len())).size(13.0).color(text_muted));
                                    ui.add_space(12.0);
                                    
                                    if ui.add(egui::Button::new(RichText::new("   ▶  Play   ").size(15.0).color(Color32::BLACK))
                                        .fill(accent_color)
                                        .rounding(20.0)
                                        .min_size(vec2(100.0, 36.0))).clicked() {
                                        if !playlist.songs.is_empty() {
                                            self.play_track(&playlist.songs[0]);
                                        }
                                    }
                                }
                            );

                            ui.add_space(24.0);

                            // ПРАВАЯ КОЛОНКА (Список треков)
                            ui.allocate_ui_with_layout(
                                vec2(ui.available_width(), remaining_height),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    ui.label(RichText::new("Упорядочить").size(13.0).color(text_muted));
                                    ui.add_space(10.0);

                                    // --- НАЧАЛО: Подготовка отфильтрованного списка ---
                                    let query = self.search_query.to_lowercase();
                                    let filtered_songs: Vec<&String> = playlist.songs.iter().filter(|song| {
                                        if query.is_empty() { return true; }
                                        let meta = self.track_meta.get(*song);
                                        let title = meta.map(|m| m.title.to_lowercase()).unwrap_or_default();
                                        let artist = meta.and_then(|m| m.artist.clone()).unwrap_or_default().to_lowercase();
                                        title.contains(&query) || artist.contains(&query)
                                    }).collect();
                                    // --- КОНЕЦ ---

                                    egui::ScrollArea::vertical()
                                        .id_salt("playlist_tracks_scroll")
                                        .auto_shrink([false, false]) 
                                        .max_height(remaining_height - 40.0) 
                                        .show(ui, |ui| {
                                            // ВАЖНО: здесь теперь filtered_songs вместо &playlist.songs
                                            for song in filtered_songs {
                                                let meta = self.track_meta.get(song);
                                                let is_active = self.current_song == *song;
                                                
                                                let row_height = 56.0;
                                                // Выделяем всю ширину строки
                                                let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width() - 16.0, row_height), egui::Sense::click());
                                                
                                                let is_hovered = response.hovered();
                                                if is_hovered {
                                                    ui.painter().rect_filled(rect, Rounding::same(6.0), Color32::from_rgb(40, 40, 40));
                                                }

                                                // Рендеринг обложки
                                                let img_size = 40.0;
                                                let img_pos = rect.min + vec2(8.0, 8.0);
                                                let img_rect = Rect::from_min_size(img_pos, vec2(img_size, img_size));
                                                
                                                ui.painter().rect_filled(img_rect, Rounding::same(4.0), Color32::from_rgb(50, 50, 50));
                                                if let Some(tex) = meta.and_then(|m| m.cover.as_ref()) {
                                                    ui.painter().image(tex.id(), img_rect, Rect::from_min_max(pos2(0.0,0.0), pos2(1.0,1.0)), Color32::WHITE);
                                                }

                                                if is_hovered || (is_active && self.is_playing) {
                                                    ui.painter().rect_filled(img_rect, Rounding::same(4.0), Color32::from_black_alpha(150));
                                                    let icon = if is_active && self.is_playing { "⏸" } else { "▶" };
                                                    ui.painter().text(img_rect.center(), egui::Align2::CENTER_CENTER, icon, FontId::proportional(16.0), accent_color);
                                                }

                                                // --- Ограничение текста и кнопка лайка ---
                                                let text_color = if is_active { accent_color } else { Color32::WHITE };
                                                let title = meta.map(|m| m.title.clone()).unwrap_or_else(|| "Unknown".to_string());
                                                let artist = meta.and_then(|m| m.artist.clone()).unwrap_or_else(|| "Unknown Artist".to_string());

                                                // Вычисляем доступную ширину для текста (минусуем обложку слева и кнопку лайка справа)
                                                let max_text_width = rect.width() - img_size - 80.0;
                                                let max_chars = ((max_text_width / 8.0) as usize).clamp(20, 60);

                                                let display_title = if title.chars().count() > max_chars {
                                                    format!("{}...", title.chars().take(max_chars - 3).collect::<String>())
                                                } else { title };

                                                let display_artist = if artist.chars().count() > max_chars + 5 {
                                                    format!("{}...", artist.chars().take(max_chars + 2).collect::<String>())
                                                } else { artist };

                                                ui.painter().text(img_rect.right_top() + vec2(16.0, 4.0), egui::Align2::LEFT_TOP, display_title, FontId::proportional(14.0), text_color);
                                                ui.painter().text(img_rect.right_top() + vec2(16.0, 22.0), egui::Align2::LEFT_TOP, display_artist, FontId::proportional(12.0), text_muted);

                                                // Интерактивная кнопка-лайк (вместо кривого текста)
                                                let track_liked = self.is_liked(song);
                                                let heart_color = if track_liked { accent_color } else { Color32::from_rgb(120, 120, 120) };
                                                
                                                // Создаем область для кнопки в правой части строки
                                                let heart_btn_size = vec2(30.0, 30.0);
                                                let heart_btn_pos = pos2(rect.right() - 45.0, rect.center().y - 15.0);
                                                let heart_rect = Rect::from_min_size(heart_btn_pos, heart_btn_size);
                                                
                                                // Рисуем кнопку лайка поверх строки
                                                let mut child_ui = ui.new_child(egui::UiBuilder::new().max_rect(heart_rect));
                                                let heart_click = child_ui.add(
                                                    egui::Button::new(RichText::new("♥").size(18.0).color(heart_color))
                                                        .fill(Color32::TRANSPARENT)
                                                        .frame(false)
                                                );

                                                if heart_click.clicked() {
                                                    self.toggle_like(song);
                                                }

                                                // Клик по самой строке (играть трек) срабатывает только если не нажали на лайк
                                                if response.clicked() && !heart_click.clicked() {
                                                    if is_active {
                                                        if self.is_playing { self.player.pause(); self.is_playing = false; }
                                                        else { self.player.resume(); self.is_playing = true; }
                                                    } else {
                                                        self.play_track(song);
                                                    }
                                                }
                                            }
                                        });
                                }
                            );
                        });
                    }
                } else {
                    // -------------------------------------------------------------
                    // 🏠 ГЛАВНАЯ СТРАНИЦА (Структура YT Music, Ваш стиль карточек)
                    // -------------------------------------------------------------
                    
                    // 1. Оборачиваем ВСЮ главную страницу в один общий вертикальный скролл
                    egui::ScrollArea::vertical()
                        .id_salt("main_page_vertical_scroll") // Обязательно даем уникальный ID
                        .show(ui, |ui| {
                            
                            // 2. Чипсы (категории) - даем им свой ID, чтобы не было конфликтов
                            egui::ScrollArea::horizontal()
                                .id_salt("chips_horizontal_scroll")
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        let chips = ["пинаю хуй"];
                                        for chip in chips {
                                            ui.add(egui::Button::new(RichText::new(chip).size(13.0).color(Color32::WHITE))
                                                .fill(Color32::from_rgb(30, 30, 30))
                                                .rounding(16.0));
                                            ui.add_space(4.0);
                                        }
                                    });
                                });
                            ui.add_space(30.0);

                            // Заголовок секции
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.label(RichText::new("Послушать ещё раз").size(26.0).strong().color(Color32::WHITE));
                                });
                            });
                            ui.add_space(20.0);

                            // 3. Сетка всех треков (ВАШ ОРИГИНАЛЬНЫЙ ДИЗАЙН КАРТОЧЕК)
                            // Мы УБРАЛИ отсюда второй ScrollArea::vertical(), так как он теперь общий для всей страницы
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing = egui::vec2(18.0, 24.0);

                                // «Понравившаяся музыка» — это копии треков, которые уже
                                // есть в обычных плейлистах. Пропускаем её здесь, иначе
                                // лайкнутые песни задваивались бы на главной.
                                let query = self.search_query.to_lowercase(); // <-- Берем текст из поиска
                                let all_songs: Vec<String> = self
                                    .playlists
                                    .iter()
                                    .filter(|p| p.name != "Понравившаяся музыка")
                                    .flat_map(|p| p.songs.clone())
                                    // --- НАЧАЛО НОВОГО ФИЛЬТРА ---
                                    .filter(|song| {
                                        if query.is_empty() { return true; }
                                        let meta = self.track_meta.get(song);
                                        let title = meta.map(|m| m.title.to_lowercase()).unwrap_or_default();
                                        let artist = meta.and_then(|m| m.artist.clone()).unwrap_or_default().to_lowercase();
                                        title.contains(&query) || artist.contains(&query)
                                    })
                                    // --- КОНЕЦ НОВОГО ФИЛЬТРА ---
                                    .collect();
                                    
                                for song in all_songs {
                                    let meta = self.track_meta.get(&song);
                                    let is_active = self.current_song == song;
                                    
                                    // Фиксированный размер карточки
                                    let card_size = Vec2::new(160.0, 240.0);
                                    let (rect, response) = ui.allocate_exact_size(card_size, egui::Sense::click());
                                    let is_hovered = response.hovered();

                                    let bg_color = if is_hovered {
                                        Color32::from_rgb(40, 40, 40)
                                    } else {
                                        Color32::from_rgb(24, 24, 24)
                                    };
                                    ui.painter().rect_filled(rect, Rounding::same(8.0), bg_color);

                                    // Размеры и позиция обложки внутри карточки
                                    let cover_size = 132.0;
                                    let cover_pos = rect.min + Vec2::new(14.0, 14.0);
                                    let cover_rect = egui::Rect::from_min_size(cover_pos, Vec2::new(cover_size, cover_size));

                                    ui.painter().rect_filled(
                                        cover_rect,
                                        Rounding::same(6.0),
                                        Color32::from_rgb(50, 50, 50),
                                    );

                                    if let Some(tex) = meta.and_then(|m| m.cover.as_ref()) {
                                        ui.painter().image(tex.id(), cover_rect, Rect::from_min_max(pos2(0.0,0.0), pos2(1.0,1.0)), Color32::WHITE);
                                    } else {
                                        ui.painter().text(cover_rect.center(), egui::Align2::CENTER_CENTER, "🎵", FontId::proportional(40.0), Color32::from_rgb(90, 90, 90));
                                    }

                                    // Позиция для текста под обложкой
                                    let text_pos = cover_rect.left_bottom() + Vec2::new(0.0, 12.0);
                                    let text_color = if is_active { accent_color } else { Color32::WHITE };

                                    // --- ИСПРАВЛЕНИЕ: Автоматическое ограничение длины текста по ширине карточки ---
                                    // Доступная ширина для текста: 132px (по ширине обложки). 
                                    // В пропорциональном шрифте один символ занимает в среднем 7-8px.
                                    let max_chars_title = 15;
                                    let max_chars_artist = 18;

                                    let title = meta.map(|m| m.title.clone()).unwrap_or_else(|| "Unknown".to_string());
                                    let display_name = if title.chars().count() > max_chars_title {
                                        format!("{}...", title.chars().take(max_chars_title - 3).collect::<String>())
                                    } else { title };

                                    ui.painter().text(text_pos, egui::Align2::LEFT_TOP, display_name, FontId::proportional(14.0), text_color);

                                    let artist = meta.and_then(|m| m.artist.clone()).unwrap_or_else(|| "Track".to_string());
                                    let subtitle = if artist.chars().count() > max_chars_artist {
                                        format!("{}...", artist.chars().take(max_chars_artist - 3).collect::<String>())
                                    } else { artist };
                                    
                                    let subtext_pos = text_pos + Vec2::new(0.0, 18.0);
                                    ui.painter().text(subtext_pos, egui::Align2::LEFT_TOP, subtitle, FontId::proportional(12.0), text_muted);

                                    // --- ИСПРАВЛЕНИЕ: Интерактивное и ровное векторное сердечко-кнопка ---
                                    let liked = self.is_liked(&song);
                                    let heart_color = if liked { accent_color } else { Color32::from_rgb(100, 100, 100) };

                                    // Выделяем аккуратную квадратную область в правом нижнем углу карточки
                                    let heart_btn_size = vec2(28.0, 28.0);
                                    let heart_btn_pos = pos2(rect.right() - 36.0, rect.bottom() - 36.0);
                                    let heart_rect = Rect::from_min_size(heart_btn_pos, heart_btn_size);

                                    // Рисуем невидимый дочерний UI поверх этой области и вставляем туда кнопку с символом "♥"
                                    let mut heart_ui = ui.new_child(egui::UiBuilder::new().max_rect(heart_rect));
                                    let heart_click = heart_ui.add(
                                        egui::Button::new(RichText::new("♥").size(16.0).color(heart_color))
                                            .fill(Color32::TRANSPARENT)
                                            .frame(false)
                                    );

                                    // Обработка клика по сердечку карточки
                                    if heart_click.clicked() {
                                        self.toggle_like(&song);
                                    }

                                    // Появление большой кнопки Play (круглый зеленый кружок) при наведении на карточку
                                    if is_hovered || is_active {
                                        let btn_radius = 22.0;
                                        let btn_center = cover_rect.max - Vec2::new(btn_radius + 4.0, btn_radius + 4.0);

                                        ui.painter().circle_filled(btn_center + Vec2::new(0.0, 2.0), btn_radius, Color32::from_black_alpha(100));
                                        ui.painter().circle_filled(btn_center, btn_radius, accent_color);

                                        let icon = if is_active && self.is_playing { "⏸" } else { "▶" };
                                        ui.painter().text(btn_center, egui::Align2::CENTER_CENTER, icon, FontId::proportional(20.0), Color32::BLACK);
                                    }

                                    // Клик по карточке включает трек, только если кликнули МИМО кнопки лайка
                                    if response.clicked() && !heart_click.clicked() {
                                        if is_active {
                                            if self.is_playing { self.player.pause(); self.is_playing = false; }
                                            else { self.player.resume(); self.is_playing = true; }
                                        } else {
                                            self.play_track(&song);
                                        }
                                    }
                                }
                            });
                        });
                }
            });

        ctx.request_repaint();
    }
}

fn main() {
    let mut options = eframe::NativeOptions::default();
    options.viewport = egui::ViewportBuilder::default()
        .with_inner_size([1200.0, 800.0])
        .with_min_inner_size([900.0, 600.0]);
        
    let _ = eframe::run_native(
        "Music Desktop",
        options,
        Box::new(|cc| {
            // ВАЖНО: Применяем шрифты до создания самого приложения!
            setup_custom_fonts(&cc.egui_ctx); 
            Ok(Box::new(App::new(&cc.egui_ctx)))
        }),
    );
}