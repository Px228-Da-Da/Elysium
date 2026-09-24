//! Interface localization.
//!
//! The app ships with two languages, Russian and Ukrainian. A [`Lang`] value
//! selects one of them, and [`strings`] returns the full [`Strings`] table of
//! UI labels for that language. The chosen language is persisted in the config
//! (see [`load_language`] / [`save_language`]).
//!
//! To add a language: extend the [`Lang`] enum, its small `match`es, and add a
//! matching arm to [`strings`] filling in every field.

use crate::config::{load_config, save_config};

/// A supported interface language.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    /// Russian.
    Ru,
    /// Ukrainian.
    Uk,
    /// English.
    En,
}

impl Lang {
    /// All languages, in the order their buttons are drawn in settings.
    pub fn all() -> &'static [Lang] {
        &[Lang::Ru, Lang::Uk, Lang::En]
    }

    /// The language's name written in that language (used as the button label).
    pub fn native_name(&self) -> &'static str {
        match self {
            Lang::Ru => "Русский",
            Lang::Uk => "Українська",
            Lang::En => "English",
        }
    }

    /// Short code persisted to the config file.
    pub fn code(&self) -> &'static str {
        match self {
            Lang::Ru => "ru",
            Lang::Uk => "uk",
            Lang::En => "en",
        }
    }

    /// Parses a saved code back into a [`Lang`]. Unknown codes default to Russian.
    pub fn from_code(code: &str) -> Lang {
        match code.trim() {
            "uk" => Lang::Uk,
            "en" => Lang::En,
            _ => Lang::Ru,
        }
    }
}

/// Reads the saved language, defaulting to Russian when none is stored.
pub fn load_language() -> Lang {
    let code = load_config().language;
    if code.is_empty() {
        Lang::Ru
    } else {
        Lang::from_code(&code)
    }
}

/// Persists the selected language to the config file.
pub fn save_language(lang: Lang) {
    let mut cfg = load_config();
    cfg.language = lang.code().to_string();
    save_config(&cfg);
}

/// Every UI label for a single language.
///
/// Fields are `&'static str`, so a `Strings` value is cheap to build and copy
/// around. A `{n}` placeholder inside a string marks where a number is
/// substituted at the call site via `.replace("{n}", ...)`.
pub struct Strings {
    pub search_hint: &'static str,
    pub user: &'static str,
    pub home: &'static str,
    pub new_playlist: &'static str,
    pub new_playlist_hint: &'static str,
    pub new_playlist_title: &'static str,
    pub create: &'static str,
    pub cancel: &'static str,
    pub liked_music: &'static str,
    pub auto_created: &'static str,
    pub liked_empty: &'static str,
    pub like_hint: &'static str,
    pub unlike_hint: &'static str,
    pub play: &'static str,
    pub delete_playlist: &'static str,
    pub sort: &'static str,
    pub listen_again: &'static str,
    pub playlist_tracks: &'static str,
    pub unknown_title: &'static str,
    pub unknown_artist: &'static str,
    pub settings: &'static str,
    pub settings_in_dev: &'static str,
    pub settings_in_dev_sub: &'static str,
    pub language: &'static str,
    pub shortcuts: &'static str,
    pub press_key: &'static str,
    pub not_set: &'static str,
    pub lyrics_searching: &'static str,
    pub lyrics_not_found: &'static str,
    pub update_title: &'static str,
    pub update_available: &'static str, // contains the {v} version placeholder
    pub update_now: &'static str,
    pub update_later: &'static str,
    pub update_downloading: &'static str,
    pub update_error: &'static str, // shown as a prefix before the error text

    // --- Lab (audio analysis) window ---
    /// Label of the button that opens the Lab, and the word after "Elysium —"
    /// in the Lab window title.
    pub lab_button: &'static str,
    /// Big title in the Lab header.
    pub lab_mode: &'static str,
    /// Caption before the widget toggle chips.
    pub lab_widgets: &'static str,
    /// Titles of the seven widgets, in toggle/draw order: waveform, spectrum,
    /// spectrogram, oscilloscope, vectorscope, loudness, L/R levels.
    pub lab_widget_names: [&'static str; 7],
    /// Shown in the waveform widget while it is being rendered off-thread.
    pub lab_computing: &'static str,
    /// Prefix for the vectorscope correlation readout ("corr 0.97").
    pub lab_corr: &'static str,

    // --- YouTube ("ЮБ") tab ---
    /// Short sidebar label for the tab. Temporarily unused while the ЮБ sidebar
    /// entry is commented out — kept so re-enabling it needs no lang changes.
    #[allow(dead_code)]
    pub yt_tab: &'static str,
    /// Placeholder text in the YouTube search box.
    pub yt_search_hint: &'static str,
    /// Label of the "search" button.
    pub yt_search_btn: &'static str,
    /// Shown while a search request is running.
    pub yt_searching: &'static str,
    /// Shown when a search returns nothing.
    pub yt_no_results: &'static str,
    /// Shown before the first search (empty state hint).
    pub yt_empty: &'static str,
    /// Shown on a row whose audio is being downloaded.
    pub yt_loading: &'static str,
}

/// Returns the full label table for `lang`.
///
/// To add a language, add another `match` arm here and fill in every field.
pub fn strings(lang: Lang) -> Strings {
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
            lyrics_searching: "Ищем текст песни…",
            lyrics_not_found: "Текст песни не найден",
            update_title: "Доступно обновление",
            update_available: "Версия {v} уже готова к установке.",
            update_now: "Обновить сейчас",
            update_later: "Позже",
            update_downloading: "Скачивание и установка…",
            update_error: "Ошибка: ",
            lab_button: "Лаб",
            lab_mode: "РЕЖИМ ЛАБ",
            lab_widgets: "Виджеты:",
            lab_widget_names: [
                "Форма волны",
                "Спектр",
                "Спектрограмма",
                "Осциллограф",
                "Вектороскоп",
                "Громкость",
                "Уровень L/R",
            ],
            lab_computing: "вычисление волны…",
            lab_corr: "корр",
            yt_tab: "ЮБ",
            yt_search_hint: "Поиск музыки на YouTube…",
            yt_search_btn: "Найти",
            yt_searching: "Поиск…",
            yt_no_results: "Ничего не найдено",
            yt_empty: "Введите запрос, чтобы найти музыку на YouTube.",
            yt_loading: "Загрузка…",
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
            lyrics_searching: "Шукаємо текст пісні…",
            lyrics_not_found: "Текст пісні не знайдено",
            update_title: "Доступне оновлення",
            update_available: "Версія {v} вже готова до встановлення.",
            update_now: "Оновити зараз",
            update_later: "Пізніше",
            update_downloading: "Завантаження та встановлення…",
            update_error: "Помилка: ",
            lab_button: "Лаб",
            lab_mode: "РЕЖИМ ЛАБ",
            lab_widgets: "Віджети:",
            lab_widget_names: [
                "Форма хвилі",
                "Спектр",
                "Спектрограма",
                "Осцилограф",
                "Вектороскоп",
                "Гучність",
                "Рівень L/R",
            ],
            lab_computing: "обчислення хвилі…",
            lab_corr: "кор",
            yt_tab: "ЮБ",
            yt_search_hint: "Пошук музики на YouTube…",
            yt_search_btn: "Знайти",
            yt_searching: "Пошук…",
            yt_no_results: "Нічого не знайдено",
            yt_empty: "Введіть запит, щоб знайти музику на YouTube.",
            yt_loading: "Завантаження…",
        },
        Lang::En => Strings {
            search_hint: "Search tracks, artists...",
            user: "👤 Profile",
            home: "Home",
            new_playlist: "➕ New",
            new_playlist_hint: "Playlist name",
            new_playlist_title: "New playlist",
            create: "Create",
            cancel: "Cancel",
            liked_music: "Liked music",
            auto_created: "📌 Created automatically",
            liked_empty: "Nothing here yet. Tracks you like will appear here.",
            like_hint: "Save to “Liked music”",
            unlike_hint: "Remove from “Liked music”",
            play: "   ▶  Play   ",
            delete_playlist: "Delete playlist",
            sort: "Sort",
            listen_again: "Listen again",
            playlist_tracks: "Playlist • {n} tracks",
            unknown_title: "Untitled",
            unknown_artist: "Unknown artist",
            settings: "Settings",
            settings_in_dev: "More settings in development",
            settings_in_dev_sub: "They will appear in one of the next updates.",
            language: "Language",
            shortcuts: "Hotkeys",
            press_key: "Press a key…",
            not_set: "not set",
            lyrics_searching: "Searching for lyrics…",
            lyrics_not_found: "Lyrics not found",
            update_title: "Update available",
            update_available: "Version {v} is ready to install.",
            update_now: "Update now",
            update_later: "Later",
            update_downloading: "Downloading and installing…",
            update_error: "Error: ",
            lab_button: "Lab",
            lab_mode: "LAB MODE",
            lab_widgets: "Widgets:",
            lab_widget_names: [
                "Waveform",
                "Spectrum",
                "Spectrogram",
                "Oscilloscope",
                "Vectorscope",
                "Loudness",
                "Level L/R",
            ],
            lab_computing: "computing waveform…",
            lab_corr: "corr",
            yt_tab: "YT",
            yt_search_hint: "Search music on YouTube…",
            yt_search_btn: "Search",
            yt_searching: "Searching…",
            yt_no_results: "Nothing found",
            yt_empty: "Type a query to find music on YouTube.",
            yt_loading: "Loading…",
        },
    }
}
