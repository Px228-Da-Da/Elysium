//! Visual theme, shared colors and small formatting helpers.
//!
//! The app uses a dark, Spotify-like palette with a single green accent. The
//! named color constants here are reused across every UI module so the look
//! stays consistent and tweaking a shade only means editing one place.

use eframe::egui;
use egui::{Color32, Stroke};
use std::time::Duration;

// ---------------------------------------------------------------------------
// Shared palette
// ---------------------------------------------------------------------------

/// Main window background (near-black).
pub const BG_MAIN: Color32 = Color32::from_rgb(18, 18, 18);
/// Brand accent color (green) used for highlights, active items and primary
/// buttons.
pub const ACCENT: Color32 = Color32::from_rgb(29, 185, 84);
/// Muted gray used for secondary text (artists, hints, captions).
pub const TEXT_MUTED: Color32 = Color32::from_rgb(167, 167, 167);

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Formats a duration as `M:SS` (e.g. `3:07`). Hours are not expected for
/// individual tracks and intentionally roll into the minutes field.
pub fn format_duration(d: Duration) -> String {
    let total_secs = d.as_secs();
    let mins = total_secs / 60;
    let secs = total_secs % 60;
    format!("{}:{:02}", mins, secs)
}

/// Applies a flat, borderless style to menu items (used for both dropdown and
/// right-click context menus): transparent at rest, a soft gray highlight on
/// hover, white text when hovered or active.
pub fn style_menu(ui: &mut egui::Ui) {
    let v = ui.visuals_mut();
    v.widgets.inactive.bg_stroke = Stroke::NONE;
    v.widgets.hovered.bg_stroke = Stroke::NONE;
    v.widgets.active.bg_stroke = Stroke::NONE;

    v.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    v.widgets.inactive.bg_fill = Color32::TRANSPARENT;

    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(48, 48, 48);
    v.widgets.hovered.bg_fill = Color32::from_rgb(48, 48, 48);
    v.widgets.active.weak_bg_fill = Color32::from_rgb(60, 60, 60);
    v.widgets.active.bg_fill = Color32::from_rgb(60, 60, 60);

    v.widgets.hovered.fg_stroke.color = Color32::WHITE;
    v.widgets.active.fg_stroke.color = Color32::WHITE;
}

/// Installs the global dark theme (dark base + green accent) into `ctx`.
///
/// Called once per frame; egui caches the visuals, so re-applying is cheap and
/// keeps the theme correct even after egui resets internal state.
pub fn apply_custom_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();

    visuals.panel_fill = Color32::from_rgb(0, 0, 0);
    visuals.window_fill = BG_MAIN;
    visuals.selection.bg_fill = Color32::WHITE;

    visuals.widgets.inactive.bg_fill = Color32::from_rgb(77, 77, 77);
    visuals.widgets.inactive.bg_stroke = Stroke::NONE;
    visuals.widgets.inactive.fg_stroke.color = Color32::from_rgb(179, 179, 179);

    visuals.widgets.hovered.bg_fill = ACCENT;
    visuals.widgets.hovered.fg_stroke.color = Color32::WHITE;

    visuals.widgets.active.bg_fill = ACCENT;
    visuals.widgets.active.fg_stroke.color = Color32::WHITE;

    ctx.set_visuals(visuals);
    ctx.style_mut(|style| {
        // Track rows draw their own text; disable egui's label selection so
        // clicks always register as "play", never as text selection.
        style.interaction.selectable_labels = false;
        style.interaction.multi_widget_text_select = false;
    });
}
