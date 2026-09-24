//! First-run welcome wizard.
//!
//! Shown once, the very first time the app is launched (detected by the absence
//! of a saved config — see [`App::new`]). It walks the user through three steps
//! — language, appearance and music folders — then hands off to a short loading
//! screen while the library is scanned.
//!
//! The appearance choices (theme / accent) are only *saved* here; applying a
//! light theme app-wide is a separate change. Language and the chosen music
//! folders take effect immediately.

use super::App;
use crate::config::{load_config, save_config};
use crate::lang::Lang;
use crate::shortcuts::{key_label, save_shortcuts, Shortcut};
use eframe::egui;
use egui::{pos2, vec2, Align, Align2, Color32, FontId, Layout, Rect, RichText, Rounding, Sense, Stroke};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;

// --- Palette (self-contained; the wizard does not use the app theme) ---
const RIGHT_BG: Color32 = Color32::from_rgb(13, 13, 15);
const LEFT_BG: Color32 = Color32::from_rgb(26, 24, 30);
const CARD_BG: Color32 = Color32::from_rgb(24, 24, 28);
const CARD_BORDER: Color32 = Color32::from_rgb(46, 46, 52);
const MUTED: Color32 = Color32::from_rgb(150, 150, 156);
const FAINT: Color32 = Color32::from_rgb(110, 110, 116);

/// The four selectable accent colors: `(config key, color)`.
pub(crate) const ACCENTS: [(&str, Color32); 4] = [
    ("purple", Color32::from_rgb(139, 127, 217)),
    ("teal", Color32::from_rgb(23, 184, 196)),
    ("green", Color32::from_rgb(91, 190, 130)),
    ("coral", Color32::from_rgb(238, 122, 95)),
];

/// Multilingual greetings cycled (with a fade) on the left panel. Limited to
/// scripts the bundled font can render (Latin + Cyrillic) — no CJK, which would
/// otherwise show as empty boxes.
const GREETINGS: [&str; 10] = [
    "Hello", "Hola", "Привіт", "Bonjour", "Ciao", "Hallo", "Olá", "Привет", "Salam", "Merhaba",
];

/// Seconds each greeting stays before cross-fading to the next.
const GREETING_INTERVAL: f32 = 2.6;
/// Fade in/out duration at each end of the interval.
const GREETING_FADE: f32 = 0.5;

/// A candidate music source (folder or single file) shown on the last step.
struct FolderChoice {
    kind: FolderKind,
    path: String,
    selected: bool,
    /// `true` when this row is a single dragged-in file rather than a folder.
    is_file: bool,
}

enum FolderKind {
    Music,
    Downloads,
    Desktop,
    /// Manually picked or dropped folder; carries its display name.
    Custom(String),
}

/// Live state of the welcome wizard.
pub(crate) struct Onboarding {
    /// 0 = language, 1 = appearance, 2 = hotkeys, 3 = music folders.
    step: u8,
    lang: Lang,
    theme_light: bool,
    accent_idx: usize,
    /// Index into [`GREETINGS`] of the greeting currently showing.
    greeting_idx: usize,
    /// When the current greeting started (drives the fade + advance).
    greeting_at: Instant,
    /// Global-hotkey bindings the user assigns on the hotkeys step. Starts empty
    /// (every action "unassigned", like the mockup).
    shortcuts: HashMap<Shortcut, egui::Key>,
    /// Action currently waiting for a key press, if any.
    rebinding: Option<Shortcut>,
    folders: Vec<FolderChoice>,
    /// Recursive audio-file counts per folder path, filled by background threads.
    counts: Arc<Mutex<HashMap<String, usize>>>,
}

impl Onboarding {
    pub(crate) fn new() -> Self {
        let lang = crate::lang::load_language();
        let counts: Arc<Mutex<HashMap<String, usize>>> = Arc::new(Mutex::new(HashMap::new()));

        // Suggested sources: the OS Music, Downloads and Desktop folders.
        let mut folders = Vec::new();
        let suggested = [
            (FolderKind::Music, dirs::audio_dir()),
            (FolderKind::Downloads, dirs::download_dir()),
            (FolderKind::Desktop, dirs::desktop_dir()),
        ];
        for (i, (kind, dir)) in suggested.into_iter().enumerate() {
            if let Some(dir) = dir {
                if dir.is_dir() {
                    let path = dir.to_string_lossy().to_string();
                    // Count audio files off-thread (can be slow for big folders).
                    let (p, c) = (path.clone(), counts.clone());
                    std::thread::spawn(move || {
                        let mut out = Vec::new();
                        crate::audio::collect_audio_files(Path::new(&p), &mut out);
                        if let Ok(mut m) = c.lock() {
                            m.insert(p, out.len());
                        }
                    });
                    folders.push(FolderChoice {
                        kind,
                        path,
                        selected: i == 0, // Music pre-selected, like the mockup
                        is_file: false,
                    });
                }
            }
        }

        // Existing installs auto-scanned the app's default folder before
        // onboarding. If it still exists, suggest it *pre-selected* so upgrading
        // users don't lose their library after finishing the wizard.
        if let Ok(abs) = std::fs::canonicalize(super::MUSIC_ROOT) {
            if abs.is_dir() {
                let raw = abs.to_string_lossy().to_string();
                // Strip Windows' \\?\ extended-length prefix for a clean path.
                let path = raw.strip_prefix(r"\\?\").unwrap_or(&raw).to_string();
                if !folders.iter().any(|f| f.path == path) {
                    let name = abs
                        .file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| path.clone());
                    let (p, c) = (path.clone(), counts.clone());
                    std::thread::spawn(move || {
                        let mut out = Vec::new();
                        crate::audio::collect_audio_files(Path::new(&p), &mut out);
                        if let Ok(mut m) = c.lock() {
                            m.insert(p, out.len());
                        }
                    });
                    folders.push(FolderChoice {
                        kind: FolderKind::Custom(name),
                        path,
                        selected: true,
                        is_file: false,
                    });
                }
            }
        }

        Self {
            step: 0,
            lang,
            theme_light: false,
            accent_idx: 0,
            greeting_idx: 0,
            greeting_at: Instant::now(),
            shortcuts: HashMap::new(),
            rebinding: None,
            folders,
            counts,
        }
    }

    /// Advances the cycling greeting and returns the word plus its current fade
    /// alpha (0..1), so the left panel can render it mid-transition.
    fn greeting(&mut self) -> (&'static str, f32) {
        let mut e = self.greeting_at.elapsed().as_secs_f32();
        if e >= GREETING_INTERVAL {
            self.greeting_idx = (self.greeting_idx + 1) % GREETINGS.len();
            self.greeting_at = Instant::now();
            e = 0.0;
        }
        // Fade in at the start, fade out at the end, full opacity in between.
        let raw = if e < GREETING_FADE {
            e / GREETING_FADE
        } else if e > GREETING_INTERVAL - GREETING_FADE {
            (GREETING_INTERVAL - e) / GREETING_FADE
        } else {
            1.0
        }
        .clamp(0.0, 1.0);
        // Smoothstep for a softer ease.
        let alpha = raw * raw * (3.0 - 2.0 * raw);
        (GREETINGS[self.greeting_idx], alpha)
    }

    /// Paths of the ticked *folders*.
    fn selected_folders(&self) -> Vec<String> {
        self.folders
            .iter()
            .filter(|f| f.selected && !f.is_file)
            .map(|f| f.path.clone())
            .collect()
    }

    /// Paths of the ticked *files*.
    fn selected_files(&self) -> Vec<String> {
        self.folders
            .iter()
            .filter(|f| f.selected && f.is_file)
            .map(|f| f.path.clone())
            .collect()
    }

    /// Adds a dropped/picked folder (selected), avoiding duplicates.
    fn add_folder(&mut self, path: String) {
        if self.folders.iter().any(|f| f.path == path) {
            return;
        }
        let name = Path::new(&path)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| path.clone());
        let (p, c) = (path.clone(), self.counts.clone());
        std::thread::spawn(move || {
            let mut out = Vec::new();
            crate::audio::collect_audio_files(Path::new(&p), &mut out);
            if let Ok(mut m) = c.lock() {
                m.insert(p, out.len());
            }
        });
        self.folders.push(FolderChoice {
            kind: FolderKind::Custom(name),
            path,
            selected: true,
            is_file: false,
        });
    }

    /// Adds a single dragged-in audio file (selected), avoiding duplicates.
    fn add_file(&mut self, path: String) {
        if !crate::audio::is_audio_file(Path::new(&path)) || self.folders.iter().any(|f| f.path == path) {
            return;
        }
        let name = Path::new(&path)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| path.clone());
        self.folders.push(FolderChoice {
            kind: FolderKind::Custom(name),
            path,
            selected: true,
            is_file: true,
        });
    }
}

impl App {
    /// Handles a folder dropped onto the window while the wizard is open.
    pub(in crate::app) fn onboarding_handle_drop(&mut self, paths: &[std::path::PathBuf]) {
        if let Some(ob) = &mut self.onboarding {
            for p in paths {
                // Folders become folder rows; individual audio files become
                // their own rows in the list below.
                if p.is_dir() {
                    ob.add_folder(p.to_string_lossy().to_string());
                } else {
                    ob.add_file(p.to_string_lossy().to_string());
                }
            }
        }
    }

    /// Draws the whole welcome wizard. Called instead of the normal UI while
    /// [`App::onboarding`] is set.
    pub(in crate::app) fn ui_onboarding(&mut self, ctx: &egui::Context) {
        let mut ob = match self.onboarding.take() {
            Some(o) => o,
            None => return,
        };
        let accent = ACCENTS[ob.accent_idx].1;
        let t = ot(ob.lang);

        let mut finish = false;
        let mut skip = false;
        let (greeting, greeting_alpha) = ob.greeting();

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(RIGHT_BG))
            .show(ctx, |ui| {
                let area = ui.max_rect();
                let left_w = (area.width() * 0.31).clamp(300.0, 470.0);
                let left = Rect::from_min_max(area.min, pos2(area.left() + left_w, area.bottom()));
                let right = Rect::from_min_max(pos2(area.left() + left_w, area.top()), area.max);

                draw_left_panel(ui, left, accent, &ob, &t, greeting, greeting_alpha);

                match ob.step {
                    0 => step_language(ui, right, accent, &mut ob),
                    1 => step_appearance(ui, right, accent, &mut ob, &t),
                    2 => step_hotkeys(ui, right, accent, &mut ob, &t),
                    _ => step_folders(ui, right, accent, &mut ob, &t),
                }

                // --- Skip (top-right) — a pill that fills on hover ---
                let sk_w = ui
                    .ctx()
                    .fonts(|f| f.layout_no_wrap(t.skip.to_owned(), FontId::proportional(15.0), Color32::WHITE).size().x)
                    + 40.0;
                let sk_h = 40.0;
                let skip_rect = Rect::from_min_size(pos2(right.right() - 40.0 - sk_w, right.top() + 28.0), vec2(sk_w, sk_h));
                let sr = ui.interact(skip_rect, ui.id().with("ob_skip"), Sense::click());
                if sr.hovered() {
                    ui.painter().rect_filled(skip_rect, Rounding::same(sk_h / 2.0), Color32::from_rgb(40, 40, 46));
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                ui.painter().text(
                    skip_rect.center(),
                    Align2::CENTER_CENTER,
                    t.skip,
                    FontId::proportional(15.0),
                    if sr.hovered() { Color32::WHITE } else { FAINT },
                );
                if sr.clicked() {
                    skip = true;
                }

                // --- Bottom navigation ---
                let is_last = ob.step == 3;
                let label = if is_last { t.finish } else { t.next };
                let btn_rect = Rect::from_min_size(
                    pos2(right.right() - 40.0 - 168.0, right.bottom() - 40.0 - 56.0),
                    vec2(168.0, 56.0),
                );
                if pill_button(ui, btn_rect, label, accent, ui.id().with("ob_next")) {
                    if is_last {
                        finish = true;
                    } else {
                        ob.step += 1;
                    }
                }

                if ob.step > 0 {
                    let back_label = t.back.to_string();
                    let font = FontId::proportional(16.0);
                    let galley = ui.ctx().fonts(|f| f.layout_no_wrap(back_label, font, Color32::WHITE));
                    let (icon_sz, gap) = (16.0, 8.0);
                    let bw = icon_sz + gap + galley.rect.width() + 44.0;
                    let bh = 48.0;
                    let back_rect = Rect::from_min_size(pos2(right.left() + 40.0, right.bottom() - 40.0 - bh), vec2(bw, bh));
                    let br = ui.interact(back_rect, ui.id().with("ob_back"), Sense::click());
                    if br.hovered() {
                        ui.painter().rect_filled(back_rect, Rounding::same(bh / 2.0), Color32::from_rgb(40, 40, 46));
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    let col = if br.hovered() { Color32::WHITE } else { MUTED };
                    let start_x = back_rect.center().x - (icon_sz + gap + galley.rect.width()) / 2.0;
                    crate::icons::paint(ui, Rect::from_min_size(pos2(start_x, back_rect.center().y - icon_sz / 2.0), vec2(icon_sz, icon_sz)), crate::icons::Icon::ArrowLeft, col);
                    ui.painter().galley(pos2(start_x + icon_sz + gap, back_rect.center().y - galley.rect.height() / 2.0), galley, col);
                    if br.clicked() {
                        ob.step -= 1;
                    }
                }
            });

        if skip || finish {
            self.finish_onboarding(ob, ctx, skip);
        } else {
            self.onboarding = Some(ob);
            // Repaint continuously for the smooth greeting cross-fade (and so
            // background folder counts appear as they finish).
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }

    /// Persists the wizard's choices, then kicks off the library scan and shows
    /// the loading screen. `skipped` uses defaults and no chosen folders.
    fn finish_onboarding(&mut self, ob: Onboarding, ctx: &egui::Context, skipped: bool) {
        let mut cfg = load_config();
        cfg.onboarded = true;
        cfg.language = ob.lang.code().to_string();
        cfg.theme = if ob.theme_light { "light" } else { "dark" }.to_string();
        cfg.accent = ACCENTS[ob.accent_idx].0.to_string();
        cfg.music_folders = if skipped { Vec::new() } else { ob.selected_folders() };
        cfg.music_files = if skipped { Vec::new() } else { ob.selected_files() };
        save_config(&cfg);

        // Install the chosen accent + light/dark palette globally and rebuild
        // the theme so it takes effect immediately (not just on the next launch).
        crate::theme::set_accent(ACCENTS[ob.accent_idx].1);
        crate::theme::set_light_theme(ob.theme_light);
        crate::theme::apply_custom_theme(ctx);

        // Apply chosen hotkeys (unless the whole wizard was skipped, which keeps
        // the defaults). `save_shortcuts` writes "None" for actions left unset.
        if !skipped {
            save_shortcuts(&ob.shortcuts);
            self.shortcuts = ob.shortcuts.clone();
        }

        self.language = ob.lang;
        self.onboarding = None;
        self.loading = Some(super::LoadingState::default());
        self.start_library_scan(ctx, cfg.music_folders, cfg.music_files);
    }
}

// ---------------------------------------------------------------------------
// Left panel
// ---------------------------------------------------------------------------

fn draw_left_panel(ui: &mut egui::Ui, left: Rect, accent: Color32, ob: &Onboarding, t: &Ot, greeting: &str, alpha: f32) {
    // Diagonal gradient: a faint accent-tinted glow at the top-left fading to
    // near-black toward the bottom-right (matches the mockup).
    let p = ui.painter();
    p.rect_filled(left, Rounding::ZERO, LEFT_BG);
    let tint = |amt: f32| {
        let m = |base: u8, acc: u8| (base as f32 + (acc as f32 - base as f32) * amt) as u8;
        Color32::from_rgb(m(LEFT_BG.r(), accent.r()), m(LEFT_BG.g(), accent.g()), m(LEFT_BG.b(), accent.b()))
    };
    let mut mesh = egui::epaint::Mesh::default();
    let uv = egui::epaint::WHITE_UV;
    let corners = [
        (left.left_top(), tint(0.16)),      // brightest
        (left.right_top(), tint(0.03)),
        (left.right_bottom(), Color32::from_rgb(15, 14, 18)),
        (left.left_bottom(), tint(0.02)),
    ];
    for (pos, col) in corners {
        mesh.vertices.push(egui::epaint::Vertex { pos, uv, color: col });
    }
    mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
    p.add(mesh);

    // Logo + accent dot.
    let logo_pos = pos2(left.left() + 40.0, left.top() + 44.0);
    let logo_end = p.text(
        logo_pos,
        Align2::LEFT_CENTER,
        "Elysium",
        FontId::proportional(26.0),
        Color32::WHITE,
    );
    p.circle_filled(pos2(logo_end.right() + 14.0, logo_end.center().y), 5.0, accent);

    // Big greeting, cross-fading between languages. Shrink the font for long
    // words (e.g. "Bonjour", "Merhaba") so they always fit the left panel.
    let greet_y = left.top() + left.height() * 0.42;
    let a = (alpha * 255.0) as u8;
    let max_w = left.width() - 80.0;
    let base = 88.0_f32;
    let measured = ui
        .ctx()
        .fonts(|f| f.layout_no_wrap(greeting.to_owned(), FontId::proportional(base), Color32::WHITE).size().x);
    let size = if measured > max_w {
        (base * max_w / measured).max(44.0)
    } else {
        base
    };
    p.text(
        pos2(left.left() + 40.0, greet_y),
        Align2::LEFT_TOP,
        greeting,
        FontId::proportional(size),
        Color32::from_rgba_unmultiplied(240, 240, 244, a),
    );

    // Subtitle (wrapped) under the greeting, spaced from its actual height.
    let sub_rect = Rect::from_min_max(
        pos2(left.left() + 42.0, greet_y + size + 30.0),
        pos2(left.right() - 40.0, greet_y + size + 120.0),
    );
    let mut sub_ui = ui.new_child(egui::UiBuilder::new().max_rect(sub_rect).layout(Layout::top_down(Align::Min)));
    sub_ui.label(RichText::new(t.subtitle).size(17.0).color(MUTED));

    // Step indicator near the bottom.
    let steps = [t.step_language, t.step_appearance, t.step_hotkeys, t.step_music];
    let mut y = left.bottom() - 238.0;
    for (i, label) in steps.iter().enumerate() {
        let active = i as u8 == ob.step;
        let done = (i as u8) < ob.step;
        let c = pos2(left.left() + 58.0, y);
        let circle = if active || done { accent } else { Color32::from_rgb(40, 40, 46) };
        ui.painter().circle_filled(c, 15.0, circle);
        ui.painter().text(
            c,
            Align2::CENTER_CENTER,
            format!("{}", i + 1),
            FontId::proportional(13.0),
            if active || done { Color32::WHITE } else { FAINT },
        );
        ui.painter().text(
            pos2(c.x + 30.0, c.y),
            Align2::LEFT_CENTER,
            *label,
            FontId::proportional(17.0),
            if active { Color32::WHITE } else { MUTED },
        );
        y += 48.0;
    }
}

// ---------------------------------------------------------------------------
// Step 1 — language
// ---------------------------------------------------------------------------

fn step_language(ui: &mut egui::Ui, right: Rect, accent: Color32, ob: &mut Onboarding) {
    let t = ot(ob.lang);
    draw_heading(ui, right, t.choose_language, t.change_in_settings);

    let cards_top = right.top() + 150.0;
    let cw = 220.0;
    let ch = 96.0;
    let gap = 22.0;
    for (i, lang) in Lang::all().iter().enumerate() {
        let x = right.left() + 40.0 + i as f32 * (cw + gap);
        let rect = Rect::from_min_size(pos2(x, cards_top), vec2(cw, ch));
        let selected = ob.lang == *lang;
        if lang_card(ui, rect, lang.native_name(), lang.code(), selected, accent, ui.id().with(("ob_lang", i))) {
            ob.lang = *lang;
        }
    }
}

fn lang_card(ui: &mut egui::Ui, rect: Rect, name: &str, code: &str, selected: bool, accent: Color32, id: egui::Id) -> bool {
    let r = ui.interact(rect, id, Sense::click());
    // Lift the card up a little on hover.
    let rect = rect.translate(vec2(0.0, -hover_anim(ui, id, r.hovered(), 6.0)));
    let border = if selected {
        Stroke::new(1.6, accent)
    } else if r.hovered() {
        Stroke::new(1.0, Color32::from_rgb(70, 70, 78))
    } else {
        Stroke::new(1.0, CARD_BORDER)
    };
    ui.painter().rect(rect, Rounding::same(14.0), CARD_BG, border);
    ui.painter().text(
        pos2(rect.left() + 24.0, rect.top() + 34.0),
        Align2::LEFT_CENTER,
        name,
        FontId::proportional(21.0),
        Color32::WHITE,
    );
    ui.painter().text(
        pos2(rect.left() + 24.0, rect.top() + 66.0),
        Align2::LEFT_CENTER,
        code.to_uppercase(),
        FontId::proportional(13.0),
        FAINT,
    );
    if r.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    r.clicked()
}

// ---------------------------------------------------------------------------
// Step 2 — appearance
// ---------------------------------------------------------------------------

fn step_appearance(ui: &mut egui::Ui, right: Rect, accent: Color32, ob: &mut Onboarding, t: &Ot) {
    draw_heading(ui, right, t.choose_appearance, t.appearance_hint);

    let top = right.top() + 158.0;
    let cw = 300.0;
    let ch = 190.0;
    let gap = 24.0;
    // Dark card
    let dark_rect = Rect::from_min_size(pos2(right.left() + 40.0, top), vec2(cw, ch));
    if theme_card(ui, dark_rect, t.dark, false, !ob.theme_light, accent, ui.id().with("ob_dark")) {
        ob.theme_light = false;
    }
    // Light card
    let light_rect = Rect::from_min_size(pos2(right.left() + 40.0 + cw + gap, top), vec2(cw, ch));
    if theme_card(ui, light_rect, t.light, true, ob.theme_light, accent, ui.id().with("ob_light")) {
        ob.theme_light = true;
    }

    // Accent row
    let ay = top + ch + 46.0;
    ui.painter().text(
        pos2(right.left() + 40.0, ay),
        Align2::LEFT_CENTER,
        t.accent,
        FontId::proportional(16.0),
        MUTED,
    );
    for (i, (_, color)) in ACCENTS.iter().enumerate() {
        let id = ui.id().with(("ob_accent", i));
        let base = pos2(right.left() + 130.0 + i as f32 * 46.0, ay);
        let dr = ui.interact(Rect::from_center_size(base, vec2(38.0, 38.0)), id, Sense::click());
        let g = hover_anim(ui, id, dr.hovered(), 2.5);
        if i == ob.accent_idx {
            ui.painter().circle_stroke(base, 18.0 + g, Stroke::new(2.0, Color32::WHITE));
        }
        ui.painter().circle_filled(base, 13.0 + g, *color);
        if dr.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if dr.clicked() {
            ob.accent_idx = i;
        }
    }
}

/// A theme preview card with a tiny mock window and a select indicator.
fn theme_card(ui: &mut egui::Ui, rect: Rect, label: &str, light: bool, selected: bool, accent: Color32, id: egui::Id) -> bool {
    let r = ui.interact(rect, id, Sense::click());
    // Lift the card up a little on hover.
    let rect = rect.translate(vec2(0.0, -hover_anim(ui, id, r.hovered(), 6.0)));
    let border = if selected { Stroke::new(1.8, accent) } else { Stroke::new(1.0, CARD_BORDER) };
    ui.painter().rect(rect, Rounding::same(16.0), CARD_BG, border);

    // Mock app preview inside the top area.
    let preview = Rect::from_min_max(rect.min + vec2(18.0, 18.0), pos2(rect.right() - 18.0, rect.bottom() - 52.0));
    let (win_bg, side_bg, line, header) = if light {
        (
            Color32::from_rgb(245, 245, 247),
            Color32::from_rgb(255, 255, 255),
            Color32::from_rgb(205, 205, 210),
            Color32::from_rgb(224, 224, 228),
        )
    } else {
        (
            Color32::from_rgb(20, 20, 24),
            Color32::from_rgb(31, 31, 37),
            Color32::from_rgb(60, 60, 68),
            Color32::from_rgb(44, 44, 52),
        )
    };
    ui.painter().rect_filled(preview, Rounding::same(9.0), win_bg);
    // Sidebar with a few menu "lines".
    let side = Rect::from_min_size(preview.min + vec2(10.0, 10.0), vec2(preview.width() * 0.26, preview.height() - 20.0));
    ui.painter().rect_filled(side, Rounding::same(7.0), side_bg);
    for k in 0..3 {
        let ly = side.top() + 14.0 + k as f32 * 12.0;
        let w = side.width() * if k == 0 { 0.66 } else { 0.5 };
        ui.painter().rect_filled(
            Rect::from_min_size(pos2(side.left() + 12.0, ly), vec2(w, 5.0)),
            Rounding::same(2.5),
            line,
        );
    }
    // Header bar spanning the content area.
    let content_l = side.right() + 12.0;
    ui.painter().rect_filled(
        Rect::from_min_max(pos2(content_l, preview.top() + 12.0), pos2(preview.right() - 12.0, preview.top() + 26.0)),
        Rounding::same(6.0),
        header,
    );
    // Three gradient album squares (top lighter, like the mockup).
    let grads = [
        (Color32::from_rgb(150, 136, 224), Color32::from_rgb(112, 96, 178)),
        (Color32::from_rgb(42, 192, 206), Color32::from_rgb(17, 132, 150)),
        (Color32::from_rgb(228, 134, 110), Color32::from_rgb(186, 94, 74)),
    ];
    let sw = 38.0;
    for (k, (top_c, bot_c)) in grads.into_iter().enumerate() {
        let sx = content_l + k as f32 * (sw + 12.0);
        let sq = Rect::from_min_size(pos2(sx, preview.top() + 40.0), vec2(sw, sw));
        grad_square(ui.painter(), sq, top_c, bot_c);
    }

    // Label + radio/check.
    ui.painter().text(
        pos2(rect.left() + 20.0, rect.bottom() - 26.0),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(18.0),
        Color32::WHITE,
    );
    let cc = pos2(rect.right() - 26.0, rect.bottom() - 26.0);
    if selected {
        ui.painter().circle_filled(cc, 12.0, accent);
        draw_check(ui.painter(), cc, 11.0, Color32::WHITE);
    } else {
        ui.painter().circle_stroke(cc, 11.0, Stroke::new(1.5, FAINT));
    }

    if r.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    r.clicked()
}

// ---------------------------------------------------------------------------
// Step 3 — global hotkeys
// ---------------------------------------------------------------------------

fn step_hotkeys(ui: &mut egui::Ui, right: Rect, accent: Color32, ob: &mut Onboarding, t: &Ot) {
    draw_heading(ui, right, t.configure_hotkeys, t.hotkeys_hint);

    // While waiting for a key, bind the first one pressed (Esc cancels).
    if let Some(action) = ob.rebinding {
        let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
        if esc {
            ob.rebinding = None;
        } else if let Some(key) = ui.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Key { key, pressed: true, .. } => Some(*key),
                _ => None,
            })
        }) {
            ob.shortcuts.insert(action, key);
            ob.rebinding = None;
        }
    }

    let btn_w = 210.0;
    let btn_h = 44.0;
    let row_h = 62.0;
    let trash_slot = 48.0; // space reserved on the far right for the delete icon
    let mut click: Option<Shortcut> = None;
    let mut clear: Option<Shortcut> = None;
    let mut y = right.top() + 142.0;
    for action in Shortcut::all() {
        let cy = y + row_h / 2.0;
        // Action label (left).
        ui.painter().text(
            pos2(right.left() + 40.0, cy),
            Align2::LEFT_CENTER,
            action.label(ob.lang),
            FontId::proportional(18.0),
            Color32::WHITE,
        );
        // Binding button (right, leaving room for the trash icon).
        let listening = ob.rebinding == Some(*action);
        let hk_id = ui.id().with(("ob_hk", action.code()));
        let base_br = Rect::from_min_size(pos2(right.right() - 40.0 - trash_slot - btn_w, cy - btn_h / 2.0), vec2(btn_w, btn_h));
        let resp = ui.interact(base_br, hk_id, Sense::click());
        let br = base_br;
        let (fill, txt, tcol) = if listening {
            (accent, t.press_key.to_string(), Color32::WHITE)
        } else {
            match ob.shortcuts.get(action) {
                Some(&key) => (Color32::from_rgb(38, 38, 44), key_label(key), Color32::WHITE),
                None => (Color32::from_rgb(30, 30, 35), t.not_assigned.to_string(), FAINT),
            }
        };
        let fill = if resp.hovered() && !listening { lighten(fill, 8) } else { fill };
        ui.painter().rect_filled(br, Rounding::same(10.0), fill);
        ui.painter().text(br.center(), Align2::CENTER_CENTER, txt, FontId::proportional(14.0), tcol);
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked() {
            click = Some(*action);
        }

        // Delete (trash) icon — only for actions that have a binding.
        if ob.shortcuts.contains_key(action) {
            let tc = pos2(right.right() - 40.0 - 17.0, cy);
            let tr = ui.interact(
                Rect::from_center_size(tc, vec2(34.0, 34.0)),
                ui.id().with(("ob_hk_del", action.code())),
                Sense::click(),
            );
            if tr.hovered() {
                ui.painter().circle_filled(tc, 17.0, Color32::from_rgb(74, 32, 36));
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            let col = if tr.hovered() { Color32::from_rgb(232, 96, 96) } else { FAINT };
            crate::icons::paint(ui, Rect::from_center_size(tc, vec2(17.0, 17.0)), crate::icons::Icon::Delete, col);
            if tr.clicked() {
                clear = Some(*action);
            }
        }

        // Subtle divider under the row.
        ui.painter().line_segment(
            [pos2(right.left() + 40.0, y + row_h), pos2(right.right() - 40.0, y + row_h)],
            Stroke::new(1.0, Color32::from_rgb(30, 30, 36)),
        );
        y += row_h;
    }
    if let Some(a) = clear {
        ob.shortcuts.remove(&a);
        if ob.rebinding == Some(a) {
            ob.rebinding = None;
        }
    }
    if let Some(a) = click {
        // Toggle: clicking the active one cancels, otherwise start listening.
        ob.rebinding = if ob.rebinding == Some(a) { None } else { Some(a) };
    }
}

// ---------------------------------------------------------------------------
// Step 4 — music folders
// ---------------------------------------------------------------------------

fn step_folders(ui: &mut egui::Ui, right: Rect, accent: Color32, ob: &mut Onboarding, t: &Ot) {
    draw_heading(ui, right, t.where_music, t.where_music_sub);

    // Drop / pick zone.
    let dz_id = ui.id().with("ob_dropzone");
    let dz_base = Rect::from_min_size(pos2(right.left() + 40.0, right.top() + 150.0), vec2(right.width() - 80.0, 150.0));
    let dzr = ui.interact(dz_base, dz_id, Sense::click());
    let dz = dz_base;
    // Highlight on mouse hover *and* while a file/folder is being dragged over
    // the window (so it's clearly a valid drop target).
    let dragging = ui.input(|i| !i.raw.hovered_files.is_empty());
    let active = dzr.hovered() || dragging;
    let fill = if active { Color32::from_rgb(28, 28, 36) } else { Color32::from_rgb(20, 20, 24) };
    ui.painter().rect_filled(dz, Rounding::same(16.0), fill);
    let border_col = if active { accent } else { CARD_BORDER };
    for s in egui::Shape::dashed_line(&rrect_path(dz, 16.0), Stroke::new(1.6, border_col), 9.0, 6.0) {
        ui.painter().add(s);
    }
    ui.painter().text(dz.center() - vec2(0.0, 22.0), Align2::CENTER_CENTER, "⬆", FontId::proportional(30.0), MUTED);
    ui.painter().text(dz.center() + vec2(0.0, 14.0), Align2::CENTER_CENTER, t.drop_folder, FontId::proportional(18.0), Color32::WHITE);
    ui.painter().text(dz.center() + vec2(0.0, 40.0), Align2::CENTER_CENTER, t.or_pick, FontId::proportional(13.0), FAINT);
    if dzr.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if dzr.clicked() {
        if let Some(dir) = rfd::FileDialog::new().pick_folder() {
            ob.add_folder(dir.to_string_lossy().to_string());
        }
    }

    // Folder rows.
    let counts = ob.counts.lock().ok().map(|m| m.clone()).unwrap_or_default();
    let mut y = dz.bottom() + 22.0;
    let row_h = 68.0;
    // Collect toggles to apply after drawing (avoid borrow issues).
    let mut toggle: Option<usize> = None;
    for (i, f) in ob.folders.iter().enumerate() {
        let fid = ui.id().with(("ob_folder", i));
        let base = Rect::from_min_size(pos2(right.left() + 40.0, y), vec2(right.width() - 80.0, row_h - 10.0));
        let rr = ui.interact(base, fid, Sense::click());
        let rect = base;
        // Highlight (lighter fill + brighter border) on hover.
        let fill = if rr.hovered() { Color32::from_rgb(34, 34, 40) } else { CARD_BG };
        let border = if f.selected {
            Stroke::new(1.5, accent)
        } else if rr.hovered() {
            Stroke::new(1.0, Color32::from_rgb(64, 64, 72))
        } else {
            Stroke::new(1.0, CARD_BORDER)
        };
        ui.painter().rect(rect, Rounding::same(12.0), fill, border);
        let icon = if f.is_file { crate::icons::Icon::Music } else { crate::icons::Icon::Folder };
        crate::icons::paint(ui, Rect::from_center_size(pos2(rect.left() + 30.0, rect.center().y), vec2(22.0, 22.0)), icon, MUTED);
        ui.painter().text(pos2(rect.left() + 58.0, rect.top() + 20.0), Align2::LEFT_CENTER, folder_name(&f.kind, t), FontId::proportional(17.0), Color32::WHITE);
        ui.painter().text(pos2(rect.left() + 58.0, rect.top() + 42.0), Align2::LEFT_CENTER, &f.path, FontId::monospace(12.0), FAINT);

        // Track count on the right (a single file is always one track).
        let count_txt = if f.is_file {
            t.n_tracks.replace("{n}", "1")
        } else {
            match counts.get(&f.path) {
                Some(n) => t.n_tracks.replace("{n}", &n.to_string()),
                None => "…".to_string(),
            }
        };
        ui.painter().text(pos2(rect.right() - 70.0, rect.center().y), Align2::RIGHT_CENTER, count_txt, FontId::proportional(14.0), MUTED);

        // Checkbox.
        let cb = Rect::from_center_size(pos2(rect.right() - 30.0, rect.center().y), vec2(22.0, 22.0));
        if f.selected {
            ui.painter().rect_filled(cb, Rounding::same(6.0), accent);
            draw_check(ui.painter(), cb.center(), 9.0, Color32::WHITE);
        } else {
            ui.painter().rect_stroke(cb, Rounding::same(6.0), Stroke::new(1.4, FAINT));
        }
        if rr.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if rr.clicked() {
            toggle = Some(i);
        }
        y += row_h;
    }
    if let Some(i) = toggle {
        ob.folders[i].selected = !ob.folders[i].selected;
    }
}

fn folder_name(kind: &FolderKind, t: &Ot) -> String {
    match kind {
        FolderKind::Music => t.folder_music.to_string(),
        FolderKind::Downloads => t.folder_downloads.to_string(),
        FolderKind::Desktop => t.folder_desktop.to_string(),
        FolderKind::Custom(name) => name.clone(),
    }
}

// ---------------------------------------------------------------------------
// Shared drawing helpers
// ---------------------------------------------------------------------------

fn draw_heading(ui: &mut egui::Ui, right: Rect, title: &str, sub: &str) {
    ui.painter().text(
        pos2(right.left() + 40.0, right.top() + 54.0),
        Align2::LEFT_CENTER,
        title,
        FontId::proportional(40.0),
        Color32::WHITE,
    );
    let sub_rect = Rect::from_min_max(pos2(right.left() + 42.0, right.top() + 84.0), pos2(right.right() - 180.0, right.top() + 140.0));
    let mut sub_ui = ui.new_child(egui::UiBuilder::new().max_rect(sub_rect).layout(Layout::top_down(Align::Min)));
    sub_ui.label(RichText::new(sub).size(16.0).color(MUTED));
}

/// A filled, pill-shaped primary button with a trailing glyph. Returns whether
/// it was clicked.
fn pill_button(ui: &mut egui::Ui, rect: Rect, label: &str, accent: Color32, id: egui::Id) -> bool {
    let r = ui.interact(rect, id, Sense::click());
    let fill = if r.hovered() { lighten(accent, 20) } else { accent };
    ui.painter().rect_filled(rect, Rounding::same(rect.height() / 2.0), fill);
    ui.painter().text(rect.center() - vec2(12.0, 0.0), Align2::CENTER_CENTER, label, FontId::proportional(18.0), Color32::WHITE);
    crate::icons::paint(ui, Rect::from_center_size(pos2(rect.right() - 26.0, rect.center().y), vec2(16.0, 16.0)), crate::icons::Icon::ArrowRight, Color32::WHITE);
    if r.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    r.clicked()
}

fn lighten(c: Color32, by: u8) -> Color32 {
    Color32::from_rgb(c.r().saturating_add(by), c.g().saturating_add(by), c.b().saturating_add(by))
}

/// Smoothly-animated hover amount in pixels: eases from 0 toward `amount` while
/// `hovered`, and back when not. Callers use it either as a lift (translate up)
/// or a grow (expand). Drives the "alive" feel of the wizard.
fn hover_anim(ui: &egui::Ui, id: egui::Id, hovered: bool, amount: f32) -> f32 {
    ui.ctx().animate_bool_with_time(id.with("hover_anim"), hovered, 0.11) * amount
}

/// Appends a quarter-circle arc (center `cx,cy`, radius `r`, from `a0` to `a1`).
fn push_arc(pts: &mut Vec<egui::Pos2>, cx: f32, cy: f32, r: f32, a0: f32, a1: f32) {
    let n = 5;
    for i in 0..=n {
        let a = a0 + (a1 - a0) * (i as f32 / n as f32);
        pts.push(pos2(cx + r * a.cos(), cy + r * a.sin()));
    }
}

/// A clockwise point path tracing a rounded rectangle — fed to
/// [`egui::Shape::dashed_line`] to draw a dashed rounded border.
fn rrect_path(rect: Rect, r: f32) -> Vec<egui::Pos2> {
    use std::f32::consts::PI;
    let mut pts = Vec::new();
    push_arc(&mut pts, rect.left() + r, rect.top() + r, r, PI, 1.5 * PI);
    push_arc(&mut pts, rect.right() - r, rect.top() + r, r, 1.5 * PI, 2.0 * PI);
    push_arc(&mut pts, rect.right() - r, rect.bottom() - r, r, 0.0, 0.5 * PI);
    push_arc(&mut pts, rect.left() + r, rect.bottom() - r, r, 0.5 * PI, PI);
    if let Some(&first) = pts.first() {
        pts.push(first);
    }
    pts
}

/// Draws a checkmark from two line segments (font-independent — the bundled font
/// has no `✓` glyph). `s` is roughly the half-size.
fn draw_check(p: &egui::Painter, c: egui::Pos2, s: f32, color: Color32) {
    let stroke = Stroke::new((s * 0.26).max(1.7), color);
    let a = pos2(c.x - s * 0.52, c.y + s * 0.04);
    let mid = pos2(c.x - s * 0.12, c.y + s * 0.42);
    let d = pos2(c.x + s * 0.56, c.y - s * 0.44);
    p.line_segment([a, mid], stroke);
    p.line_segment([mid, d], stroke);
}

/// Draws a small rounded square with a vertical gradient (`top` → `bottom`).
/// A rounded base in the average color gives the corners; a slightly inset mesh
/// paints the gradient body.
fn grad_square(p: &egui::Painter, sq: Rect, top: Color32, bottom: Color32) {
    let avg = Color32::from_rgb(
        ((top.r() as u16 + bottom.r() as u16) / 2) as u8,
        ((top.g() as u16 + bottom.g() as u16) / 2) as u8,
        ((top.b() as u16 + bottom.b() as u16) / 2) as u8,
    );
    p.rect_filled(sq, Rounding::same(8.0), avg);
    let inner = sq.shrink(2.5);
    let uv = egui::epaint::WHITE_UV;
    let mut mesh = egui::epaint::Mesh::default();
    for (pos, color) in [
        (inner.left_top(), top),
        (inner.right_top(), top),
        (inner.right_bottom(), bottom),
        (inner.left_bottom(), bottom),
    ] {
        mesh.vertices.push(egui::epaint::Vertex { pos, uv, color });
    }
    mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
    p.add(mesh);
}

// ---------------------------------------------------------------------------
// Loading screen (shown after finishing, while the library scans)
// ---------------------------------------------------------------------------

impl App {
    /// Draws the post-onboarding loading screen with a live track counter.
    pub(in crate::app) fn ui_loading(&self, ctx: &egui::Context) {
        let (loaded, total, last) = match &self.loading {
            Some(l) => (l.loaded, l.total, l.last_file.clone()),
            None => return,
        };
        let t = ot(self.language);
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(LEFT_BG))
            .show(ctx, |ui| {
                let a = ui.max_rect();
                let cx = a.center().x;
                ui.painter().text(pos2(cx, a.top() + a.height() * 0.28), Align2::CENTER_CENTER, "Elysium", FontId::proportional(28.0), Color32::WHITE);

                let spin = pos2(cx, a.top() + a.height() * 0.38);
                let mut sp_ui = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_center_size(spin, vec2(40.0, 40.0))));
                sp_ui.add(egui::Spinner::new().size(28.0));

                // Big counter.
                ui.painter().text(pos2(cx, a.center().y - 10.0), Align2::CENTER_CENTER, format!("{loaded}"), FontId::proportional(96.0), Color32::WHITE);
                ui.painter().text(pos2(cx, a.center().y + 60.0), Align2::CENTER_CENTER, t.loading_music, FontId::proportional(22.0), Color32::WHITE);
                ui.painter().text(pos2(cx, a.center().y + 92.0), Align2::CENTER_CENTER, t.found_tracks.replace("{n}", &total.to_string()), FontId::proportional(15.0), MUTED);

                // Progress bar.
                let bar = Rect::from_center_size(pos2(cx, a.center().y + 150.0), vec2(560.0, 6.0));
                ui.painter().rect_filled(bar, Rounding::same(3.0), Color32::from_rgb(48, 48, 54));
                let frac = if total > 0 { (loaded as f32 / total as f32).clamp(0.0, 1.0) } else { 0.0 };
                let fill = Rect::from_min_size(bar.min, vec2(bar.width() * frac, bar.height()));
                ui.painter().rect_filled(fill, Rounding::same(3.0), ACCENTS[3].1);

                if !last.is_empty() {
                    ui.painter().text(pos2(cx, a.center().y + 182.0), Align2::CENTER_CENTER, last, FontId::monospace(13.0), FAINT);
                }
            });
        ctx.request_repaint_after(std::time::Duration::from_millis(60));
    }
}

// ---------------------------------------------------------------------------
// Localized wizard text
// ---------------------------------------------------------------------------

struct Ot {
    subtitle: &'static str,
    step_language: &'static str,
    step_appearance: &'static str,
    step_hotkeys: &'static str,
    step_music: &'static str,
    choose_language: &'static str,
    change_in_settings: &'static str,
    choose_appearance: &'static str,
    appearance_hint: &'static str,
    dark: &'static str,
    light: &'static str,
    accent: &'static str,
    configure_hotkeys: &'static str,
    hotkeys_hint: &'static str,
    not_assigned: &'static str,
    press_key: &'static str,
    where_music: &'static str,
    where_music_sub: &'static str,
    drop_folder: &'static str,
    or_pick: &'static str,
    n_tracks: &'static str,
    folder_music: &'static str,
    folder_downloads: &'static str,
    folder_desktop: &'static str,
    skip: &'static str,
    back: &'static str,
    next: &'static str,
    finish: &'static str,
    loading_music: &'static str,
    found_tracks: &'static str,
}

fn ot(lang: Lang) -> Ot {
    match lang {
        Lang::Ru => Ot {
            subtitle: "Пара шагов — и твоя музыка зазвучит так, как тебе нравится.",
            step_language: "Язык",
            step_appearance: "Оформление",
            step_hotkeys: "Хоткеи",
            step_music: "Твоя музыка",
            choose_language: "Выбери язык",
            change_in_settings: "Можно изменить в любой момент в настройках.",
            choose_appearance: "Выбери оформление",
            appearance_hint: "Так выглядит приложение. Фон песни всегда подстраивается под обложку.",
            dark: "Тёмная",
            light: "Светлая",
            accent: "Акцент",
            configure_hotkeys: "Настрой свои хоткеи",
            hotkeys_hint: "В игре? Чтобы остановить музыку — не нужен альт-таб. Просто нажми клавишу.",
            not_assigned: "не назначено",
            press_key: "Нажми клавишу…",
            where_music: "Где твоя музыка?",
            where_music_sub: "Укажи папки — Elysium сразу найдёт и загрузит твои треки.",
            drop_folder: "Перетащи папку сюда",
            or_pick: "или нажми, чтобы выбрать вручную",
            n_tracks: "{n} треков",
            folder_music: "Музыка",
            folder_downloads: "Загрузки",
            folder_desktop: "Рабочий стол",
            skip: "Пропустить",
            back: "Назад",
            next: "Далее",
            finish: "Завершить",
            loading_music: "Загрузка музыки",
            found_tracks: "Найдено {n} треков",
        },
        Lang::Uk => Ot {
            subtitle: "Кілька кроків — і твоя музика зазвучить так, як тобі подобається.",
            step_language: "Мова",
            step_appearance: "Оформлення",
            step_hotkeys: "Хоткеї",
            step_music: "Твоя музика",
            choose_language: "Обери мову",
            change_in_settings: "Можна змінити будь-коли в налаштуваннях.",
            choose_appearance: "Обери оформлення",
            appearance_hint: "Так виглядає застосунок. Фон пісні завжди підлаштовується під обкладинку.",
            dark: "Темна",
            light: "Світла",
            accent: "Акцент",
            configure_hotkeys: "Налаштуй свої хоткеї",
            hotkeys_hint: "У грі? Щоб зупинити музику — не потрібен альт-таб. Просто натисни клавішу.",
            not_assigned: "не призначено",
            press_key: "Натисни клавішу…",
            where_music: "Де твоя музика?",
            where_music_sub: "Вкажи папки — Elysium одразу знайде та завантажить твої треки.",
            drop_folder: "Перетягни папку сюди",
            or_pick: "або натисни, щоб обрати вручну",
            n_tracks: "{n} треків",
            folder_music: "Музика",
            folder_downloads: "Завантаження",
            folder_desktop: "Робочий стіл",
            skip: "Пропустити",
            back: "Назад",
            next: "Далі",
            finish: "Завершити",
            loading_music: "Завантаження музики",
            found_tracks: "Знайдено {n} треків",
        },
        Lang::En => Ot {
            subtitle: "A few steps and your music will sound just the way you like.",
            step_language: "Language",
            step_appearance: "Appearance",
            step_hotkeys: "Hotkeys",
            step_music: "Your music",
            choose_language: "Choose a language",
            change_in_settings: "You can change it anytime in settings.",
            choose_appearance: "Choose appearance",
            appearance_hint: "This is how the app looks. The song background always adapts to the cover.",
            dark: "Dark",
            light: "Light",
            accent: "Accent",
            configure_hotkeys: "Set up your hotkeys",
            hotkeys_hint: "In a game? To pause the music you don't need alt-tab. Just press a key.",
            not_assigned: "not assigned",
            press_key: "Press a key…",
            where_music: "Where is your music?",
            where_music_sub: "Point to your folders — Elysium finds and loads your tracks right away.",
            drop_folder: "Drop a folder here",
            or_pick: "or click to choose manually",
            n_tracks: "{n} tracks",
            folder_music: "Music",
            folder_downloads: "Downloads",
            folder_desktop: "Desktop",
            skip: "Skip",
            back: "Back",
            next: "Continue",
            finish: "Finish",
            loading_music: "Loading music",
            found_tracks: "Found {n} tracks",
        },
    }
}
