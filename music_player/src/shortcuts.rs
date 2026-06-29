//! Global media hotkeys: actions, key bindings and persistence.
//!
//! Each [`Shortcut`] is a player action (play/pause, next, like, ...). The app
//! maps every action to an `egui::Key`, lets the user rebind them in settings,
//! and saves the result to the config.
//!
//! Hotkeys are *global*: they fire even when the window is not focused (e.g.
//! while gaming). That is implemented with `rdev::grab` on a background thread,
//! which reports keys as `rdev::Key`. [`rdev_to_egui`] converts those into the
//! `egui::Key` values used everywhere else, and [`GrabShared`] is the small
//! piece of state shared between that thread and the UI.

use crate::config::{load_config, save_config};
use crate::lang::Lang;
use eframe::egui;
use std::collections::{HashMap, HashSet};

/// A bindable player action.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Shortcut {
    PlayPause,
    Next,
    Prev,
    VolumeUp,
    VolumeDown,
    ToggleLike,
}

impl Shortcut {
    /// All actions, in the order their rows appear in settings.
    pub fn all() -> &'static [Shortcut] {
        &[
            Shortcut::PlayPause,
            Shortcut::Next,
            Shortcut::Prev,
            Shortcut::VolumeUp,
            Shortcut::VolumeDown,
            Shortcut::ToggleLike,
        ]
    }

    /// Short, stable code used as the config key for this action.
    pub fn code(&self) -> &'static str {
        match self {
            Shortcut::PlayPause => "play_pause",
            Shortcut::Next => "next",
            Shortcut::Prev => "prev",
            Shortcut::VolumeUp => "vol_up",
            Shortcut::VolumeDown => "vol_down",
            Shortcut::ToggleLike => "like",
        }
    }

    /// Parses a config code back into a [`Shortcut`]; unknown codes yield `None`.
    pub fn from_code(code: &str) -> Option<Shortcut> {
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

    /// Localized, human-readable name of the action for the settings screen.
    pub fn label(&self, lang: Lang) -> &'static str {
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
            Lang::En => match self {
                Shortcut::PlayPause => "Play / Pause",
                Shortcut::Next => "Next track",
                Shortcut::Prev => "Previous track",
                Shortcut::VolumeUp => "Volume up",
                Shortcut::VolumeDown => "Volume down",
                Shortcut::ToggleLike => "Like / unlike",
            },
        }
    }
}

/// Keys whose bindings can be *saved* across restarts.
///
/// At rebind time the user may press any key, but only keys in this list have a
/// stable name that survives a restart. To support another key, just add it
/// here (e.g. `egui::Key::Backslash` for "\").
pub const BINDABLE_KEYS: &[egui::Key] = &[
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

/// Human-readable, stable name of a key for display and storage, e.g. `"Space"`,
/// `"ArrowRight"`, `"L"`. Derived from the key's `Debug` representation.
pub fn key_label(key: egui::Key) -> String {
    format!("{:?}", key)
}

/// Inverse of [`key_label`]: looks a key up by name among [`BINDABLE_KEYS`].
pub fn key_from_label(name: &str) -> Option<egui::Key> {
    BINDABLE_KEYS.iter().copied().find(|k| key_label(*k) == name)
}

/// The default key bindings used on first run or for unbound actions.
pub fn default_shortcuts() -> HashMap<Shortcut, egui::Key> {
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

/// Loads saved bindings, starting from the defaults.
///
/// A stored value of `"None"` explicitly *unbinds* an action (removing the
/// default), while a recognized key name overrides it. Unknown names are
/// ignored, leaving the default in place.
pub fn load_shortcuts() -> HashMap<Shortcut, egui::Key> {
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

/// Saves the current bindings, writing `"None"` for any unbound action so the
/// unbinding survives a restart.
pub fn save_shortcuts(map: &HashMap<Shortcut, egui::Key>) {
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

/// State shared between the global key-grabbing thread (`rdev::grab`) and the
/// UI thread.
///
/// The grab thread reads this to decide whether to "swallow" a key (consume it
/// so the OS and other apps never see it) and whether grabbing is active at all
/// (it is paused while the user is typing or rebinding).
pub struct GrabShared {
    /// Keys currently bound to actions — these are the ones to swallow.
    pub keys: HashSet<egui::Key>,
    /// Whether global grabbing is active right now.
    pub active: bool,
}

/// Converts an `rdev::Key` (from the global hook) to the matching `egui::Key`.
///
/// Returns `None` for keys we do not handle, so unmapped keys pass straight
/// through to the OS untouched.
pub fn rdev_to_egui(k: rdev::Key) -> Option<egui::Key> {
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
