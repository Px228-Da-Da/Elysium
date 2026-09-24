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

// The shared palette comes in two variants — a "mono / dark" set (the design
// mockup) and a light set. Each color is a function that returns the value for
// the currently active theme (see `is_light` / `set_light_theme`), so switching
// theme recolors the whole UI without touching call sites.

/// Whether the light theme is active. `false` = dark (the default).
static LIGHT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Selects the light (`true`) or dark (`false`) palette globally.
pub fn set_light_theme(on: bool) {
    LIGHT.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Whether the light theme is currently active.
pub fn is_light() -> bool {
    LIGHT.load(std::sync::atomic::Ordering::Relaxed)
}

/// Picks the dark or light value for the current theme.
#[inline]
fn pick(dark: Color32, light: Color32) -> Color32 {
    if is_light() {
        light
    } else {
        dark
    }
}

/// Main window background.
pub fn bg_main() -> Color32 {
    pick(Color32::from_rgb(13, 13, 16), Color32::from_rgb(245, 245, 248))
}
/// Card / panel surface.
pub fn surface() -> Color32 {
    pick(Color32::from_rgb(21, 21, 25), Color32::from_rgb(255, 255, 255))
}
/// Raised surface (hovered cards, key chips).
pub fn surface_2() -> Color32 {
    pick(Color32::from_rgb(28, 28, 34), Color32::from_rgb(236, 236, 241))
}
/// Hairline borders / dividers.
pub fn line() -> Color32 {
    pick(Color32::from_rgb(39, 39, 48), Color32::from_rgb(221, 221, 229))
}
/// Primary text.
pub fn text() -> Color32 {
    pick(Color32::from_rgb(242, 242, 245), Color32::from_rgb(26, 26, 32))
}
/// Default brand accent (soft periwinkle purple), used until the user picks one.
pub const ACCENT_DEFAULT: Color32 = Color32::from_rgb(143, 138, 218);

/// The user-selected accent, packed as `0x01_RR_GG_BB` (the high byte is a
/// "set" flag). `0` means unset → [`ACCENT_DEFAULT`]. Stored in an atomic so any
/// UI code can read it cheaply every frame without locking.
static ACCENT_RGB: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Overrides the global accent color (from the saved config / onboarding choice).
pub fn set_accent(c: Color32) {
    let packed = 0x0100_0000 | ((c.r() as u32) << 16) | ((c.g() as u32) << 8) | c.b() as u32;
    ACCENT_RGB.store(packed, std::sync::atomic::Ordering::Relaxed);
}

/// The current global accent color: the user's onboarding choice, or the default.
pub fn accent() -> Color32 {
    let v = ACCENT_RGB.load(std::sync::atomic::Ordering::Relaxed);
    if v == 0 {
        ACCENT_DEFAULT
    } else {
        Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
    }
}
/// Muted gray used for secondary text (artists, hints, captions).
pub fn text_muted() -> Color32 {
    pick(Color32::from_rgb(148, 148, 160), Color32::from_rgb(104, 104, 116))
}
/// Faintest text (captions, section labels).
pub fn text_faint() -> Color32 {
    pick(Color32::from_rgb(92, 92, 102), Color32::from_rgb(150, 150, 162))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Converts HSV (`h` in 0..360, `s`/`v` in 0..1) to an opaque [`Color32`].
pub fn hsv(h: f32, s: f32, v: f32) -> Color32 {
    let c = v * s;
    let hp = (h.rem_euclid(360.0)) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    Color32::from_rgb(
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    )
}

/// Stable hue (0..360) derived from `seed` via an FNV-1a hash.
fn hue_of(seed: &str) -> f32 {
    let mut hash: u32 = 2166136261;
    for b in seed.bytes() {
        hash ^= b as u32;
        hash = hash.wrapping_mul(16777619);
    }
    (hash % 360) as f32
}

/// A deterministic, vivid two-stop gradient (top-left → bottom-right) for a
/// generated cover, derived from `seed`. Matches the mockup's diagonal tint.
pub fn gen_gradient(seed: &str) -> (Color32, Color32) {
    let hue = hue_of(seed);
    (hsv(hue, 0.62, 0.70), hsv((hue + 40.0) % 360.0, 0.66, 0.40))
}

/// The bright, colored glyph color for a generated cover's big label (a light,
/// saturated tint of the cover's hue), matching the mockup's blended look.
pub fn gen_glyph(seed: &str) -> Color32 {
    hsv(hue_of(seed), 0.55, 1.0)
}

/// Short label drawn big on a generated cover, e.g. `"#"`, `"104"`, `"50"`,
/// `"2r"`. A leading symbol (like `#`) is shown alone; otherwise up to three
/// leading alphanumeric characters are used.
pub fn cover_label(title: &str) -> String {
    let t = title.trim();
    match t.chars().next() {
        // Leading non-alphanumeric symbol → show it by itself.
        Some(c) if !c.is_alphanumeric() => c.to_string(),
        // Otherwise up to three leading alphanumeric characters.
        Some(_) => t.chars().take_while(|c| c.is_alphanumeric()).take(3).collect(),
        None => String::new(),
    }
}

/// Builds a clockwise point path around a rounded rectangle.
fn rrect_perimeter(rect: egui::Rect, r: f32) -> Vec<egui::Pos2> {
    use std::f32::consts::PI;
    let mut pts = Vec::new();
    let mut arc = |cx: f32, cy: f32, a0: f32, a1: f32| {
        let n = 6;
        for i in 0..=n {
            let a = a0 + (a1 - a0) * (i as f32 / n as f32);
            pts.push(egui::pos2(cx + r * a.cos(), cy + r * a.sin()));
        }
    };
    arc(rect.left() + r, rect.top() + r, PI, 1.5 * PI);
    arc(rect.right() - r, rect.top() + r, 1.5 * PI, 2.0 * PI);
    arc(rect.right() - r, rect.bottom() - r, 0.0, 0.5 * PI);
    arc(rect.left() + r, rect.bottom() - r, 0.5 * PI, PI);
    if let Some(&first) = pts.first() {
        pts.push(first);
    }
    pts
}

/// Fills a rounded rectangle with a **diagonal** gradient (`top` at the
/// top-left → `bottom` at the bottom-right, ~145°), following the rounded
/// corners via a triangle fan from the center. Matches the mockup's tint.
pub fn gradient_rrect(p: &egui::Painter, rect: egui::Rect, rounding: f32, top: Color32, bottom: Color32) {
    let r = rounding.min(rect.width() / 2.0).min(rect.height() / 2.0);
    let path = rrect_perimeter(rect, r);
    let col_at = |pos: egui::Pos2| {
        // Diagonal parameter: 0 at top-left corner, 1 at bottom-right corner.
        let fx = (pos.x - rect.left()) / rect.width().max(1.0);
        let fy = (pos.y - rect.top()) / rect.height().max(1.0);
        let t = ((fx + fy) * 0.5).clamp(0.0, 1.0);
        let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t) as u8;
        Color32::from_rgb(mix(top.r(), bottom.r()), mix(top.g(), bottom.g()), mix(top.b(), bottom.b()))
    };
    let mut mesh = egui::epaint::Mesh::default();
    mesh.colored_vertex(rect.center(), col_at(rect.center()));
    for pt in &path {
        mesh.colored_vertex(*pt, col_at(*pt));
    }
    let n = path.len() as u32;
    for i in 0..n.saturating_sub(1) {
        mesh.add_triangle(0, 1 + i, 2 + i);
    }
    p.add(mesh);
}

/// Truncates `text` with an ellipsis so it fits within `max_w` pixels when laid
/// out on a single line in `font`, measuring the *actual* glyph widths (so wide
/// Cyrillic text can't spill out of its row). Returns the string to draw.
pub fn fit_text(ui: &egui::Ui, text: &str, font: egui::FontId, max_w: f32) -> String {
    let width = |s: &str| {
        ui.fonts(|f| f.layout_no_wrap(s.to_string(), font.clone(), Color32::WHITE).rect.width())
    };
    if width(text) <= max_w {
        return text.to_string();
    }
    // Binary-search the longest prefix that still fits once an ellipsis is added.
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        let candidate: String = chars[..mid].iter().collect::<String>() + "…";
        if width(&candidate) <= max_w {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    chars[..lo].iter().collect::<String>() + "…"
}

/// Linearly interpolates between two opaque colors (`t` in 0..1: 0 → `a`,
/// 1 → `b`). Used for the subtle cover-adaptive background tint.
pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// Formats a duration as `M:SS` (e.g. `3:07`). Hours are not expected for
/// individual tracks and intentionally roll into the minutes field.
pub fn format_duration(d: Duration) -> String {
    let total_secs = d.as_secs();
    let mins = total_secs / 60;
    let secs = total_secs % 60;
    format!("{}:{:02}", mins, secs)
}

/// Applies a flat, borderless style to menu items (used for both dropdown and
/// right-click context menus): transparent at rest, a soft theme-appropriate
/// highlight on hover, primary text when hovered or active. Theme-aware so the
/// menu is readable in both dark and light themes.
pub fn style_menu(ui: &mut egui::Ui) {
    // The window/menu background itself follows the theme.
    let bg = surface();
    let hover = surface_2();
    let active = line();
    let fg = text();

    let v = ui.visuals_mut();
    v.window_fill = bg;
    v.panel_fill = bg;

    v.widgets.inactive.bg_stroke = Stroke::NONE;
    v.widgets.hovered.bg_stroke = Stroke::NONE;
    v.widgets.active.bg_stroke = Stroke::NONE;

    v.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    v.widgets.inactive.bg_fill = Color32::TRANSPARENT;
    v.widgets.inactive.fg_stroke.color = fg;

    v.widgets.hovered.weak_bg_fill = hover;
    v.widgets.hovered.bg_fill = hover;
    v.widgets.active.weak_bg_fill = active;
    v.widgets.active.bg_fill = active;

    v.widgets.hovered.fg_stroke.color = fg;
    v.widgets.active.fg_stroke.color = fg;
}

/// Installs the global theme (dark or light base + the user's accent) into
/// `ctx`, following [`is_light`].
///
/// Re-applied whenever the theme changes; egui caches the visuals, so this is
/// cheap and keeps the theme correct even after egui resets internal state.
pub fn apply_custom_theme(ctx: &egui::Context) {
    let mut visuals = if is_light() {
        egui::Visuals::light()
    } else {
        egui::Visuals::dark()
    };

    visuals.panel_fill = bg_main();
    visuals.window_fill = bg_main();
    visuals.selection.bg_fill = accent();

    visuals.widgets.inactive.bg_fill = surface_2();
    visuals.widgets.inactive.bg_stroke = Stroke::NONE;
    visuals.widgets.inactive.fg_stroke.color = text();

    visuals.widgets.hovered.bg_fill = accent();
    visuals.widgets.hovered.fg_stroke.color = Color32::WHITE;

    visuals.widgets.active.bg_fill = accent();
    visuals.widgets.active.fg_stroke.color = Color32::WHITE;

    ctx.set_visuals(visuals);
    ctx.style_mut(|style| {
        // Track rows draw their own text; disable egui's label selection so
        // clicks always register as "play", never as text selection.
        style.interaction.selectable_labels = false;
        style.interaction.multi_widget_text_select = false;
    });
}
