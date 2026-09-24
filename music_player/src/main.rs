//! Elysium — a desktop music player built with `eframe`/`egui`.
//!
//! `main` is intentionally tiny: it installs the bundled fonts, configures the
//! native window, and hands control to [`app::App`]. All real behavior lives in
//! the modules below. See `AGENTS.md` for an architecture overview.

// Use the Windows GUI subsystem in release builds so launching the .exe does not
// open a console window. Debug builds keep the console so `println!` diagnostics
// are visible while developing. Has no effect on other platforms.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod audio;
mod config;
mod icons;
mod lab;
mod lang;
mod meta;
mod player;
mod scanner;
mod shortcuts;
mod theme;
mod ytdl;

use app::App;
use eframe::egui;

/// Registers the bundled fonts so the UI can render Latin, Cyrillic, accented
/// characters and emoji.
///
/// Font order within a family is the fallback order egui uses per glyph: the
/// main text font comes first, with the emoji font appended as a fallback for
/// characters the main font lacks.
fn setup_custom_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    // Primary text: Latin + Cyrillic + diacritics.
    fonts.font_data.insert(
        "noto_sans".to_owned(),
        egui::FontData::from_static(include_bytes!("fonts/ttf/NotoSans-Regular.ttf")),
    );
    // Emoji glyphs.
    fonts.font_data.insert(
        "emoji_font".to_owned(),
        egui::FontData::from_static(include_bytes!("fonts/ttf/NotoEmoji-VariableFont_wght.ttf")),
    );

    // Proportional family: main font first, emoji as the fallback.
    let prop = fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default();
    prop.insert(0, "noto_sans".to_owned());
    prop.push("emoji_font".to_owned());

    // Give the monospace family Cyrillic coverage as a fallback too.
    fonts
        .families
        .entry(egui::FontFamily::Monospace)
        .or_default()
        .push("noto_sans".to_owned());

    ctx.set_fonts(fonts);
}

/// Configures the native window and runs the egui event loop until the window
/// is closed.
fn main() {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([900.0, 600.0]),
        ..Default::default()
    };

    let _ = eframe::run_native(
        "Elysium",
        options,
        Box::new(|cc| {
            // Fonts must be installed before the app builds any UI.
            setup_custom_fonts(&cc.egui_ctx);
            Ok(Box::new(App::new(&cc.egui_ctx)))
        }),
    );
}
