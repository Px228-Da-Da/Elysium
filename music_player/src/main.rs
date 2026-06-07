#![windows_subsystem = "windows"]
mod player;
mod scanner;

use player::Player;
use scanner::{scan_music, Playlist, LyricLine};
use eframe::egui;
use egui::{vec2, Color32, FontId, RichText, Rounding, Stroke, Vec2, Rect, pos2};
use std::time::{Duration, Instant};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;

#[derive(Default, Clone)]
struct UpdateState {
    available: bool,
    latest_version: String,
    checking: bool,
    updating: bool,         // Идёт ли скачивание прямо сейчас
    error: Option<String>,  // Текст ошибки, если что-то пошло не так
}

// ============================================================
// Хранилище: всё состояние в одном config.json в пользовательской
// папке (%APPDATA%\Elysium на Windows, ~/.config/Elysium на Linux).
// ============================================================

#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct Config {
    liked: Vec<String>,                   // пути лайкнутых треков
    playlists: Vec<PlaylistData>,         // обычные плейлисты
    deleted_playlists: Vec<String>,       // имена удалённых плейлистов
    language: String,                     // код языка: "ru" / "uk"
    shortcuts: HashMap<String, String>,   // код действия -> имя клавиши ("None" если снято)
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct PlaylistData {
    name: String,
    songs: Vec<String>,
}

// Путь к config.json (папка создаётся, если её ещё нет).
fn config_path() -> std::path::PathBuf {
    let mut dir = dirs::config_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    dir.push("Elysium");
    let _ = std::fs::create_dir_all(&dir);
    dir.push("config.json");
    dir
}

// Читает весь конфиг. Если файла нет — пробует разово перенести старые .txt.
fn load_config() -> Config {
    if let Ok(text) = std::fs::read_to_string(config_path()) {
        return serde_json::from_str(&text).unwrap_or_default();
    }
    if let Some(cfg) = migrate_from_txt() {
        save_config(&cfg);
        return cfg;
    }
    Config::default()
}

// Записывает весь конфиг читаемым JSON.
fn save_config(cfg: &Config) {
    match serde_json::to_string_pretty(cfg) {
        Ok(text) => {
            if let Err(e) = std::fs::write(config_path(), text) {
                println!("⚠️ Не удалось сохранить конфиг: {:?}", e);
            }
        }
        Err(e) => println!("⚠️ Не удалось сериализовать конфиг: {:?}", e),
    }
}

// Разовый перенос данных из старых .txt (если они лежат в рабочей папке).
fn migrate_from_txt() -> Option<Config> {
    let mut cfg = Config::default();
    let mut found = false;

    if let Ok(c) = std::fs::read_to_string("liked_songs.txt") {
        cfg.liked = c.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
        found = true;
    }
    if let Ok(c) = std::fs::read_to_string("playlists.txt") {
        let mut order: Vec<String> = Vec::new();
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        for line in c.lines() {
            if let Some((name, path)) = line.split_once('\t') {
                let (name, path) = (name.trim(), path.trim());
                if name.is_empty() { continue; }
                if !map.contains_key(name) { order.push(name.to_string()); }
                let e = map.entry(name.to_string()).or_default();
                if !path.is_empty() { e.push(path.to_string()); }
            }
        }
        cfg.playlists = order.into_iter()
            .map(|name| { let songs = map.remove(&name).unwrap_or_default(); PlaylistData { name, songs } })
            .collect();
        found = true;
    }
    if let Ok(c) = std::fs::read_to_string("deleted_playlists.txt") {
        cfg.deleted_playlists = c.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
        found = true;
    }
    if let Ok(c) = std::fs::read_to_string("language.txt") {
        cfg.language = c.trim().to_string();
        found = true;
    }
    if let Ok(c) = std::fs::read_to_string("shortcuts.txt") {
        for line in c.lines() {
            if let Some((code, value)) = line.split_once('=') {
                cfg.shortcuts.insert(code.trim().to_string(), value.trim().to_string());
            }
        }
        found = true;
    }

    if found {
        println!("📦 Старые .txt перенесены в config.json — их можно удалить.");
        Some(cfg)
    } else {
        None
    }
}

// ---- Тонкие обёртки: места вызова остаются прежними ----

fn load_liked_songs() -> Vec<String> {
    load_config().liked
}

fn load_saved_playlists() -> Vec<(String, Vec<String>)> {
    load_config().playlists.into_iter().map(|p| (p.name, p.songs)).collect()
}

fn load_deleted_playlists() -> HashSet<String> {
    load_config().deleted_playlists.into_iter().collect()
}

fn save_deleted_playlists(set: &HashSet<String>) {
    let mut names: Vec<String> = set.iter().cloned().collect();
    names.sort();
    let mut cfg = load_config();
    cfg.deleted_playlists = names;
    save_config(&cfg);
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

// Единый «плоский» стиль для пунктов меню (и выпадающего, и по правому клику):
// без рамки, прозрачный фон в покое, мягкая серая подсветка при наведении.
fn style_menu(ui: &mut egui::Ui) {
    let v = ui.visuals_mut();
    v.widgets.inactive.bg_stroke = Stroke::NONE;
    v.widgets.hovered.bg_stroke  = Stroke::NONE;
    v.widgets.active.bg_stroke   = Stroke::NONE;

    v.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    v.widgets.inactive.bg_fill      = Color32::TRANSPARENT;

    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(48, 48, 48);
    v.widgets.hovered.bg_fill      = Color32::from_rgb(48, 48, 48);
    v.widgets.active.weak_bg_fill  = Color32::from_rgb(60, 60, 60);
    v.widgets.active.bg_fill       = Color32::from_rgb(60, 60, 60);

    v.widgets.hovered.fg_stroke.color = Color32::WHITE;
    v.widgets.active.fg_stroke.color  = Color32::WHITE;
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

/// Приводит текст к форме NFC: склеивает базовый символ с комбинирующим
/// значком в один (например "и" + бреве -> "й"), чтобы шрифт его нарисовал.
fn nfc(s: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    s.nfc().collect()
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

    let title = nfc(&title);
    let artist = artist.map(|a| nfc(&a));

    TrackMeta { title, artist, cover }
}

// Сообщения от фонового загрузчика к UI-потоку
enum LoaderMsg {
    Playlists(Vec<Playlist>),
    Meta(String, TrackMeta),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lang {
    Ru,
    Uk,
}

impl Lang {
    // Все языки по порядку — в этом же порядке рисуются кнопки выбора.
    fn all() -> &'static [Lang] {
        &[Lang::Ru, Lang::Uk]
    }

    // Название языка на нём самом (подпись кнопки).
    fn native_name(&self) -> &'static str {
        match self {
            Lang::Ru => "Русский",
            Lang::Uk => "Українська",
        }
    }

    // Короткий код для сохранения в файл.
    fn code(&self) -> &'static str {
        match self {
            Lang::Ru => "ru",
            Lang::Uk => "uk",
        }
    }

    // Разбор кода из файла. Неизвестный код -> язык по умолчанию.
    fn from_code(code: &str) -> Lang {
        match code.trim() {
            "uk" => Lang::Uk,
            _ => Lang::Ru,
        }
    }
}

// Читает сохранённый язык (или язык по умолчанию).
fn load_language() -> Lang {
    let code = load_config().language;
    if code.is_empty() { Lang::Ru } else { Lang::from_code(&code) }
}

// Сохраняет выбранный язык.
fn save_language(lang: Lang) {
    let mut cfg = load_config();
    cfg.language = lang.code().to_string();
    save_config(&cfg);
}

// Все подписи интерфейса для одного языка.
// {n} в строке — место для подстановки числа (см. .replace("{n}", ...)).
struct Strings {
    search_hint: &'static str,
    user: &'static str,
    home: &'static str,
    new_playlist: &'static str,
    new_playlist_hint: &'static str,
    new_playlist_title: &'static str,   // ← добавить
    create: &'static str,               // ← добавить
    cancel: &'static str,               // ← добавить
    liked_music: &'static str,
    auto_created: &'static str,
    liked_empty: &'static str,
    like_hint: &'static str,
    unlike_hint: &'static str,
    play: &'static str,
    delete_playlist: &'static str,
    sort: &'static str,
    listen_again: &'static str,
    playlist_tracks: &'static str,
    unknown_title: &'static str,
    unknown_artist: &'static str,
    settings: &'static str,
    settings_in_dev: &'static str,
    settings_in_dev_sub: &'static str,
    language: &'static str,
    shortcuts: &'static str,
    press_key: &'static str,
    not_set: &'static str,
}

// Возвращает все строки для выбранного языка.
// Чтобы добавить язык — добавь сюда ещё одну ветку match (со всеми полями).
fn strings(lang: Lang) -> Strings {
    match lang {
        Lang::Ru => Strings {
            search_hint: "Поиск треков, артистов...",
            user: "👤 Профиль",
            home: "Главная",
            new_playlist: "➕ Новый",
            new_playlist_hint: "Название плейлиста",
            new_playlist_title: "Новый плейлист",
            create: "Создать",
            cancel: "Отмена",
            liked_music: "Понравившаяся музыка",
            auto_created: "📌 Создан автоматически",
            liked_empty: "Здесь пока пусто. Треки, которые вы лайкнете, появятся тут.",
            like_hint: "Сохранить в «Понравившаяся музыка»",
            unlike_hint: "Убрать из «Понравившаяся музыка»",
            play: "   ▶  Слушать   ",
            delete_playlist: "Удалить плейлист",
            sort: "Упорядочить",
            listen_again: "Послушать ещё раз",
            playlist_tracks: "Плейлист • {n} треков",
            unknown_title: "Без названия",
            unknown_artist: "Неизвестный исполнитель",
            settings: "Настройки",
            settings_in_dev: "Остальные настройки в разработке",
            settings_in_dev_sub: "Они появятся в одном из следующих обновлений.",
            language: "Язык",
            shortcuts: "Горячие клавиши",
            press_key: "Нажмите клавишу…",
            not_set: "не задано",
        },
        Lang::Uk => Strings {
            search_hint: "Пошук треків, виконавців...",
            user: "👤 Профіль",
            home: "Головна",
            new_playlist: "➕ Новий",
            new_playlist_hint: "Назва плейлиста",
            new_playlist_title: "Новий плейлист",
            create: "Створити",
            cancel: "Скасувати",
            liked_music: "Вподобана музика",
            auto_created: "📌 Створено автоматично",
            liked_empty: "Тут поки що порожньо. Треки, які ви вподобаєте, з'являться тут.",
            like_hint: "Зберегти у «Вподобана музика»",
            unlike_hint: "Прибрати з «Вподобана музика»",
            play: "   ▶  Слухати   ",
            delete_playlist: "Видалити плейлист",
            sort: "Упорядкувати",
            listen_again: "Послухати ще раз",
            playlist_tracks: "Плейлист • {n} треків",
            unknown_title: "Без назви",
            unknown_artist: "Невідомий виконавець",
            settings: "Налаштування",
            settings_in_dev: "Інші налаштування в розробці",
            settings_in_dev_sub: "Вони з'являться в одному з наступних оновлень.",
            language: "Мова",
            shortcuts: "Гарячі клавіші",
            press_key: "Натисніть клавішу…",
            not_set: "не призначено",
        },
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Shortcut {
    PlayPause,
    Next,
    Prev,
    VolumeUp,
    VolumeDown,
    ToggleLike,
}

impl Shortcut {
    // Все действия по порядку (в этом порядке рисуются строки в настройках).
    fn all() -> &'static [Shortcut] {
        &[
            Shortcut::PlayPause,
            Shortcut::Next,
            Shortcut::Prev,
            Shortcut::VolumeUp,
            Shortcut::VolumeDown,
            Shortcut::ToggleLike,
        ]
    }

    // Короткий код для сохранения в файл.
    fn code(&self) -> &'static str {
        match self {
            Shortcut::PlayPause => "play_pause",
            Shortcut::Next => "next",
            Shortcut::Prev => "prev",
            Shortcut::VolumeUp => "vol_up",
            Shortcut::VolumeDown => "vol_down",
            Shortcut::ToggleLike => "like",
        }
    }

    fn from_code(code: &str) -> Option<Shortcut> {
        Some(match code {
            "play_pause" => Shortcut::PlayPause,
            "next" => Shortcut::Next,
            "prev" => Shortcut::Prev,
            "vol_up" => Shortcut::VolumeUp,
            "vol_down" => Shortcut::VolumeDown,
            "like" => Shortcut::ToggleLike,
            _ => return None,
        })
    }

    // Подпись действия на выбранном языке.
    fn label(&self, lang: Lang) -> &'static str {
        match lang {
            Lang::Ru => match self {
                Shortcut::PlayPause => "Воспроизведение / Пауза",
                Shortcut::Next => "Следующий трек",
                Shortcut::Prev => "Предыдущий трек",
                Shortcut::VolumeUp => "Громче",
                Shortcut::VolumeDown => "Тише",
                Shortcut::ToggleLike => "Лайк / снять лайк",
            },
            Lang::Uk => match self {
                Shortcut::PlayPause => "Відтворення / Пауза",
                Shortcut::Next => "Наступний трек",
                Shortcut::Prev => "Попередній трек",
                Shortcut::VolumeUp => "Гучніше",
                Shortcut::VolumeDown => "Тихіше",
                Shortcut::ToggleLike => "Лайк / зняти лайк",
            },
        }
    }
}

// Клавиши, которые умеем СОХРАНЯТЬ между запусками.
// Нажать в настройках можно ЛЮБУЮ клавишу, но чтобы выбор пережил перезапуск,
// клавиша должна быть в этом списке. Нужна ещё одна — просто допиши строкой
// (например egui::Key::Backslash для «\»).
const BINDABLE_KEYS: &[egui::Key] = &[
    egui::Key::Space, egui::Key::Enter, egui::Key::Tab, egui::Key::Backspace,
    egui::Key::Delete, egui::Key::Insert, egui::Key::Home, egui::Key::End,
    egui::Key::PageUp, egui::Key::PageDown,
    egui::Key::ArrowUp, egui::Key::ArrowDown, egui::Key::ArrowLeft, egui::Key::ArrowRight,
    egui::Key::Num0, egui::Key::Num1, egui::Key::Num2, egui::Key::Num3, egui::Key::Num4,
    egui::Key::Num5, egui::Key::Num6, egui::Key::Num7, egui::Key::Num8, egui::Key::Num9,
    egui::Key::A, egui::Key::B, egui::Key::C, egui::Key::D, egui::Key::E, egui::Key::F,
    egui::Key::G, egui::Key::H, egui::Key::I, egui::Key::J, egui::Key::K, egui::Key::L,
    egui::Key::M, egui::Key::N, egui::Key::O, egui::Key::P, egui::Key::Q, egui::Key::R,
    egui::Key::S, egui::Key::T, egui::Key::U, egui::Key::V, egui::Key::W, egui::Key::X,
    egui::Key::Y, egui::Key::Z,
    egui::Key::F1, egui::Key::F2, egui::Key::F3, egui::Key::F4, egui::Key::F5, egui::Key::F6,
    egui::Key::F7, egui::Key::F8, egui::Key::F9, egui::Key::F10, egui::Key::F11, egui::Key::F12,
    egui::Key::Backslash,
];

// Имя клавиши для показа и сохранения, напр. "Space", "ArrowRight", "L".
fn key_label(key: egui::Key) -> String {
    format!("{:?}", key)
}

// Обратное преобразование имени в клавишу (для загрузки из файла).
fn key_from_label(name: &str) -> Option<egui::Key> {
    BINDABLE_KEYS.iter().copied().find(|k| key_label(*k) == name)
}

// Клавиши по умолчанию.
fn default_shortcuts() -> HashMap<Shortcut, egui::Key> {
    use egui::Key;
    let mut m = HashMap::new();
    m.insert(Shortcut::PlayPause, Key::Space);
    m.insert(Shortcut::Next, Key::ArrowRight);
    m.insert(Shortcut::Prev, Key::ArrowLeft);
    m.insert(Shortcut::VolumeUp, Key::ArrowUp);
    m.insert(Shortcut::VolumeDown, Key::ArrowDown);
    m.insert(Shortcut::ToggleLike, Key::L);
    m
}

// Загружает назначения (отсутствующие берутся из значений по умолчанию).
fn load_shortcuts() -> HashMap<Shortcut, egui::Key> {
    let mut map = default_shortcuts();
    for (code, value) in load_config().shortcuts {
        if let Some(action) = Shortcut::from_code(&code) {
            if value == "None" {
                map.remove(&action);
            } else if let Some(key) = key_from_label(&value) {
                map.insert(action, key);
            }
        }
    }
    map
}

// Сохраняет назначения.
fn save_shortcuts(map: &HashMap<Shortcut, egui::Key>) {
    let mut shortcuts = HashMap::new();
    for &action in Shortcut::all() {
        let value = match map.get(&action) {
            Some(&key) => key_label(key),
            None => "None".to_string(),
        };
        shortcuts.insert(action.code().to_string(), value);
    }
    let mut cfg = load_config();
    cfg.shortcuts = shortcuts;
    save_config(&cfg);
}


// Преобразование клавиши rdev (глобальный хук) в клавишу egui,
// чтобы сравнивать её с назначенными в настройках.
// Состояние, общее между потоком-перехватчиком (rdev::grab) и UI:
// какие клавиши «глотать» и активен ли перехват прямо сейчас.
struct GrabShared {
    keys: HashSet<egui::Key>,
    active: bool,
}

fn rdev_to_egui(k: rdev::Key) -> Option<egui::Key> {
    use egui::Key as E;
    use rdev::Key as R;
    Some(match k {
        R::Space => E::Space,
        R::Return => E::Enter,
        R::Tab => E::Tab,
        R::Backspace => E::Backspace,
        R::Delete => E::Delete,
        R::Insert => E::Insert,
        R::Home => E::Home,
        R::End => E::End,
        R::PageUp => E::PageUp,
        R::PageDown => E::PageDown,
        R::UpArrow => E::ArrowUp,
        R::DownArrow => E::ArrowDown,
        R::LeftArrow => E::ArrowLeft,
        R::RightArrow => E::ArrowRight,
        R::KeyA => E::A, R::KeyB => E::B, R::KeyC => E::C, R::KeyD => E::D,
        R::KeyE => E::E, R::KeyF => E::F, R::KeyG => E::G, R::KeyH => E::H,
        R::KeyI => E::I, R::KeyJ => E::J, R::KeyK => E::K, R::KeyL => E::L,
        R::KeyM => E::M, R::KeyN => E::N, R::KeyO => E::O, R::KeyP => E::P,
        R::KeyQ => E::Q, R::KeyR => E::R, R::KeyS => E::S, R::KeyT => E::T,
        R::KeyU => E::U, R::KeyV => E::V, R::KeyW => E::W, R::KeyX => E::X,
        R::KeyY => E::Y, R::KeyZ => E::Z,
        R::Num0 => E::Num0, R::Num1 => E::Num1, R::Num2 => E::Num2, R::Num3 => E::Num3,
        R::Num4 => E::Num4, R::Num5 => E::Num5, R::Num6 => E::Num6, R::Num7 => E::Num7,
        R::Num8 => E::Num8, R::Num9 => E::Num9,
        R::F1 => E::F1, R::F2 => E::F2, R::F3 => E::F3, R::F4 => E::F4,
        R::F5 => E::F5, R::F6 => E::F6, R::F7 => E::F7, R::F8 => E::F8,
        R::F9 => E::F9, R::F10 => E::F10, R::F11 => E::F11, R::F12 => E::F12,
        R::BackSlash => E::Backslash,
        _ => return None,
    })
}


// Поддерживаемые расширения аудио (для drag-and-drop папок и файлов).
fn is_audio_file(path: &std::path::Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("mp3" | "flac" | "wav" | "ogg" | "m4a" | "aac" | "opus" | "wma")
    )
}

// Рекурсивно собирает все аудиофайлы внутри папки (по алфавиту).
fn collect_audio_files(dir: &std::path::Path, out: &mut Vec<String>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        let mut items: Vec<std::path::PathBuf> = entries.flatten().map(|e| e.path()).collect();
        items.sort();
        for path in items {
            if path.is_dir() {
                collect_audio_files(&path, out);
            } else if is_audio_file(&path) {
                if let Some(s) = path.to_str() {
                    out.push(s.to_string());
                }
            }
        }
    }
}

struct App {
    playlists: Vec<Playlist>,
    player: Player,
    current_song: String,
    playback_queue: Vec<String>, // <--- очередь, зафиксированная при запуске трека
    is_playing: bool,
    volume: f32,
    total_duration: Option<Duration>,
    elapsed_duration: Duration,
    last_frame_instant: Instant,
    selected_playlist_idx: Option<usize>,
    track_meta: HashMap<String, TrackMeta>,
    loader_rx: Receiver<LoaderMsg>,
    loader_tx: std::sync::mpsc::Sender<LoaderMsg>, // <--- для дозагрузки обложек перетащенных треков
    search_query: String, // <--- ДОБАВЛЕНО ТУТ
    show_settings: bool,   // <--- открыто ли окно настроек
    language: Lang,        // <--- текущий язык интерфейса
    shortcuts: HashMap<Shortcut, egui::Key>, // <--- назначенные клавиши
    rebinding: Option<Shortcut>,             // <--- какое действие ждёт нажатие (None = нет)
    global_key_rx: Receiver<egui::Key>,      // <--- глобальные нажатия (rdev)
    grab_shared: Arc<Mutex<GrabShared>>,     // <--- что глотать (общее с потоком)
    update: Arc<Mutex<UpdateState>>,
    show_new_playlist: bool,     // ← открыт ли ввод имени нового плейлиста
    new_playlist_name: String,   // ← что пользователь печатает
    focus_new_playlist: bool,    // ← поставить фокус в поле один раз
    current_lyrics: Option<Vec<LyricLine>>,
    current_playback_time_ms: u32,
    song_start_time: Option<std::time::Instant>,
    lyrics_receiver: Option<Receiver<Option<Vec<LyricLine>>>>,
    lyrics_cache: HashMap<String, Vec<LyricLine>>,
    track_context_menu: Option<String>,  // путь к треку, для которого открыто меню "⋮"
    context_menu_pos: egui::Pos2,  // фиксированная позиция попапа
    context_menu_just_opened: bool,
    rename_playlist_idx: Option<usize>,  // какой плейлист переименовываем
    rename_playlist_name: String,        // текущий текст в поле ввода
    focus_rename_playlist: bool,         // поставить фокус один раз
}

// 1. Помещаем функцию на самый верхний уровень, прямо перед main
fn setup_custom_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    // Основной текст: латиница + кириллица + диакритика.
    fonts.font_data.insert(
        "noto_sans".to_owned(),
        egui::FontData::from_static(include_bytes!("fonts/ttf/NotoSans-Regular.ttf")),
    );
    // Эмодзи (как и было).
    fonts.font_data.insert(
        "emoji_font".to_owned(),
        egui::FontData::from_static(include_bytes!("fonts/ttf/NotoEmoji-VariableFont_wght.ttf")),
    );

    // Порядок в списке = приоритет перебора шрифтов для каждого символа.
    let prop = fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default();
    prop.insert(0, "noto_sans".to_owned()); // основной — первым
    prop.push("emoji_font".to_owned());     // эмодзи — в конец, фолбэком

    // Моноширинному семейству тоже дадим кириллицу запасным вариантом.
    fonts
        .families
        .entry(egui::FontFamily::Monospace)
        .or_default()
        .push("noto_sans".to_owned());

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

    // Сохраняет треки плейлиста «Понравившаяся музыка».
    fn save_liked(&self) {
        let songs: Vec<String> = self
            .playlists
            .iter()
            .find(|p| p.name == "Понравившаяся музыка")
            .map(|p| p.songs.clone())
            .unwrap_or_default();
        let mut cfg = load_config();
        cfg.liked = songs;
        save_config(&cfg);
    }

    // Сохраняет составы обычных плейлистов (кроме «Понравившаяся музыка»).
    fn save_playlists(&self) {
        let playlists: Vec<PlaylistData> = self
            .playlists
            .iter()
            .filter(|p| p.name != "Понравившаяся музыка")
            .map(|p| PlaylistData { name: p.name.clone(), songs: p.songs.clone() })
            .collect();
        let mut cfg = load_config();
        cfg.playlists = playlists;
        save_config(&cfg);
    }

    // Удаляет плейлист по индексу. Папку и mp3-файлы НЕ трогает — только убирает
    // плейлист из списка и запоминает, что он удалён (чтобы не вернулся при перезапуске).
    fn delete_playlist(&mut self, idx: usize) {
        if idx >= self.playlists.len() {
            return;
        }
        let name = self.playlists[idx].name.clone();
        if name == "Понравившаяся музыка" {
            return; // лайки удаляются сердечком, не отсюда
        }

        // 1) убираем из памяти
        self.playlists.remove(idx);

        // 2) поправляем открытую страницу
        match self.selected_playlist_idx {
            Some(sel) if sel == usize::MAX => {}                          // страница лайков — не трогаем
            Some(sel) if sel == idx => self.selected_playlist_idx = None, // удалили открытый — на Главную
            Some(sel) if sel > idx => self.selected_playlist_idx = Some(sel - 1),
            _ => {}
        }

        // 3) запоминаем удаление
        let mut deleted = load_deleted_playlists();
        deleted.insert(name);
        save_deleted_playlists(&deleted);

        // 4) перезаписываем составы плейлистов без удалённого
        self.save_playlists();
    }
    fn new(ctx: &egui::Context) -> Self {
        let (tx, loader_rx) = channel();
        let loader_tx = tx.clone(); // копия отправителя для дозагрузки перетащенных треков

        // Сканирование папок и чтение метаданных (теги + обложки) — самая тяжёлая
        // часть запуска. Уносим её в отдельный поток, чтобы окно открылось мгновенно,
        // а не после загрузки всей фонотеки.
        // --- ГЛОБАЛЬНЫЕ горячие клавиши (через rdev::grab) ---
        // grab перехватывает клавиатуру во всей системе и, в отличие от listen,
        // умеет «глотать» нажатие (вернуть None) — тогда Windows не пикает и
        // клавиша не уходит в игру. Что именно глотать — берём из grab_shared.
        let (global_tx, global_key_rx) = channel::<egui::Key>();
        let grab_shared = Arc::new(Mutex::new(GrabShared {
            keys: HashSet::new(),
            active: true,
        }));
        let shared_for_thread = grab_shared.clone();
        let ctx_global = ctx.clone();
        thread::spawn(move || {
            let callback = move |event: rdev::Event| -> Option<rdev::Event> {
                if let rdev::EventType::KeyPress(rkey) = event.event_type {
                    if let Some(ekey) = rdev_to_egui(rkey) {
                        let consume = {
                            let st = shared_for_thread.lock().unwrap();
                            st.active && st.keys.contains(&ekey)
                        };
                        if consume {
                            let _ = global_tx.send(ekey);
                            ctx_global.request_repaint(); // будим UI, даже если свёрнуто
                            return None; // ГЛОТАЕМ клавишу: тишина + не уходит в игру
                        }
                    }
                }
                Some(event) // остальное пропускаем дальше
            };
            if let Err(err) = rdev::grab(callback) {
                eprintln!("⚠️ Не удалось запустить глобальные хоткеи: {:?}", err);
            }
        });


        let ctx_clone = ctx.clone();
        thread::spawn(move || {
            let mut playlists = scan_music("../DownloadedMusic");

            // Прячем плейлисты, удалённые пользователем (их папки/файлы остаются на диске).
            let deleted = load_deleted_playlists();
            if !deleted.is_empty() {
                playlists.retain(|p| !deleted.contains(&p.name));
            }

            // Подмешиваем вручную добавленные треки (через «три точки» / правый клик)
            // из playlists.txt в соответствующие плейлисты.
            for (name, songs) in load_saved_playlists() {
                if name == "Понравившаяся музыка" || deleted.contains(&name) {
                    continue; // лайки восстанавливаются отдельно; удалённые не возвращаем
                }
                if let Some(p) = playlists.iter_mut().find(|p| p.name == name) {
                    for s in songs {
                        // пропускаем дубли и пути к уже удалённым файлам
                        if std::path::Path::new(&s).exists() && !p.songs.contains(&s) {
                            p.songs.push(s);
                        }
                    }
                } else {
                    // Плейлист есть в сохранёнке, но папку сканер не нашёл — создаём заново.
                    let songs: Vec<String> = songs
                        .into_iter()
                        .filter(|s| std::path::Path::new(s).exists())
                        .collect();
                    playlists.push(Playlist { name, songs });
                }
            }

            // Метаданные (теги + обложки) грузим для треков всех плейлистов,
            // но каждый путь — только один раз (трек может быть сразу в нескольких).
            let mut seen_paths = HashSet::new();
            let all_paths: Vec<String> = playlists
                .iter()
                .flat_map(|p| p.songs.iter().cloned())
                .filter(|s| seen_paths.insert(s.clone()))
                .collect();

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

        let update = Arc::new(Mutex::new(UpdateState {
            checking: true,
            ..Default::default()
        }));

        {
            let update_clone = update.clone();

            std::thread::spawn(move || {
                let result = self_update::backends::github::Update::configure()
                    .repo_owner("Px228-Da-Da")
                    .repo_name("test_dowd")
                    .bin_name("Elysium")
                    .show_download_progress(false)
                    .current_version(env!("CARGO_PKG_VERSION"))
                    .build();

                if let Ok(updater) = result {
                    if let Ok(release) = updater.get_latest_release() {
                        let latest = release.version.trim_start_matches('v').to_string();
                        let mut state = update_clone.lock().unwrap();
                        state.checking = false;

                        // Показываем окно ТОЛЬКО если релиз строго новее текущей версии.
                        let current = env!("CARGO_PKG_VERSION");
                        let newer = self_update::version::bump_is_greater(current, &latest)
                            .unwrap_or(false);
                        if newer {
                            state.available = true;
                            state.latest_version = latest;
                        }
                    }
                }
            });
        }

        Self {
            playlists: Vec::new(),
            player: Player::new(),
            current_song: String::new(),
            playback_queue: Vec::new(),
            is_playing: false,
            volume: 0.5,
            total_duration: None,
            elapsed_duration: Duration::ZERO,
            last_frame_instant: Instant::now(),
            selected_playlist_idx: None,
            track_meta: HashMap::new(),
            loader_rx,
            loader_tx,
            search_query: String::new(), // <--- ДОБАВЛЕНО ТУТ
            show_settings: false,         // <--- окно настроек по умолчанию закрыто
            language: load_language(), // <--- загружаем сохранённый язык
            shortcuts: load_shortcuts(), // <--- загружаем сохранённые хоткеи
            rebinding: None,
            global_key_rx,
            grab_shared,
            update,
            show_new_playlist: false,
            new_playlist_name: String::new(),
            focus_new_playlist: false,
            current_lyrics: None,
            current_playback_time_ms: 0,
            song_start_time: None,
            lyrics_receiver: None,
            lyrics_cache: HashMap::new(),
            track_context_menu: None,
            context_menu_pos: egui::pos2(0.0, 0.0),
            context_menu_just_opened: false,
            rename_playlist_idx: None,
            rename_playlist_name: String::new(),
            focus_rename_playlist: false,
        }
    }

    fn play_next_track(&mut self) {
        // Очередь берём из playback_queue — она зафиксирована при запуске трека,
        // поэтому переключение вкладок (Главная / другой плейлист) на неё не влияет.
        if self.playback_queue.is_empty() {
            self.is_playing = false;
            return;
        }

        if let Some(current_idx) = self.playback_queue.iter().position(|s| s == &self.current_song) {
            let next_idx = current_idx + 1;
            if next_idx < self.playback_queue.len() {
                self.play_track(&self.playback_queue[next_idx].clone());
            } else {
                self.is_playing = false;
                self.elapsed_duration = Duration::ZERO;
            }
        } else {
            self.play_track(&self.playback_queue[0].clone());
        }
    }

    fn play_previous_track(&mut self) {
        if self.playback_queue.is_empty() { return; }

        if let Some(current_idx) = self.playback_queue.iter().position(|s| s == &self.current_song) {
            if current_idx > 0 {
                self.play_track(&self.playback_queue[current_idx - 1].clone());
            } else {
                self.player.seek(&self.current_song, Duration::ZERO);
                self.elapsed_duration = Duration::ZERO;
            }
        }
    }

    // Выполняет действие горячей клавиши.
    fn do_shortcut(&mut self, action: Shortcut) {
        match action {
            Shortcut::PlayPause => {
                if self.current_song.is_empty() {
                    return;
                }
                if self.is_playing {
                    self.player.pause();
                    self.is_playing = false;
                } else {
                    self.player.resume();
                    self.is_playing = true;
                }
            }
            Shortcut::Next => self.play_next_track(),
            Shortcut::Prev => self.play_previous_track(),
            Shortcut::VolumeUp => {
                self.volume = (self.volume + 0.05).min(1.0);
                self.player.set_volume(self.volume);
            }
            Shortcut::VolumeDown => {
                self.volume = (self.volume - 0.05).max(0.0);
                self.player.set_volume(self.volume);
            }
            Shortcut::ToggleLike => {
                if !self.current_song.is_empty() {
                    let song = self.current_song.clone();
                    self.toggle_like(&song);
                }
            }
        }
    }

    // Обрабатывает клавиатуру каждый кадр: переназначение или срабатывание хоткеев.
    // В фокусе нажатия не нужны: их перехватывает rdev::grab (см. поток в new()).
    // Здесь оставляем только Esc и захват клавиши при переназначении в настройках.
    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let esc = ctx.input(|i| i.key_pressed(egui::Key::Escape));

        // Ждём нажатие клавиши для переназначения
        if let Some(action) = self.rebinding {
            if esc {
                self.rebinding = None; // Esc — отмена
            } else if let Some(key) = ctx.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Key { key, pressed: true, .. } => Some(*key),
                    _ => None,
                })
            }) {
                self.shortcuts.insert(action, key);
                save_shortcuts(&self.shortcuts);
                self.rebinding = None;
            }
            return;
        }

        // Esc закрывает окно настроек
        if self.show_settings && esc {
            self.show_settings = false;
        }
    }


    // Глобальные хоткеи: срабатывают всегда (в т.ч. когда окно не в фокусе — в игре).
    // Клавишу «глотает» rdev::grab, поэтому здесь просто выполняем действие.
    fn handle_global_keys(&mut self, ctx: &egui::Context) {
        // Сообщаем потоку-перехватчику, какие клавиши глотать и активен ли перехват.
        {
            let mut st = self.grab_shared.lock().unwrap();
            // Не перехватываем в настройках / при переназначении / когда печатаем
            // в поле поиска — иначе клавишу не получится ни ввести, ни назначить.
            st.active =
                !(self.show_settings || self.rebinding.is_some() || ctx.wants_keyboard_input());
            st.keys = self.shortcuts.values().copied().collect();
        }

        // Выполняем действия по перехваченным клавишам.
        while let Ok(key) = self.global_key_rx.try_recv() {
            if self.show_settings || self.rebinding.is_some() {
                continue; // опустошаем очередь, но не реагируем
            }
            if let Some(action) = self
                .shortcuts
                .iter()
                .find(|(_, &k)| k == key)
                .map(|(&a, _)| a)
            {
                self.do_shortcut(action);
            }
        }
    }


    fn play_track(&mut self, path: &str) {
        self.current_song = path.to_string();
        self.total_duration = self.player.play(path);

        // 1. Проверяем кэш СНАЧАЛА (до запуска потока)
        if let Some(cached) = self.lyrics_cache.get(path) {
            self.current_lyrics = Some(cached.clone());
            self.lyrics_receiver = None; // Поток не нужен
        } else {
            // Если в кэше нет — очищаем старый текст и запускаем поток
            self.current_lyrics = None;
            let (tx, rx) = std::sync::mpsc::channel();
            self.lyrics_receiver = Some(rx);
            
            let path_for_thread = path.to_string();
            let duration_for_thread = self.total_duration; // Option<Duration> копируется (Copy)

            std::thread::spawn(move || {
                // Сначала ищем текст, вшитый прямо в MP3
                if let Some(lyrics) = scanner::get_synced_lyrics(&path_for_thread) {
                    let _ = tx.send(Some(lyrics));
                    return;
                }

                // Если вшитого нет — ищем в интернете по тегам и длительности.
                // Передаём ПОЛНЫЙ путь (он нужен для чтения ID3-тегов), а не имя файла.
                let internet_lyrics =
                    scanner::fetch_lyrics_from_internet(&path_for_thread, duration_for_thread);
                let _ = tx.send(internet_lyrics);
            });
        }

        self.current_playback_time_ms = 0;
        self.song_start_time = Some(std::time::Instant::now());
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

    // Обрабатывает перетащенные в окно пути.
    //  • Папка  -> отдельный плейлист с именем папки (рекурсивно собирает аудио внутри).
    //  • Файлы  -> добавляются в открытый сейчас плейлист, а если ничего не открыто —
    //              в общий плейлист «Добавленные треки».
    fn add_dropped_paths(&mut self, ctx: &egui::Context, paths: Vec<std::path::PathBuf>) {
        let mut loose_files: Vec<String> = Vec::new();
        let mut changed_playlists = false;
        let mut changed_liked = false;
        let mut new_meta_paths: Vec<String> = Vec::new();

        for path in paths {
            if path.is_dir() {
                // Папка -> плейлист по имени папки.
                let mut songs = Vec::new();
                collect_audio_files(&path, &mut songs);
                if songs.is_empty() {
                    continue;
                }
                let name = path
                    .file_name()
                    .map(|n| nfc(&n.to_string_lossy()))
                    .unwrap_or_else(|| "Новый плейлист".to_string());

                // Если такой плейлист раньше удаляли — «возвращаем» его.
                let mut deleted = load_deleted_playlists();
                if deleted.remove(&name) {
                    save_deleted_playlists(&deleted);
                }

                if let Some(pl) = self.playlists.iter_mut().find(|p| p.name == name) {
                    for s in &songs {
                        if !pl.songs.contains(s) {
                            pl.songs.push(s.clone());
                        }
                    }
                } else {
                    self.playlists.push(Playlist { name, songs: songs.clone() });
                }
                new_meta_paths.extend(songs);
                changed_playlists = true;
            } else if is_audio_file(&path) {
                if let Some(s) = path.to_str() {
                    loose_files.push(s.to_string());
                }
            }
        }

        // Отдельные файлы.
        if !loose_files.is_empty() {
            let target_idx = match self.selected_playlist_idx {
                Some(idx) if idx != usize::MAX && idx < self.playlists.len() => Some(idx),
                _ => None,
            };

            if let Some(idx) = target_idx {
                let is_liked = self.playlists[idx].name == "Понравившаяся музыка";
                for s in &loose_files {
                    if !self.playlists[idx].songs.contains(s) {
                        self.playlists[idx].songs.push(s.clone());
                    }
                }
                if is_liked {
                    changed_liked = true;
                } else {
                    changed_playlists = true;
                }
            } else {
                let name = match self.language {
                    Lang::Ru => "Добавленные треки",
                    Lang::Uk => "Додані треки",
                }
                .to_string();
                if let Some(pl) = self.playlists.iter_mut().find(|p| p.name == name) {
                    for s in &loose_files {
                        if !pl.songs.contains(s) {
                            pl.songs.push(s.clone());
                        }
                    }
                } else {
                    self.playlists.push(Playlist { name, songs: loose_files.clone() });
                }
                changed_playlists = true;
            }
            new_meta_paths.extend(loose_files);
        }

        if changed_liked {
            self.save_liked();
        }
        if changed_playlists {
            self.save_playlists();
        }

        // Фоновая загрузка обложек/тегов для новых треков (без дублей и уже известных).
        let mut seen = HashSet::new();
        let to_load: Vec<String> = new_meta_paths
            .into_iter()
            .filter(|s| !self.track_meta.contains_key(s) && seen.insert(s.clone()))
            .collect();
        if !to_load.is_empty() {
            let tx = self.loader_tx.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                for path in to_load {
                    let meta = read_track_meta(&ctx, &path);
                    if tx.send(LoaderMsg::Meta(path, meta)).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            });
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        
        // 1. Обновляем время воспроизведения для синхронизации текста
        if self.is_playing { 
            // Используем встроенный таймер elapsed_duration, так как
            // get_pos() в rodio может отсутствовать в вашей версии
            self.current_playback_time_ms = self.elapsed_duration.as_millis() as u32;
        }

        // 2. Проверяем почтовый ящик: скачался ли текст из интернета?
        if let Some(rx) = &self.lyrics_receiver {
            if let Ok(lyrics_result) = rx.try_recv() {
                self.current_lyrics = lyrics_result.clone();
                self.lyrics_receiver = None; // Ящик больше не нужен
                
                // Сохраняем в кэш успешно загруженный текст, чтобы не качать снова
                if let Some(lyrics) = lyrics_result {
                    self.lyrics_cache.insert(self.current_song.clone(), lyrics);
                }
            }
        }

        apply_custom_theme(ctx);

        let update_info = {
            self.update.lock().unwrap().clone()
        };
        
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

        // Перетаскивание в окно: папка -> новый плейлист, файлы -> в открытый плейлист.
        let dropped: Vec<std::path::PathBuf> = ctx.input(|i| {
            i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect()
        });
        if !dropped.is_empty() {
            self.add_dropped_paths(ctx, dropped);
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

        if update_info.available {
            egui::Window::new("Обновление")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(format!(
                        "Доступна новая версия {}",
                        update_info.latest_version
                    ));

                    // Если в прошлый раз произошла ошибка — покажем её красным цветом
                    if let Some(ref err) = update_info.error {
                        ui.colored_label(Color32::LIGHT_RED, format!("Ошибка: {}", err));
                    }

                    // Если идет скачивание — блокируем кнопку и показываем индикатор
                    if update_info.updating {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Скачивание и замена файла...");
                        });
                    } else {
                        if ui.button("Обновить сейчас").clicked() {
                            let update_clone = self.update.clone();

                            // Переводим статус в режим загрузки и очищаем старые ошибки
                            {
                                let mut state = update_clone.lock().unwrap();
                                state.updating = true;
                                state.error = None;
                            }

                            std::thread::spawn(move || {
                                let result = self_update::backends::github::Update::configure()
                                    .repo_owner("Px228-Da-Da")
                                    .repo_name("test_dowd")
                                    .bin_name("Elysium")
                                    .show_download_progress(false) // ВАЖНО: false для GUI-приложений!
                                    .current_version(env!("CARGO_PKG_VERSION"))
                                    .build()
                                    .unwrap()
                                    .update();

                                let mut state = update_clone.lock().unwrap();
                                state.updating = false;

                                match result {
                                    Ok(_) => {
                                        // Всё прошло успешно, заменяем файл и закрываемся.
                                        // При следующем ручном запуске откроется новая версия.
                                        std::process::exit(0);
                                    }
                                    Err(e) => {
                                        // Записываем ошибку в состояние, чтобы вывести её в UI
                                        state.error = Some(e.to_string());
                                    }
                                }
                            });
                        }
                    }
                });
        }

        let text_muted = Color32::from_rgb(167, 167, 167);
        let accent_color = Color32::from_rgb(29, 185, 84);

        // Все подписи интерфейса на выбранном языке (см. fn strings ниже).
        let s = strings(self.language);

        // Горячие клавиши: переназначение и срабатывание.
        self.handle_shortcuts(ctx);
        self.handle_global_keys(ctx); // глобальные хоткеи (когда окно не в фокусе)

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
                                    s.unlike_hint
                                } else {
                                    s.like_hint
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
                    ui.label(RichText::new("Elysium").size(20.0).strong().color(Color32::WHITE));
                    ui.add_space(25.0);

                    // Навигация (структура YT, но ваш стиль)
                    // let nav_items = [("🏠", "Главная"), ("🧭", "Навигатор"), ("📚", "Библиотека")];
                    // Навигация (в стиле закругленной плашки, как на фото 2)
                    let nav_items = [("🏠", s.home)];
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

                    // Кнопка «Новый» — открывает модальное окно создания плейлиста
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
                    ui.painter().text(text_pos, egui::Align2::LEFT_TOP, s.liked_music, FontId::proportional(15.0), Color32::WHITE);
                    ui.painter().text(text_pos + vec2(0.0, 20.0), egui::Align2::LEFT_TOP, s.auto_created, FontId::proportional(12.0), text_muted);

                    // Обработка клика (используем usize::MAX как специальный ID для этой страницы)
                    if response.clicked() {
                        self.selected_playlist_idx = Some(usize::MAX);
                    }

                    ui.add_space(20.0);

                    // Список обычных плейлистов
                    let playlist_to_delete: Option<usize> = None;
                    egui::ScrollArea::vertical().show(ui, |ui| {
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

                            // Правый клик по плейлисту — меню с удалением
                            // response.context_menu(|ui| {
                            //     style_menu(ui);
                            //     ui.set_min_width(190.0);
                            //     if ui.add(
                            //         egui::Button::new(
                            //             RichText::new("🗑  Удалить плейлист")
                            //                 .size(14.0)
                            //                 .color(Color32::from_rgb(240, 110, 110)),
                            //         )
                            //         .min_size(vec2(182.0, 32.0))
                            //         .rounding(6.0),
                            //     ).clicked() {
                            //         playlist_to_delete = Some(idx);
                            //         ui.close_menu();
                            //     }
                            // });

                            ui.add_space(4.0);
                        }
                    });

                    // Удаляем уже ПОСЛЕ цикла — внутри нельзя, список занят итератором.
                    if let Some(idx) = playlist_to_delete {
                        self.delete_playlist(idx);
                    }
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
                            .hint_text(RichText::new(s.search_hint).color(text_muted))
                            .text_color(Color32::WHITE)
                            .desired_width(340.0)
                    );

                    // 2. ДОБАВЛЕНО ТУТ: Если нажали на поиск или начали вводить текст — перекидываем на Главную
                    if response.gained_focus() || response.changed() {
                        self.selected_playlist_idx = None;
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Клик по кнопке профиля открывает окно настроек
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
                            ui.label(RichText::new(s.liked_music).size(28.0).strong().color(Color32::WHITE));
                            ui.add_space(10.0);
                            ui.label(RichText::new(s.liked_empty).size(16.0).color(text_muted));
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
                                    // Заголовок плейлиста: у плейлиста лайков name хранит русский ключ —
                                    // поэтому для показа подменяем его на перевод s.liked_music.
                                    let playlist_title: &str = if playlist.name == "Понравившаяся музыка" {
                                        s.liked_music
                                    } else {
                                        playlist.name.as_str()
                                    };
                                    ui.label(RichText::new(playlist_title).size(24.0).strong().color(Color32::WHITE));
                                    ui.add_space(4.0);
                                    ui.label(RichText::new(s.playlist_tracks.replace("{n}", &playlist.songs.len().to_string())).size(13.0).color(text_muted));
                                    ui.add_space(12.0);
                                    
                                    ui.horizontal(|ui| {
                                        // if ui.add(egui::Button::new(RichText::new(s.play).size(15.0).color(Color32::BLACK))
                                        //     .fill(accent_color)
                                        //     .rounding(20.0)
                                        //     .min_size(vec2(100.0, 36.0))).clicked() {
                                        //     if !playlist.songs.is_empty() {
                                        //         self.playback_queue = self.get_current_queue();
                                        //         self.play_track(&playlist.songs[0]);
                                        //     }
                                        // }

                                        ui.horizontal(|ui| {
                                            if idx != usize::MAX && playlist.name != "Понравившаяся музыка" {
                                                // ✏️ Кнопка переименования — первая
                                                let rename_btn = ui.add(
                                                    egui::Button::new(
                                                        RichText::new("✏").size(16.0).color(Color32::from_rgb(180, 180, 180)),
                                                    )
                                                    .fill(Color32::from_rgb(45, 45, 45))
                                                    .rounding(20.0)
                                                    .min_size(vec2(40.0, 36.0)),
                                                ).on_hover_text(match self.language {
                                                    Lang::Ru => "Переименовать плейлист",
                                                    Lang::Uk => "Перейменувати плейлист",
                                                });
                                                if rename_btn.clicked() {
                                                    self.rename_playlist_idx = Some(idx);
                                                    self.rename_playlist_name = playlist.name.clone();
                                                    self.focus_rename_playlist = true;
                                                }
                                                ui.add_space(8.0);
                                            }

                                            // ▶ Кнопка Слушать — вторая
                                            if ui.add(egui::Button::new(RichText::new(s.play).size(15.0).color(Color32::BLACK))
                                                .fill(accent_color)
                                                .rounding(20.0)
                                                .min_size(vec2(100.0, 36.0))).clicked() {
                                                if !playlist.songs.is_empty() {
                                                    self.playback_queue = self.get_current_queue();
                                                    self.play_track(&playlist.songs[0]);
                                                }
                                            }

                                            if idx != usize::MAX && playlist.name != "Понравившаяся музыка" {
                                                // 🗑 Кнопка удаления — третья
                                                ui.add_space(8.0);
                                                let del = ui.add(
                                                    egui::Button::new(
                                                        RichText::new("🗑").size(16.0).color(Color32::from_rgb(240, 110, 110)),
                                                    )
                                                    .fill(Color32::from_rgb(45, 45, 45))
                                                    .rounding(20.0)
                                                    .min_size(vec2(40.0, 36.0)),
                                                ).on_hover_text(s.delete_playlist);
                                                if del.clicked() {
                                                    self.delete_playlist(idx);
                                                }
                                            }
                                        });
                                    });
                                }
                            );

                            ui.add_space(24.0);

                            // ПРАВАЯ КОЛОНКА (Список треков)
                            ui.allocate_ui_with_layout(
                                vec2(ui.available_width(), remaining_height),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    ui.label(RichText::new(s.sort).size(13.0).color(text_muted));
                                    ui.add_space(10.0);

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
                                            for song in filtered_songs {
                                                let meta = self.track_meta.get(song);
                                                let is_active = self.current_song == *song;

                                                let row_height = 56.0;
                                                let (rect, response) = ui.allocate_exact_size(
                                                    vec2(ui.available_width() - 16.0, row_height),
                                                    egui::Sense::click(),
                                                );

                                                let is_hovered = response.hovered();
                                                if is_hovered {
                                                    ui.painter().rect_filled(rect, Rounding::same(6.0), Color32::from_rgb(40, 40, 40));
                                                }

                                                // Обложка
                                                let img_size = 40.0;
                                                let img_pos = rect.min + vec2(8.0, 8.0);
                                                let img_rect = Rect::from_min_size(img_pos, vec2(img_size, img_size));
                                                ui.painter().rect_filled(img_rect, Rounding::same(4.0), Color32::from_rgb(50, 50, 50));
                                                if let Some(tex) = meta.and_then(|m| m.cover.as_ref()) {
                                                    ui.painter().image(tex.id(), img_rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                                                }
                                                if is_hovered || (is_active && self.is_playing) {
                                                    ui.painter().rect_filled(img_rect, Rounding::same(4.0), Color32::from_black_alpha(150));
                                                    let icon = if is_active && self.is_playing { "⏸" } else { "▶" };
                                                    ui.painter().text(img_rect.center(), egui::Align2::CENTER_CENTER, icon, FontId::proportional(16.0), accent_color);
                                                }

                                                // Текст
                                                let text_color = if is_active { accent_color } else { Color32::WHITE };
                                                let title = meta.map(|m| m.title.clone()).unwrap_or_else(|| s.unknown_title.to_string());
                                                let artist = meta.and_then(|m| m.artist.clone()).unwrap_or_else(|| s.unknown_artist.to_string());
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

                                                // ─── Кнопка ❤ ────────────────────────────────────────────────────
                                                let track_liked = self.is_liked(song);
                                                let heart_color = if track_liked { accent_color } else { Color32::from_rgb(120, 120, 120) };
                                                let heart_rect = Rect::from_min_size(
                                                    pos2(rect.right() - 72.0, rect.center().y - 15.0),
                                                    vec2(30.0, 30.0),
                                                );
                                                let mut child_ui = ui.new_child(egui::UiBuilder::new().max_rect(heart_rect));
                                                let heart_click = child_ui.add(
                                                    egui::Button::new(RichText::new("❤").size(18.0).color(heart_color))
                                                        .fill(Color32::TRANSPARENT)
                                                        .frame(false)
                                                );
                                                if heart_click.clicked() {
                                                    self.toggle_like(song);
                                                }

                                                // ─── Кнопка ⋮ (три точки) ────────────────────────────────────────
                                                let dots_rect = Rect::from_min_size(
                                                    pos2(rect.right() - 36.0, rect.center().y - 15.0),
                                                    vec2(28.0, 30.0),
                                                );
                                                let dots_click = ui.interact(
                                                    dots_rect,
                                                    ui.id().with(song.as_str()),
                                                    egui::Sense::click(),
                                                );
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

                                                // ─── Клик по строке ───────────────────────────────────────────────
                                                if response.clicked() && !heart_click.clicked() && !dots_click.clicked() {
                                                    if is_active {
                                                        if self.is_playing { self.player.pause(); self.is_playing = false; }
                                                        else { self.player.resume(); self.is_playing = true; }
                                                    } else {
                                                        self.playback_queue = self.get_current_queue();
                                                        self.play_track(song);
                                                    }
                                                }
                                            }
                                        }); // конец ScrollArea

                                    // ─── Popup-меню (вне цикла, рисуется поверх всего) ───────────────────
                                    if let Some(ref ctx_song) = self.track_context_menu.clone() {
                                        let popup_rect = Rect::from_min_size(
                                            self.context_menu_pos,
                                            vec2(180.0, 44.0),
                                        );

                                        // Закрываем по клику мимо, пропуская первый фрейм
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
                                            Lang::Ru => "Удалить из плейлиста   ",
                                            Lang::Uk => "Видалити з плейлиста   ",
                                        };

                                        let is_menu_hovered = ui.input(|i| {
                                            i.pointer.hover_pos().map(|p| popup_rect.contains(p)).unwrap_or(false)
                                        });
                                        let bg_color = if is_menu_hovered { Color32::from_rgb(45, 45, 45) } else { Color32::from_rgb(32, 32, 32) };
                                        let text_color = if is_menu_hovered { Color32::from_rgb(255, 130, 130) } else { Color32::from_rgb(240, 110, 110) };

                                        painter.rect_filled(popup_rect, Rounding::same(8.0), bg_color);
                                        painter.rect_stroke(popup_rect, Rounding::same(8.0), Stroke::new(1.0, Color32::from_rgb(70, 70, 70)));
                                        painter.text(
                                            pos2(popup_rect.min.x + 14.0, popup_rect.center().y),
                                            egui::Align2::LEFT_CENTER,
                                            "🗑",
                                            FontId::proportional(13.0),
                                            text_color,
                                        );
                                        painter.text(
                                            pos2(popup_rect.min.x + 32.0, popup_rect.center().y),
                                            egui::Align2::LEFT_CENTER,
                                            remove_label,
                                            FontId::proportional(13.0),
                                            text_color,
                                        );

                                        let btn_resp = ui.interact(
                                            popup_rect,
                                            egui::Id::new("ctx_menu_delete_btn"),
                                            egui::Sense::click(),
                                        );
                                        if btn_resp.clicked() {
                                            let song_path = ctx_song.clone();
                                            if let Some(pl_idx) = self.selected_playlist_idx {
                                                let pl = if pl_idx == usize::MAX {
                                                    self.playlists.iter_mut().find(|p| p.name == "Понравившаяся музыка")
                                                } else {
                                                    self.playlists.get_mut(pl_idx)
                                                };
                                                if let Some(pl) = pl {
                                                    pl.songs.retain(|s| s != &song_path);
                                                }
                                                if pl_idx == usize::MAX { self.save_liked(); } else { self.save_playlists(); }
                                            }
                                            self.track_context_menu = None;
                                        }
                                    }
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
                            // egui::ScrollArea::horizontal()
                            //     .id_salt("chips_horizontal_scroll")
                            //     .show(ui, |ui| {
                            //         ui.horizontal(|ui| {
                            //             let chips = ["пинаю хуй"];
                            //             for chip in chips {
                            //                 ui.add(egui::Button::new(RichText::new(chip).size(13.0).color(Color32::WHITE))
                            //                     .fill(Color32::from_rgb(30, 30, 30))
                            //                     .rounding(16.0));
                            //                 ui.add_space(4.0);
                            //             }
                            //         });
                            //     });
                            // ui.add_space(30.0);

                            // Заголовок секции
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.label(RichText::new(s.listen_again).size(26.0).strong().color(Color32::WHITE));
                                });
                            });
                            ui.add_space(20.0);

                            // 3. Сетка всех треков (ВАШ ОРИГИНАЛЬНЫЙ ДИЗАЙН КАРТОЧЕК)
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing = egui::vec2(18.0, 24.0);

                                let query = self.search_query.to_lowercase(); // <-- Берем текст из поиска
                                
                                // --- ИСПРАВЛЕНИЕ: Используем HashSet для дедупликации треков на Главной ---
                                let mut seen_songs = HashSet::new();
                                let all_songs: Vec<String> = self
                                    .playlists
                                    .iter()
                                    .filter(|p| p.name != "Понравившаяся музыка")
                                    .flat_map(|p| p.songs.clone())
                                    .filter(|song| {
                                        // Если песню уже видели в другом плейлисте, не дублируем её на Главной
                                        if !seen_songs.insert(song.clone()) {
                                            return false;
                                        }
                                        if query.is_empty() { return true; }
                                        let meta = self.track_meta.get(song);
                                        let title = meta.map(|m| m.title.to_lowercase()).unwrap_or_default();
                                        let artist = meta.and_then(|m| m.artist.clone()).unwrap_or_default().to_lowercase();
                                        title.contains(&query) || artist.contains(&query)
                                    })
                                    .collect();
                                    
                                let home_queue = all_songs.clone(); // очередь = ровно тот список, что виден на Главной
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

                                    // --- Автоматическое ограничение длины текста по ширине карточки ---
                                    let max_chars_title = 15;
                                    let max_chars_artist = 18;

                                    let title = meta.map(|m| m.title.clone()).unwrap_or_else(|| s.unknown_title.to_string());
                                    let display_name = if title.chars().count() > max_chars_title {
                                        format!("{}...", title.chars().take(max_chars_title - 3).collect::<String>())
                                    } else { title };

                                    ui.painter().text(text_pos, egui::Align2::LEFT_TOP, display_name, FontId::proportional(14.0), text_color);

                                    let artist = meta.and_then(|m| m.artist.clone()).unwrap_or_else(|| s.unknown_artist.to_string());
                                    let subtitle = if artist.chars().count() > max_chars_artist {
                                        format!("{}...", artist.chars().take(max_chars_artist - 3).collect::<String>())
                                    } else { artist };
                                    
                                    let subtext_pos = text_pos + Vec2::new(0.0, 18.0);
                                    ui.painter().text(subtext_pos, egui::Align2::LEFT_TOP, subtitle, FontId::proportional(12.0), text_muted);

                                    // --- Интерактивное сердечко ---
                                    let liked = self.is_liked(&song);
                                    let heart_color = if liked { accent_color } else { Color32::from_rgb(100, 100, 100) };

                                    let heart_btn_size = vec2(28.0, 28.0);
                                    let heart_btn_pos = pos2(rect.right() - 36.0, rect.bottom() - 36.0);
                                    let heart_rect = Rect::from_min_size(heart_btn_pos, heart_btn_size);

                                    let mut heart_ui = ui.new_child(egui::UiBuilder::new().max_rect(heart_rect));
                                    let heart_click = heart_ui.add(
                                        egui::Button::new(RichText::new("❤").size(16.0).color(heart_color))
                                            .fill(Color32::TRANSPARENT)
                                            .frame(false)
                                    );

                                    if heart_click.clicked() {
                                        self.toggle_like(&song);
                                    }

                                    // 🛑 ТРИ ВЕРТИКАЛЬНЫЕ ТОЧКИ в правом верхнем углу обложки
                                    let dots_btn_size = vec2(28.0, 28.0);
                                    let dots_btn_pos = pos2(rect.right() - 34.0, rect.min.y + 10.0);
                                    let dots_rect = Rect::from_min_size(dots_btn_pos, dots_btn_size);

                                    // Невидимая кликабельная зона — нет фона/рамки, значит нет «кнопки»
                                    let dots_id = ui.make_persistent_id(("dots_btn", song.as_str()));
                                    let dots_resp = ui.interact(dots_rect, dots_id, egui::Sense::click());

                                    // Лёгкая подсветка-«чип» только при наведении
                                    if dots_resp.hovered() {
                                        ui.painter().rect_filled(
                                            dots_rect,
                                            Rounding::same(6.0),
                                            Color32::from_black_alpha(70),
                                        );
                                    }

                                    // Точки рисуем вручную — не зависит от шрифта, никаких квадратов
                                    let dot_color = if dots_resp.hovered() {
                                        Color32::WHITE
                                    } else {
                                        Color32::from_gray(210)
                                    };
                                    let dot_shadow = Color32::from_black_alpha(130);
                                    let dc = dots_rect.center();
                                    let dot_r = 2.0;
                                    let dot_gap = 6.0;
                                    for dy in [-dot_gap, 0.0, dot_gap] {
                                        let p = pos2(dc.x, dc.y + dy);
                                        // тень — чтобы точки читались поверх светлой обложки
                                        ui.painter().circle_filled(p + vec2(0.0, 1.0), dot_r + 0.4, dot_shadow);
                                        ui.painter().circle_filled(p, dot_r, dot_color);
                                    }

                                    // Меню по клику на точки (выпадает вниз)
                                    let dots_popup_id = ui.make_persistent_id(("dots_popup", song.as_str()));
                                    if dots_resp.clicked() {
                                        ui.ctx().memory_mut(|mem| mem.toggle_popup(dots_popup_id));
                                    }

                                    let mut dots_clicked = false;
                                    egui::popup::popup_below_widget(
                                        ui,
                                        dots_popup_id,
                                        &dots_resp,
                                        egui::popup::PopupCloseBehavior::CloseOnClickOutside,
                                        |ui| {
                                            style_menu(ui);
                                            ui.set_min_width(210.0);
                                            for p_idx in 0..self.playlists.len() {
                                                if self.playlists[p_idx].name == "Понравившаяся музыка" {
                                                    continue;
                                                }
                                                let p_name = self.playlists[p_idx].name.clone();
                                                let already_in = self.playlists[p_idx].songs.contains(&song);
                                                let text_color = if already_in { accent_color } else { Color32::WHITE };

                                                let btn = ui.add(
                                                    egui::Button::new(
                                                        RichText::new(format!("      {}", p_name)) // отступ слева под галочку
                                                            .size(14.0)
                                                            .color(text_color),
                                                    )
                                                    .min_size(vec2(202.0, 32.0))
                                                    .rounding(6.0),
                                                );

                                                // Галочку рисуем линиями — не зависит от шрифта, никаких квадратов
                                                if already_in {
                                                    let r = btn.rect;
                                                    let cx = r.left() + 15.0;
                                                    let cy = r.center().y;
                                                    let stroke = Stroke::new(2.0, accent_color);
                                                    ui.painter().line_segment([pos2(cx - 5.0, cy + 1.0), pos2(cx - 1.0, cy + 5.0)], stroke);
                                                    ui.painter().line_segment([pos2(cx - 1.0, cy + 5.0), pos2(cx + 6.0, cy - 5.0)], stroke);
                                                }

                                                if btn.clicked() {
                                                    dots_clicked = true;
                                                    if !already_in {
                                                        self.playlists[p_idx].songs.push(song.clone());
                                                        self.save_playlists();
                                                    }
                                                    ui.ctx().memory_mut(|m| m.close_popup());
                                                }
                                            }
                                        },
                                    );
                                    // Контекстное меню по правому клику на саму карточку
                                    response.context_menu(|ui| {
                                        style_menu(ui);
                                        ui.set_min_width(210.0);
                                        for p_idx in 0..self.playlists.len() {
                                            if self.playlists[p_idx].name == "Понравившаяся музыка" {
                                                continue;
                                            }
                                            let p_name = self.playlists[p_idx].name.clone();
                                            let already_in = self.playlists[p_idx].songs.contains(&song);
                                            let text_color = if already_in { accent_color } else { Color32::WHITE };

                                            let btn = ui.add(
                                                egui::Button::new(
                                                    RichText::new(format!("      {}", p_name))
                                                        .size(14.0)
                                                        .color(text_color),
                                                )
                                                .min_size(vec2(202.0, 32.0))
                                                .rounding(6.0),
                                            );

                                            if already_in {
                                                let r = btn.rect;
                                                let cx = r.left() + 15.0;
                                                let cy = r.center().y;
                                                let stroke = Stroke::new(2.0, accent_color);
                                                ui.painter().line_segment([pos2(cx - 5.0, cy + 1.0), pos2(cx - 1.0, cy + 5.0)], stroke);
                                                ui.painter().line_segment([pos2(cx - 1.0, cy + 5.0), pos2(cx + 6.0, cy - 5.0)], stroke);
                                            }

                                            if btn.clicked() {
                                                if !already_in {
                                                    self.playlists[p_idx].songs.push(song.clone());
                                                    self.save_playlists();
                                                }
                                                ui.close_menu();
                                            }
                                        }
                                    });

                                    // Появление большой кнопки Play (круглый зеленый кружок) при наведении на карточку
                                    if (is_hovered || is_active) && !dots_resp.hovered() {
                                        let btn_radius = 22.0;
                                        let btn_center = cover_rect.max - Vec2::new(btn_radius + 4.0, btn_radius + 4.0);

                                        ui.painter().circle_filled(btn_center + Vec2::new(0.0, 2.0), btn_radius, Color32::from_black_alpha(100));
                                        ui.painter().circle_filled(btn_center, btn_radius, accent_color);

                                        let icon = if is_active && self.is_playing { "⏸" } else { "▶" };
                                        ui.painter().text(btn_center, egui::Align2::CENTER_CENTER, icon, FontId::proportional(20.0), Color32::BLACK);
                                    }

                                    // Клик по карточке включает трек, только если кликнули МИМО всех кнопок управления и меню
                                    if response.clicked() && !heart_click.clicked() && !dots_resp.clicked() && !dots_clicked {
                                        if is_active {
                                            if self.is_playing { self.player.pause(); self.is_playing = false; }
                                            else { self.player.resume(); self.is_playing = true; }
                                        } else {
                                            self.playback_queue = home_queue.clone();
                                            self.play_track(&song);
                                        }
                                    }
                                }
                            });
                        });
                }
            });

        // =============================================================
        // ⚙ ОКНО НАСТРОЕК — на весь экран, модально (клики за ним заблокированы)
        // =============================================================
        if self.show_settings {

            let screen = ctx.screen_rect();

            egui::Area::new(egui::Id::new("settings_overlay"))
                .order(egui::Order::Foreground) // поверх всех панелей
                .interactable(true)
                .fixed_pos(screen.min)
                .show(ctx, |ui| {
                    // Рисуем на весь экран, без обрезки по авто-размеру области
                    ui.set_clip_rect(screen);

                    // Непрозрачный фон на весь экран + перехват ВСЕХ кликов,
                    // чтобы элементы за окном были недоступны.
                    let _ = ui.allocate_rect(screen, egui::Sense::click_and_drag());
                    ui.painter()
                        .rect_filled(screen, Rounding::same(0.0), Color32::from_rgb(18, 18, 18));

                    // Содержимое настроек поверх фона (с отступом от краёв)
                    let mut content = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(screen.shrink(40.0))
                            .layout(egui::Layout::top_down(egui::Align::Min)),
                    );

                    // Верхняя строка: заголовок слева, кнопка закрытия справа
                    content.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("⚙  {}", s.settings))
                                .size(28.0)
                                .strong()
                                .color(Color32::WHITE),
                        );
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

                    // --- Выбор языка интерфейса ---
                    content.add_space(28.0);
                    content.label(
                        RichText::new(s.language)
                            .size(18.0)
                            .strong()
                            .color(Color32::WHITE),
                    );
                    content.add_space(12.0);
                    content.horizontal(|ui| {
                        // Кнопка для каждого языка из Lang::all()
                        for &lang in Lang::all() {
                            let active = self.language == lang;
                            let bg = if active {
                                accent_color
                            } else {
                                Color32::from_rgb(35, 35, 35)
                            };
                            let fg = if active { Color32::BLACK } else { Color32::WHITE };
                            let btn = ui.add(
                                egui::Button::new(
                                    RichText::new(lang.native_name()).size(15.0).color(fg),
                                )
                                .min_size(vec2(160.0, 42.0))
                                .rounding(10.0)
                                .fill(bg),
                            );
                            if btn.clicked() {
                                self.language = lang; // переключаем язык
                                save_language(lang);  // и сразу запоминаем на диск
                            }
                            ui.add_space(12.0);
                        }
                    });

                    // --- Горячие клавиши ---
                    content.add_space(40.0);
                    content.label(
                        RichText::new(s.shortcuts)
                            .size(18.0)
                            .strong()
                            .color(Color32::WHITE),
                    );
                    content.add_space(12.0);
                    for &action in Shortcut::all() {
                        content.horizontal(|ui| {
                            // Название действия (фикс. ширина для выравнивания)
                            ui.allocate_ui_with_layout(
                                vec2(240.0, 30.0),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        RichText::new(action.label(self.language))
                                            .size(15.0)
                                            .color(Color32::WHITE),
                                    );
                                },
                            );

                            // Кнопка с текущей клавишей (клик — ждать нажатие)
                            let listening = self.rebinding == Some(action);
                            let key_text = if listening {
                                s.press_key.to_string()
                            } else {
                                match self.shortcuts.get(&action) {
                                    Some(&key) => key_label(key),
                                    None => s.not_set.to_string(),
                                }
                            };
                            let bg = if listening {
                                accent_color
                            } else {
                                Color32::from_rgb(35, 35, 35)
                            };
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

                            // Очистить назначение
                            let clear = ui.add(
                                egui::Button::new(RichText::new("🗑").size(14.0).color(text_muted))
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

                    // --- Прочие настройки (пока заглушка) ---
                    content.add_space(28.0);
                    content.horizontal(|ui| {
                        ui.label(RichText::new("🛠").size(20.0));
                        ui.add_space(8.0);
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new(s.settings_in_dev)
                                    .size(14.0)
                                    .strong()
                                    .color(Color32::WHITE),
                            );
                            ui.label(
                                RichText::new(s.settings_in_dev_sub)
                                    .size(12.0)
                                    .color(text_muted),
                            );
                        });
                    });
                });
        }

        // =============================================================
        // ➕ ОКНО «НОВЫЙ ПЛЕЙЛИСТ» — небольшое модальное окно по центру экрана
        // =============================================================
        if self.show_new_playlist {
            let screen = ctx.screen_rect();
            let win_rect = Rect::from_center_size(screen.center(), vec2(360.0, 190.0));

            egui::Area::new(egui::Id::new("new_playlist_overlay"))
                .order(egui::Order::Foreground)
                .interactable(true)
                .fixed_pos(screen.min)
                .show(ctx, |ui| {
                    ui.set_clip_rect(screen);

                    // Затемняем фон и перехватываем клики за окном
                    let _ = ui.allocate_rect(screen, egui::Sense::click_and_drag());
                    ui.painter().rect_filled(screen, Rounding::same(0.0), Color32::from_black_alpha(160));

                    // Esc закрывает окно
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        self.new_playlist_name.clear();
                        self.show_new_playlist = false;
                    }

                    // Карточка окна
                    ui.painter().rect_filled(win_rect, Rounding::same(14.0), Color32::from_rgb(28, 28, 28));

                    let mut content = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(win_rect.shrink(20.0))
                            .layout(egui::Layout::top_down(egui::Align::Min)),
                    );

                    content.label(RichText::new(s.new_playlist_title).size(20.0).strong().color(Color32::WHITE));
                    content.add_space(16.0);

                    // Поле ввода: свой скруглённый фон + безрамочный TextEdit по центру
                    let field_h = 44.0;
                    let (field_rect, _) = content.allocate_exact_size(
                        vec2(content.available_width(), field_h),
                        egui::Sense::hover(),
                    );

                    // Фон поля
                    content.painter().rect_filled(
                        field_rect,
                        Rounding::same(10.0),
                        Color32::from_rgb(20, 20, 20),
                    );

                    // Сам ввод — без рамки, с отступом слева, по центру по вертикали
                    let mut field_ui = content.new_child(
                        egui::UiBuilder::new()
                            .max_rect(field_rect.shrink2(vec2(14.0, 0.0)))
                            .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    );
                    let resp = field_ui.add(
                        egui::TextEdit::singleline(&mut self.new_playlist_name)
                            .hint_text(RichText::new(s.new_playlist_hint).color(text_muted))
                            .frame(false)
                            .desired_width(f32::INFINITY),
                    );

                    if self.focus_new_playlist {
                        resp.request_focus();
                        self.focus_new_playlist = false;
                    }

                    // Акцентная линия снизу, когда поле в фокусе
                    if resp.has_focus() {
                        let underline = Rect::from_min_max(
                            pos2(field_rect.left() + 6.0, field_rect.bottom() - 3.0),
                            pos2(field_rect.right() - 6.0, field_rect.bottom() - 1.0),
                        );
                        content.painter().rect_filled(underline, Rounding::same(2.0), accent_color);
                    }

                    let enter_pressed =
                        resp.lost_focus() && content.input(|i| i.key_pressed(egui::Key::Enter));

                    content.add_space(22.0);

                    let mut do_create = enter_pressed;
                    let mut do_cancel = false;
                    content.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Кнопка "Создать"
                        if ui.add(
                            egui::Button::new(RichText::new(s.create).size(15.0).color(Color32::BLACK))
                                .fill(accent_color)
                                .rounding(18.0)
                                .min_size(vec2(120.0, 36.0)),
                        ).clicked() {
                            do_create = true;
                        }
                        
                        ui.add_space(10.0);
                        
                        // Кнопка "Отмена"
                        if ui.add(
                            egui::Button::new(RichText::new(s.cancel).size(15.0).color(Color32::WHITE))
                                .fill(Color32::from_rgb(45, 45, 45))
                                .rounding(18.0)
                                .min_size(vec2(120.0, 36.0)),
                        ).clicked() {
                            do_cancel = true;
                        }
                    });

                    // --- ВОТ СЮДА НУЖНО ДОБАВИТЬ ЛОГИКУ, ГДЕ ВЫ ОБРАБАТЫВАЕТЕ do_create ---
                    if do_create {
                        let name = self.new_playlist_name.trim().to_string();
                        if !name.is_empty() {
                            // 1. Создаем папку
                            // Используем "../DownloadedMusic", так как это путь из вашего кода
                            let path = std::path::Path::new("../DownloadedMusic").join(&name);
                            if let Err(e) = std::fs::create_dir_all(&path) {
                                eprintln!("Ошибка при создании папки: {}", e);
                            }

                            // Если такой плейлист раньше удаляли — снимаем пометку,
                            // иначе после перезапуска он снова исчезнет.
                            let mut deleted = load_deleted_playlists();
                            if deleted.remove(&name) {
                                save_deleted_playlists(&deleted);
                            }

                            // 2. Добавляем в список
                            self.playlists.push(Playlist { name, songs: Vec::new() });
                            self.selected_playlist_idx = Some(self.playlists.len() - 1);
                            self.save_playlists();
                        }
                        self.new_playlist_name.clear();
                        self.show_new_playlist = false;
                    }

                    if do_cancel {
                        self.new_playlist_name.clear();
                        self.show_new_playlist = false;
                    } else if do_create {
                        let name = self.new_playlist_name.trim().to_string();
                        if !name.is_empty() {
                            self.playlists.push(Playlist { name, songs: Vec::new() });
                            self.selected_playlist_idx = Some(self.playlists.len() - 1);
                        }
                        self.new_playlist_name.clear();
                        self.show_new_playlist = false;
                    }
                });
        }

        // ═══════════════════════════════════════════════════════
        // Модальное окно ПЕРЕИМЕНОВАНИЯ плейлиста
        // ═══════════════════════════════════════════════════════
        if self.rename_playlist_idx.is_some() {
            let screen = ctx.screen_rect();
            let win_w = 400.0_f32;
            let win_h = 210.0_f32;
            let win_rect = Rect::from_center_size(screen.center(), vec2(win_w, win_h));
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

                    ui.painter().rect_filled(win_rect, Rounding::same(14.0), Color32::from_rgb(28, 28, 28));

                    let mut content = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(win_rect.shrink(20.0))
                            .layout(egui::Layout::top_down(egui::Align::Min)),
                    );

                    let title_label = match lang {
                        Lang::Ru => "Переименовать плейлист",
                        Lang::Uk => "Перейменувати плейлист",
                    };
                    content.label(RichText::new(title_label).size(20.0).strong().color(Color32::WHITE));
                    content.add_space(16.0);

                    let field_h = 44.0;
                    let (field_rect, _) = content.allocate_exact_size(
                        vec2(content.available_width(), field_h),
                        egui::Sense::hover(),
                    );
                    content.painter().rect_filled(field_rect, Rounding::same(10.0), Color32::from_rgb(20, 20, 20));

                    let mut field_ui = content.new_child(
                        egui::UiBuilder::new()
                            .max_rect(field_rect.shrink2(vec2(14.0, 0.0)))
                            .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    );
                    let hint = match lang { Lang::Ru => "Новое название", Lang::Uk => "Нова назва" };
                    let resp = field_ui.add(
                        egui::TextEdit::singleline(&mut self.rename_playlist_name)
                            .hint_text(RichText::new(hint).color(Color32::from_rgb(100, 100, 100)))
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
                        content.painter().rect_filled(underline, Rounding::same(2.0), Color32::from_rgb(29, 185, 84));
                    }

                    let enter_pressed = resp.lost_focus() && content.input(|i| i.key_pressed(egui::Key::Enter));
                    content.add_space(16.0);

                    let mut do_rename = enter_pressed;
                    let mut do_cancel = false;
                    content.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let save_label = match lang { Lang::Ru => "Сохранить", Lang::Uk => "Зберегти" };
                        if ui.add(
                            egui::Button::new(RichText::new(save_label).size(15.0).color(Color32::BLACK))
                                .fill(Color32::from_rgb(29, 185, 84))
                                .rounding(18.0)
                                .min_size(vec2(120.0, 36.0)),
                        ).clicked() { do_rename = true; }

                        ui.add_space(10.0);

                        let cancel_label = match lang { Lang::Ru => "Отмена", Lang::Uk => "Скасувати" };
                        if ui.add(
                            egui::Button::new(RichText::new(cancel_label).size(15.0).color(Color32::WHITE))
                                .fill(Color32::from_rgb(45, 45, 45))
                                .rounding(18.0)
                                .min_size(vec2(120.0, 36.0)),
                        ).clicked() { do_cancel = true; }
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

        // Создаем отдельное плавающее окно для текста песни
        egui::Window::new("🎤 Текст песни BETA!!").show(ctx, |ui| {
            if let Some(lyrics) = &self.current_lyrics {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (index, line) in lyrics.iter().enumerate() {
                        let is_active = {
                            let is_past_start = self.current_playback_time_ms >= line.time_ms;
                            let is_before_next = if let Some(next_line) = lyrics.get(index + 1) {
                                self.current_playback_time_ms < next_line.time_ms
                            } else {
                                true
                            };
                            is_past_start && is_before_next
                        };

                        let (color, font_size) = if is_active {
                            (egui::Color32::WHITE, 24.0)
                        } else {
                            (egui::Color32::GRAY, 18.0)
                        };

                        ui.label(
                            egui::RichText::new(&line.text)
                                .color(color)
                                .size(font_size)
                        );
                    }
                });
            } else {
                ui.label("Текст песни отсутствует 😔");
            }
        });

        // Подсказка, пока пользователь держит файлы над окном.
        let hovering_files = ctx.input(|i| !i.raw.hovered_files.is_empty());
        if hovering_files {
            let screen = ctx.screen_rect();
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("drop_files_overlay"),
            ));
            painter.rect_filled(screen, Rounding::same(0.0), Color32::from_black_alpha(150));
            let text = match self.language {
                Lang::Ru => "Отпустите, чтобы добавить в Elysium",
                Lang::Uk => "Відпустіть, щоб додати в Elysium",
            };
            painter.text(
                screen.center(),
                egui::Align2::CENTER_CENTER,
                text,
                FontId::proportional(28.0),
                Color32::WHITE,
            );
        }

        // ctx.request_repaint();
        // Запрашиваем перерисовку не сразу, а через 50 миллисекунд (~20 FPS)
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
}

fn main() {
    let mut options = eframe::NativeOptions::default();
    options.viewport = egui::ViewportBuilder::default()
        .with_inner_size([1200.0, 800.0])
        .with_min_inner_size([900.0, 600.0]);
        
    let _ = eframe::run_native(
        "Elysium",
        options,
        Box::new(|cc| {
            // ВАЖНО: Применяем шрифты до создания самого приложения!
            setup_custom_fonts(&cc.egui_ctx); 
            Ok(Box::new(App::new(&cc.egui_ctx)))
        }),
    );
}