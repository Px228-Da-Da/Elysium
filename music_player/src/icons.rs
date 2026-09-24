//! SVG UI icons, recolored to the current theme and drawn via egui's image
//! loaders.
//!
//! Each icon is embedded at build time from `src/icons/svg`. The source SVGs are
//! monochrome (black), so [`recolored`] strips their fills and injects a single
//! theme color, and the result is handed to egui keyed by a stable `bytes://`
//! URI so the rasterized texture is cached per (icon, color).

use eframe::egui;
use egui::{Color32, Rect, Sense, Ui, Vec2};

/// The set of available icons. Each maps to one embedded SVG.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Play,
    Pause,
    Heart,
    Search,
    Home,
    Gear,
    Close,
    Rename,
    Delete,
    Tools,
    Update,
    Hourglass,
    Music,
    Folder,
    Speaker,
    Account,
    Add,
    ArrowLeft,
    ArrowRight,
}

impl Icon {
    /// The embedded SVG source for this icon.
    fn svg(self) -> &'static str {
        match self {
            Icon::Play => include_str!("icons/svg/icons8-play-48.svg"),
            Icon::Pause => include_str!("icons/svg/icons8-pause-48.svg"),
            Icon::Heart => include_str!("icons/svg/icons8-love-48.svg"),
            Icon::Search => include_str!("icons/svg/icons8-search-48.svg"),
            Icon::Home => include_str!("icons/svg/icons8-home.svg"),
            Icon::Gear => include_str!("icons/svg/icons8-gear.svg"),
            Icon::Close => include_str!("icons/svg/icons8-close.svg"),
            Icon::Rename => include_str!("icons/svg/icons8-crayon-48.svg"),
            Icon::Delete => include_str!("icons/svg/icons8-delete.svg"),
            Icon::Tools => include_str!("icons/svg/icons8-tools-48.svg"),
            Icon::Update => include_str!("icons/svg/icons8-update-48.svg"),
            Icon::Hourglass => include_str!("icons/svg/icons8-hourglass-48.svg"),
            Icon::Music => include_str!("icons/svg/icons8-music.svg"),
            Icon::Folder => include_str!("icons/svg/icons8-opened-folder.svg"),
            Icon::Speaker => include_str!("icons/svg/icons8-speaker-48.svg"),
            Icon::Account => include_str!("icons/svg/icons8-account-male-48.svg"),
            Icon::Add => include_str!("icons/svg/icons8-add-48.svg"),
            Icon::ArrowLeft => include_str!("icons/svg/left-arrow.svg"),
            Icon::ArrowRight => include_str!("icons/svg/right-arrow.svg"),
        }
    }

    /// A short stable key used in the texture cache URI.
    fn key(self) -> &'static str {
        match self {
            Icon::Play => "play",
            Icon::Pause => "pause",
            Icon::Heart => "heart",
            Icon::Search => "search",
            Icon::Home => "home",
            Icon::Gear => "gear",
            Icon::Close => "close",
            Icon::Rename => "rename",
            Icon::Delete => "delete",
            Icon::Tools => "tools",
            Icon::Update => "update",
            Icon::Hourglass => "hourglass",
            Icon::Music => "music",
            Icon::Folder => "folder",
            Icon::Speaker => "speaker",
            Icon::Account => "account",
            Icon::Add => "add",
            Icon::ArrowLeft => "arrow_left",
            Icon::ArrowRight => "arrow_right",
        }
    }
}

/// Rewrites a monochrome SVG so every path is filled with `color`: explicit
/// black fills are dropped and a root `fill` is injected, which the fills-less
/// paths then inherit.
fn recolored(svg: &str, color: Color32) -> String {
    let hex = format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b());
    let mut s = svg
        .replace("fill=\"#000000\"", "")
        .replace("fill=\"#000\"", "")
        .replace("fill=\"black\"", "")
        .replace("fill='#000000'", "")
        .replace("fill='#000'", "");
    if let Some(pos) = s.find("<svg") {
        s.insert_str(pos + 4, &format!(" fill=\"{hex}\""));
    }
    s
}

/// Builds a cached egui image for `icon` tinted to `color`.
fn image(icon: Icon, color: Color32) -> egui::Image<'static> {
    let uri = format!(
        "bytes://icon/{}/{:02x}{:02x}{:02x}.svg",
        icon.key(),
        color.r(),
        color.g(),
        color.b()
    );
    egui::Image::from_bytes(uri, recolored(icon.svg(), color).into_bytes())
}

/// Paints `icon` (tinted `color`) centered inside `rect`, preserving aspect.
pub fn paint(ui: &Ui, rect: Rect, icon: Icon, color: Color32) {
    image(icon, color)
        .fit_to_exact_size(rect.size())
        .paint_at(ui, rect);
}

/// Paints `icon` centered on `center` at the given square `size` (in points).
pub fn paint_at(ui: &Ui, center: egui::Pos2, size: f32, icon: Icon, color: Color32) {
    let rect = Rect::from_center_size(center, Vec2::splat(size));
    paint(ui, rect, icon, color);
}

/// A borderless, transparent icon button: allocates a `size`×`size` square,
/// paints `icon` (brighter on hover), shows a pointing-hand cursor, and returns
/// the click response.
pub fn button(ui: &mut Ui, size: f32, icon: Icon, color: Color32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let col = if resp.hovered() {
        color.gamma_multiply(1.35)
    } else {
        color
    };
    // Inset the glyph a little so the clickable area is comfortably larger.
    paint(ui, rect.shrink(size * 0.14), icon, col);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}
