//! Keyboard handling: in-window rebinding and global media hotkeys.
//!
//! Two complementary paths exist:
//! * [`App::handle_shortcuts`] runs only when the window has focus and deals
//!   with the settings UI: capturing a key while rebinding, and Esc.
//! * [`App::handle_global_keys`] drives the *global* hotkeys captured by the
//!   `rdev::grab` thread (see [`App::new`]), which fire even when the window is
//!   unfocused. It also tells that thread which keys to swallow and whether
//!   grabbing should be active right now.

use super::App;
use crate::shortcuts::save_shortcuts;
use eframe::egui;

impl App {
    /// Per-frame, focused-window keyboard handling.
    ///
    /// While an action is being rebound, the next key press is captured as its
    /// new binding (Esc cancels). Otherwise Esc closes the settings overlay.
    /// Normal hotkey firing is handled globally by [`Self::handle_global_keys`].
    pub(super) fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let esc = ctx.input(|i| i.key_pressed(egui::Key::Escape));

        // Waiting for a key to assign to `action`.
        if let Some(action) = self.rebinding {
            if esc {
                self.rebinding = None; // Esc cancels
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

        // Esc closes the settings overlay.
        if self.show_settings && esc {
            self.show_settings = false;
        }
    }

    /// Drives global hotkeys captured by the `rdev::grab` thread.
    ///
    /// First it syncs [`App::grab_shared`]: which keys to swallow, and whether
    /// grabbing is active (paused while in settings, rebinding, or typing, so
    /// keys can be entered/assigned normally). Then it drains captured key
    /// presses and runs the matching action.
    pub(super) fn handle_global_keys(&mut self, ctx: &egui::Context) {
        {
            // Tolerate a poisoned lock (a panicked grab thread) instead of
            // panicking the UI every frame — the shared state is trivially
            // recoverable.
            let mut st = self
                .grab_shared
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            // Do not grab in settings / while rebinding / while typing in a text
            // field — otherwise the key could neither be typed nor assigned.
            st.active =
                !(self.show_settings || self.rebinding.is_some() || ctx.wants_keyboard_input());
            st.keys = self.shortcuts.values().copied().collect();
        }

        // Run actions for any keys the grab thread captured this frame.
        while let Ok(key) = self.global_key_rx.try_recv() {
            if self.show_settings || self.rebinding.is_some() {
                continue; // drain the queue but ignore presses
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
}
