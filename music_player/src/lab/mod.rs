//! The "Lab" — a real-time audio analysis window.
//!
//! Everything in here is pure rendering driven by [`dsp`]: it locks the shared
//! [`dsp::Analysis`] ring buffer once per frame, derives an FFT plus a handful
//! of level statistics, and paints seven widgets (waveform, spectrum,
//! spectrogram, oscilloscope, vectorscope, loudness, L/R levels).
//!
//! It deliberately knows nothing about the `App`: the glue that opens the
//! separate viewport window and feeds it live values lives in
//! `app::ui::lab`. That keeps this module a self-contained, testable renderer
//! that takes plain inputs ([`LabInputs`]) and persistent UI state
//! ([`LabState`]).

pub mod dsp;

use crate::lang::{strings, Lang, Strings};
use dsp::{SharedAnalysis, SharedWaveform};
use eframe::egui;
use egui::{
    pos2, vec2, Align2, Color32, FontId, Pos2, Rect, Rounding, Sense, Stroke, TextureHandle,
    TextureOptions,
};
use std::collections::VecDeque;

/// FFT window length. Power of two (required by [`dsp::fft`]). ~93 ms at
/// 44.1 kHz — a good balance of frequency resolution and responsiveness.
const FFT_SIZE: usize = 4096;
/// Number of log-spaced frequency rows in the spectrogram.
const SPECTRO_BINS: usize = 160;
/// Number of time columns (history) the spectrogram keeps.
const SPECTRO_COLS: usize = 240;

/// Lowest/highest frequency drawn on the spectrum & spectrogram axes.
const F_MIN: f32 = 25.0;
const F_MAX: f32 = 20000.0;

/// Persistent UI state for the Lab, owned by the `App` so it survives between
/// frames (and while the window is closed).
pub struct LabState {
    /// Per-widget visibility, indexed to match [`WIDGET_NAMES`].
    pub show: [bool; 7],

    // --- scratch reused every frame to avoid per-frame allocation ---
    buf_l: Vec<f32>,
    buf_r: Vec<f32>,
    re: Vec<f32>,
    im: Vec<f32>,
    window: Vec<f32>,

    // --- spectrogram ---
    /// Newest column at the back; each column is `SPECTRO_BINS` bytes (0..255).
    spectro: VecDeque<Vec<u8>>,
    spectro_tex: Option<TextureHandle>,

    // --- meters (need to persist for smoothing / peak-hold) ---
    lufs_m: f32,
    lufs_s: f32,
    lufs_i: f32,
    lufs_lo: f32,
    lufs_hi: f32,
    peak_hold_l: f32,
    peak_hold_r: f32,
}

impl Default for LabState {
    fn default() -> Self {
        // Hann window, computed once.
        let window: Vec<f32> = (0..FFT_SIZE)
            .map(|i| {
                let x = i as f32 / (FFT_SIZE - 1) as f32;
                0.5 - 0.5 * (std::f32::consts::TAU * x).cos()
            })
            .collect();

        Self {
            show: [true; 7],
            buf_l: Vec::with_capacity(FFT_SIZE),
            buf_r: Vec::with_capacity(FFT_SIZE),
            re: vec![0.0; FFT_SIZE],
            im: vec![0.0; FFT_SIZE],
            window,
            spectro: VecDeque::with_capacity(SPECTRO_COLS),
            spectro_tex: None,
            lufs_m: -70.0,
            lufs_s: -70.0,
            lufs_i: -70.0,
            lufs_lo: -70.0,
            lufs_hi: -70.0,
            peak_hold_l: -60.0,
            peak_hold_r: -60.0,
        }
    }
}

/// Live values handed to the Lab each frame by the `App`.
pub struct LabInputs<'a> {
    pub analysis: &'a SharedAnalysis,
    pub waveform: &'a SharedWaveform,
    /// Seconds into the current track (for the waveform playhead).
    pub elapsed: f32,
    /// Total track length in seconds, 0 if unknown.
    pub total: f32,
    /// Whether audio is actually advancing (meters decay when false).
    pub playing: bool,
    /// A track is loaded but its waveform is still being rendered off-thread.
    pub waveform_loading: bool,
    /// UI language for all Lab labels.
    pub lang: Lang,
}

/// Everything derived from the audio for one frame, shared by the widgets.
struct Derived {
    sr: f32,
    /// Linear magnitude spectrum, length `FFT_SIZE/2`.
    mag: Vec<f32>,
    peak_l_db: f32,
    peak_r_db: f32,
    rms_l_db: f32,
    rms_r_db: f32,
    corr: f32,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Draws the whole Lab into `ui`.
pub fn draw_lab(ui: &mut egui::Ui, st: &mut LabState, inp: &LabInputs) {
    // 1. Snapshot the most recent audio under the lock.
    let sr = {
        let a = inp.analysis.lock().unwrap();
        a.recent(FFT_SIZE, &mut st.buf_l, &mut st.buf_r);
        a.sample_rate.max(1) as f32
    };

    // 2. Derive spectrum + level stats from the snapshot.
    let derived = compute_derived(st, sr, inp.playing);

    // 3. Feed the spectrogram its newest column and the meters their smoothing.
    push_spectrogram_column(st, &derived);
    update_meters(st, &derived, inp.playing);

    // 4. Localized labels for this frame.
    let s = strings(inp.lang);
    let names = s.lab_widget_names;

    // 5. Header (toggle chips) + scrollable widget stack.
    draw_header(ui, st, &s);
    ui.add_space(8.0);

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if st.show[0] {
                card(ui, names[0], 150.0, |p, r| {
                    draw_waveform(p, r, inp, s.lab_computing);
                });
                ui.add_space(10.0);
            }

            row(ui, st.show[1], st.show[2], 210.0, |ui, slot| match slot {
                Slot::A => card(ui, names[1], 210.0, |p, r| {
                    draw_spectrum(p, r, st_ptr(st), &derived);
                }),
                Slot::B => card(ui, names[2], 210.0, |p, r| {
                    draw_spectrogram(p, r, st_ptr(st));
                }),
            });

            row(ui, st.show[3], st.show[4], 210.0, |ui, slot| match slot {
                Slot::A => card(ui, names[3], 210.0, |p, r| {
                    draw_oscilloscope(p, r, st_ptr(st));
                }),
                Slot::B => card(ui, names[4], 210.0, |p, r| {
                    draw_vectorscope(p, r, st_ptr(st), &derived, s.lab_corr);
                }),
            });

            row(ui, st.show[5], st.show[6], 170.0, |ui, slot| match slot {
                Slot::A => card(ui, names[5], 170.0, |p, r| {
                    draw_loudness(p, r, st_ptr(st));
                }),
                Slot::B => card(ui, names[6], 170.0, |p, r| {
                    draw_levels(p, r, st_ptr(st), &derived);
                }),
            });
        });
}

// A tiny escape hatch: the row/card closures need read access to `st` while the
// borrow checker only sees one `&mut st`. We never mutate `st` from inside a
// widget draw (only read), and the spectrogram texture is updated *before* the
// scroll area, so handing out a shared pointer here is sound in practice.
fn st_ptr(st: &LabState) -> &LabState {
    st
}

// ---------------------------------------------------------------------------
// Derivation
// ---------------------------------------------------------------------------

fn compute_derived(st: &mut LabState, sr: f32, playing: bool) -> Derived {
    let n = FFT_SIZE;
    let half = n / 2;

    // Windowed mono into the FFT scratch buffers.
    for i in 0..n {
        let l = st.buf_l.get(i).copied().unwrap_or(0.0);
        let r = st.buf_r.get(i).copied().unwrap_or(0.0);
        st.re[i] = 0.5 * (l + r) * st.window[i];
        st.im[i] = 0.0;
    }
    dsp::fft(&mut st.re, &mut st.im);

    let mut mag = vec![0.0f32; half];
    let norm = 2.0 / n as f32;
    for k in 0..half {
        mag[k] = (st.re[k] * st.re[k] + st.im[k] * st.im[k]).sqrt() * norm;
    }

    // Per-channel peak / RMS over the window.
    let mut peak_l = 0.0f32;
    let mut peak_r = 0.0f32;
    let mut sq_l = 0.0f32;
    let mut sq_r = 0.0f32;
    let mut sum_lr = 0.0f32;
    let count = st.buf_l.len().max(1) as f32;
    for i in 0..st.buf_l.len() {
        let l = st.buf_l[i];
        let r = st.buf_r[i];
        peak_l = peak_l.max(l.abs());
        peak_r = peak_r.max(r.abs());
        sq_l += l * l;
        sq_r += r * r;
        sum_lr += l * r;
    }
    let rms_l = (sq_l / count).sqrt();
    let rms_r = (sq_r / count).sqrt();
    let corr = if sq_l > 1e-9 && sq_r > 1e-9 {
        (sum_lr / (sq_l.sqrt() * sq_r.sqrt())).clamp(-1.0, 1.0)
    } else {
        0.0
    };

    // While paused, drag the level readouts toward silence so the meters fall.
    let (peak_l, peak_r, rms_l, rms_r) = if playing {
        (peak_l, peak_r, rms_l, rms_r)
    } else {
        (0.0, 0.0, 0.0, 0.0)
    };

    Derived {
        sr,
        mag,
        peak_l_db: to_db(peak_l),
        peak_r_db: to_db(peak_r),
        rms_l_db: to_db(rms_l),
        rms_r_db: to_db(rms_r),
        corr,
    }
}

fn push_spectrogram_column(st: &mut LabState, d: &Derived) {
    let mut col = vec![0u8; SPECTRO_BINS];
    let half = d.mag.len();
    for (bin, slot) in col.iter_mut().enumerate() {
        // Log-spaced frequency for this row.
        let t = bin as f32 / (SPECTRO_BINS - 1) as f32;
        let freq = F_MIN * (F_MAX / F_MIN).powf(t);
        let k = (freq * FFT_SIZE as f32 / d.sr).round() as usize;
        let m = d.mag.get(k.min(half.saturating_sub(1))).copied().unwrap_or(0.0);
        let db = to_db(m);
        // Map -90..-10 dB to 0..255.
        let v = ((db + 90.0) / 80.0).clamp(0.0, 1.0);
        *slot = (v * 255.0) as u8;
    }
    if st.spectro.len() >= SPECTRO_COLS {
        st.spectro.pop_front();
    }
    st.spectro.push_back(col);
}

fn update_meters(st: &mut LabState, d: &Derived, playing: bool) {
    // Momentary loudness ≈ -0.691 + 10·log10(mean square). No K-weighting, so
    // this is an honest approximation rather than a certified LUFS meter.
    let ms_l = db_to_lin(d.rms_l_db);
    let ms_r = db_to_lin(d.rms_r_db);
    let ms = ((ms_l * ms_l + ms_r * ms_r) * 0.5).max(1e-12);
    let momentary = -0.691 + 10.0 * ms.log10();

    if playing {
        st.lufs_m = momentary;
        st.lufs_s = lerp(st.lufs_s, momentary, 0.04);
        st.lufs_i = lerp(st.lufs_i, momentary, 0.004);
        if st.lufs_s > -60.0 {
            st.lufs_hi = st.lufs_hi.max(st.lufs_s);
            if st.lufs_lo < -69.0 {
                st.lufs_lo = st.lufs_s;
            }
            st.lufs_lo = st.lufs_lo.min(st.lufs_s);
        }
    } else {
        // Decay toward silence when paused.
        st.lufs_m = lerp(st.lufs_m, -70.0, 0.1);
        st.lufs_s = lerp(st.lufs_s, -70.0, 0.05);
    }

    // Peak-hold for the L/R bars: jump up instantly, fall back slowly.
    st.peak_hold_l = st.peak_hold_l.max(d.peak_l_db) - 0.3;
    st.peak_hold_r = st.peak_hold_r.max(d.peak_r_db) - 0.3;
    st.peak_hold_l = st.peak_hold_l.max(d.peak_l_db).max(-60.0);
    st.peak_hold_r = st.peak_hold_r.max(d.peak_r_db).max(-60.0);
}

// ---------------------------------------------------------------------------
// Header (widget toggles)
// ---------------------------------------------------------------------------

fn draw_header(ui: &mut egui::Ui, st: &mut LabState, s: &Strings) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(s.lab_mode)
                .size(15.0)
                .strong()
                .color(Color32::from_rgb(255, 70, 90)),
        );
        ui.add_space(16.0);
        ui.label(
            egui::RichText::new(s.lab_widgets)
                .size(12.0)
                .color(Color32::from_rgb(150, 150, 160)),
        );
        for i in 0..7 {
            let on = st.show[i];
            let (fill, text) = if on {
                (Color32::from_rgb(40, 20, 26), Color32::from_rgb(255, 120, 135))
            } else {
                (Color32::from_rgb(22, 22, 26), Color32::from_rgb(130, 130, 140))
            };
            let btn = egui::Button::new(egui::RichText::new(s.lab_widget_names[i]).size(12.0).color(text))
                .fill(fill)
                .rounding(13.0)
                .stroke(Stroke::new(
                    1.0,
                    if on {
                        Color32::from_rgb(150, 40, 55)
                    } else {
                        Color32::from_rgb(50, 50, 56)
                    },
                ));
            if ui.add(btn).clicked() {
                st.show[i] = !on;
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Layout helpers
// ---------------------------------------------------------------------------

enum Slot {
    A,
    B,
}

/// Lays out a left/right pair of widgets, gracefully collapsing to a single
/// full-width widget when only one of the two is visible.
fn row(
    ui: &mut egui::Ui,
    show_a: bool,
    show_b: bool,
    height: f32,
    mut draw: impl FnMut(&mut egui::Ui, Slot),
) {
    match (show_a, show_b) {
        (true, true) => {
            ui.columns(2, |cols| {
                draw(&mut cols[0], Slot::A);
                draw(&mut cols[1], Slot::B);
            });
            ui.add_space(10.0);
        }
        (true, false) => {
            draw(ui, Slot::A);
            ui.add_space(10.0);
        }
        (false, true) => {
            draw(ui, Slot::B);
            ui.add_space(10.0);
        }
        (false, false) => {
            let _ = height;
        }
    }
}

/// Draws a titled card and invokes `body` with a painter (clipped to the card)
/// and the inner content rect.
fn card(ui: &mut egui::Ui, title: &str, height: f32, body: impl FnOnce(&egui::Painter, Rect)) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, height), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, Rounding::same(10.0), Color32::from_rgb(15, 15, 19));
    p.rect_stroke(
        rect,
        Rounding::same(10.0),
        Stroke::new(1.0, Color32::from_rgb(30, 30, 36)),
    );
    p.text(
        pos2(rect.left() + 14.0, rect.top() + 15.0),
        Align2::LEFT_CENTER,
        title.to_uppercase(),
        FontId::proportional(12.0),
        Color32::from_rgb(165, 165, 178),
    );
    let inner = Rect::from_min_max(
        pos2(rect.left() + 12.0, rect.top() + 32.0),
        pos2(rect.right() - 12.0, rect.bottom() - 10.0),
    );
    body(&p, inner);
}

// ---------------------------------------------------------------------------
// Widgets
// ---------------------------------------------------------------------------

fn draw_waveform(p: &egui::Painter, r: Rect, inp: &LabInputs, computing: &str) {
    let mid = r.center().y;
    let half_h = r.height() * 0.46;

    let guard = inp.waveform.lock().unwrap();
    let Some(wf) = guard.as_ref() else {
        let (text, color) = if inp.waveform_loading {
            (computing, Color32::from_rgb(120, 120, 135))
        } else {
            ("—", Color32::from_rgb(70, 70, 80))
        };
        p.text(
            r.center(),
            Align2::CENTER_CENTER,
            text,
            FontId::proportional(15.0),
            color,
        );
        return;
    };
    if wf.peaks.is_empty() {
        return;
    }

    let n = wf.peaks.len();
    let progress = if inp.total > 0.0 {
        (inp.elapsed / inp.total).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let play_x = r.left() + r.width() * progress;

    // One thin vertical bar per pixel column, colored by whether it has played.
    let cols = r.width().max(1.0) as usize;
    for x in 0..cols {
        let fx = x as f32 / cols as f32;
        let idx = ((fx * n as f32) as usize).min(n - 1);
        let amp = wf.peaks[idx] * half_h;
        let px = r.left() + x as f32;
        let played = px <= play_x;
        let col = if played {
            Color32::from_rgb(255, 64, 90)
        } else {
            Color32::from_rgb(70, 70, 82)
        };
        p.line_segment(
            [pos2(px, mid - amp), pos2(px, mid + amp)],
            Stroke::new(1.0, col),
        );
    }

    // Playhead.
    p.line_segment(
        [pos2(play_x, r.top()), pos2(play_x, r.bottom())],
        Stroke::new(1.5, Color32::from_white_alpha(230)),
    );

    // Time ticks at 1/4, 1/2, 3/4.
    if inp.total > 0.0 {
        for frac in [0.25, 0.5, 0.75] {
            let tx = r.left() + r.width() * frac;
            let secs = (inp.total * frac) as u32;
            let label = format!("{}:{:02}", secs / 60, secs % 60);
            p.text(
                pos2(tx, r.bottom() - 2.0),
                Align2::CENTER_BOTTOM,
                label,
                FontId::proportional(10.0),
                Color32::from_rgb(120, 120, 130),
            );
        }
    }
}

fn draw_spectrum(p: &egui::Painter, r: Rect, st: &LabState, d: &Derived) {
    grid(p, r);

    let half = d.mag.len();
    let baseline = r.bottom();
    let cols = r.width().max(1.0) as usize;
    let mut top: Vec<Pos2> = Vec::with_capacity(cols);

    for x in 0..cols {
        let fx = x as f32 / cols as f32;
        let freq = F_MIN * (F_MAX / F_MIN).powf(fx);
        let k = (freq * FFT_SIZE as f32 / d.sr).round() as usize;
        let m = d.mag.get(k.min(half.saturating_sub(1))).copied().unwrap_or(0.0);
        // Display smoothing per source bin would need alignment; smooth in the
        // display domain instead.
        let db = to_db(m);
        let v = ((db + 90.0) / 88.0).clamp(0.0, 1.0);
        let px = r.left() + x as f32;
        let py = baseline - v * r.height();
        // Translucent fill under the curve.
        p.line_segment(
            [pos2(px, baseline), pos2(px, py)],
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(190, 40, 60, 70)),
        );
        top.push(pos2(px, py));
    }
    let _ = st;
    if top.len() >= 2 {
        p.add(egui::Shape::line(
            top,
            Stroke::new(1.4, Color32::from_rgb(90, 200, 235)),
        ));
    }

    // Frequency axis labels.
    for (freq, label) in [(100.0, "100Hz"), (1000.0, "1kHz"), (10000.0, "10kHz")] {
        let fx = (freq / F_MIN).log10() / (F_MAX / F_MIN).log10();
        let tx = r.left() + r.width() * fx;
        p.text(
            pos2(tx, r.top() + 2.0),
            Align2::CENTER_TOP,
            label,
            FontId::proportional(10.0),
            Color32::from_rgb(110, 110, 120),
        );
    }
}

fn draw_spectrogram(p: &egui::Painter, r: Rect, st: &LabState) {
    if let Some(tex) = &st.spectro_tex {
        p.image(
            tex.id(),
            r,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    } else {
        p.rect_filled(r, Rounding::ZERO, Color32::from_rgb(10, 8, 14));
    }

    // Frequency markers down the left edge.
    for (freq, label) in [(100.0, "100"), (1000.0, "1k"), (10000.0, "10k")] {
        let t = (freq / F_MIN).log10() / (F_MAX / F_MIN).log10();
        let ty = r.bottom() - t * r.height();
        p.text(
            pos2(r.left() + 3.0, ty),
            Align2::LEFT_CENTER,
            label,
            FontId::proportional(10.0),
            Color32::from_white_alpha(160),
        );
    }
}

fn draw_oscilloscope(p: &egui::Painter, r: Rect, st: &LabState) {
    grid(p, r);
    let n = st.buf_l.len();
    if n == 0 {
        return;
    }
    // Show the most recent ~1/4 of the window for a readable trace.
    let span = (n / 4).max(2);
    let start = n - span;
    let cols = r.width().max(1.0) as usize;
    let mut pts: Vec<Pos2> = Vec::with_capacity(cols);
    for x in 0..cols {
        let fx = x as f32 / (cols - 1).max(1) as f32;
        let i = start + (fx * (span - 1) as f32) as usize;
        let s = 0.5 * (st.buf_l[i] + st.buf_r[i]);
        let py = r.center().y - s * r.height() * 0.45;
        pts.push(pos2(r.left() + x as f32, py));
    }
    p.add(egui::Shape::line(
        pts,
        Stroke::new(1.3, Color32::from_rgb(255, 80, 95)),
    ));
}

fn draw_vectorscope(p: &egui::Painter, r: Rect, st: &LabState, d: &Derived, corr_label: &str) {
    let c = r.center();
    let radius = r.height().min(r.width()) * 0.42;

    // Diamond guides + axis labels (M top, S sides, L/R diagonals).
    let guide = Color32::from_rgb(45, 45, 52);
    let top = pos2(c.x, c.y - radius);
    let bottom = pos2(c.x, c.y + radius);
    let left = pos2(c.x - radius, c.y);
    let right = pos2(c.x + radius, c.y);
    p.add(egui::Shape::line(
        vec![top, right, bottom, left, top],
        Stroke::new(1.0, guide),
    ));
    for (pos, label) in [
        (pos2(c.x, c.y - radius - 8.0), "M"),
        (pos2(c.x - radius * 0.72, c.y - radius * 0.72), "L"),
        (pos2(c.x + radius * 0.72, c.y - radius * 0.72), "R"),
        (pos2(c.x - radius - 8.0, c.y), "S"),
        (pos2(c.x + radius + 8.0, c.y), "S"),
    ] {
        p.text(
            pos,
            Align2::CENTER_CENTER,
            label,
            FontId::proportional(10.0),
            Color32::from_rgb(120, 120, 130),
        );
    }

    // Lissajous: x = side, y = mid (up). Sub-sample so we plot a few hundred
    // points rather than the whole window.
    let n = st.buf_l.len();
    if n > 0 {
        let inv_sqrt2 = std::f32::consts::FRAC_1_SQRT_2;
        let step = (n / 400).max(1);
        let mut i = 0;
        while i < n {
            let l = st.buf_l[i];
            let rr = st.buf_r[i];
            let side = (l - rr) * inv_sqrt2;
            let mid = (l + rr) * inv_sqrt2;
            let px = c.x + side * radius;
            let py = c.y - mid * radius;
            p.circle_filled(pos2(px, py), 1.0, Color32::from_rgb(120, 240, 130));
            i += step;
        }
    }

    // Correlation readout + bar at the bottom.
    let bar = Rect::from_min_max(
        pos2(r.left() + 6.0, r.bottom() - 8.0),
        pos2(r.right() - 6.0, r.bottom() - 4.0),
    );
    p.rect_filled(bar, Rounding::same(2.0), Color32::from_rgb(35, 35, 40));
    let t = (d.corr + 1.0) * 0.5;
    let fill = Rect::from_min_max(bar.min, pos2(bar.left() + bar.width() * t, bar.bottom()));
    p.rect_filled(fill, Rounding::same(2.0), Color32::from_rgb(90, 200, 235));
    p.text(
        pos2(c.x, r.bottom() - 16.0),
        Align2::CENTER_CENTER,
        format!("{} {:.2}", corr_label, d.corr),
        FontId::proportional(11.0),
        Color32::from_rgb(170, 170, 180),
    );
}

fn draw_loudness(p: &egui::Painter, r: Rect, st: &LabState) {
    // Big momentary LUFS number.
    p.text(
        pos2(r.center().x, r.top() + 28.0),
        Align2::CENTER_CENTER,
        format!("{:.1} LUFS", st.lufs_m),
        FontId::proportional(30.0),
        Color32::WHITE,
    );

    // Horizontal meter (-60..0) with the momentary marker and short-term tick.
    let bar = Rect::from_min_max(
        pos2(r.left() + 8.0, r.top() + 56.0),
        pos2(r.right() - 8.0, r.top() + 72.0),
    );
    meter_gradient(p, bar);
    let m_t = ((st.lufs_m + 60.0) / 60.0).clamp(0.0, 1.0);
    let s_t = ((st.lufs_s + 60.0) / 60.0).clamp(0.0, 1.0);
    let mx = bar.left() + bar.width() * m_t;
    let sx = bar.left() + bar.width() * s_t;
    p.line_segment(
        [pos2(mx, bar.top() - 2.0), pos2(mx, bar.bottom() + 2.0)],
        Stroke::new(2.0, Color32::WHITE),
    );
    p.line_segment(
        [pos2(sx, bar.top() - 2.0), pos2(sx, bar.bottom() + 2.0)],
        Stroke::new(2.0, Color32::from_rgb(90, 200, 235)),
    );
    // Endpoint scale labels.
    p.text(
        pos2(bar.left(), bar.bottom() + 8.0),
        Align2::LEFT_TOP,
        "-60",
        FontId::proportional(9.0),
        Color32::from_rgb(110, 110, 120),
    );
    p.text(
        pos2(bar.right(), bar.bottom() + 8.0),
        Align2::RIGHT_TOP,
        "0",
        FontId::proportional(9.0),
        Color32::from_rgb(110, 110, 120),
    );

    // M / S / INT / LRA / PK sub-readouts.
    let lra = (st.lufs_hi - st.lufs_lo).clamp(0.0, 30.0);
    let stats = [
        ("M", format!("{:.1}", st.lufs_m)),
        ("S", format!("{:.1}", st.lufs_s)),
        ("INT", format!("{:.1}", st.lufs_i)),
        ("LRA", format!("{:.1}", lra)),
        ("PK", format!("{:.1}", st.peak_hold_l.max(st.peak_hold_r))),
    ];
    let y = r.bottom() - 16.0;
    let n = stats.len();
    for (i, (k, v)) in stats.iter().enumerate() {
        let x = r.left() + r.width() * (i as f32 + 0.5) / n as f32;
        p.text(
            pos2(x, y - 8.0),
            Align2::CENTER_CENTER,
            *k,
            FontId::proportional(10.0),
            Color32::from_rgb(130, 130, 140),
        );
        p.text(
            pos2(x, y + 6.0),
            Align2::CENTER_CENTER,
            v,
            FontId::proportional(12.0),
            Color32::from_rgb(90, 200, 235),
        );
    }
}

fn draw_levels(p: &egui::Painter, r: Rect, st: &LabState, d: &Derived) {
    let bar_h = 22.0;
    let gap = 18.0;
    let top = r.top() + 8.0;

    for (i, (label, rms_db, peak_db)) in [
        ("L", d.rms_l_db, st.peak_hold_l),
        ("R", d.rms_r_db, st.peak_hold_r),
    ]
    .iter()
    .enumerate()
    {
        let y = top + i as f32 * (bar_h + gap);
        let bar = Rect::from_min_max(
            pos2(r.left() + 28.0, y + 14.0),
            pos2(r.right() - 8.0, y + 14.0 + bar_h),
        );
        // Channel label + peak number.
        p.text(
            pos2(r.left() + 2.0, y + 8.0),
            Align2::LEFT_CENTER,
            *label,
            FontId::proportional(12.0),
            Color32::from_rgb(180, 180, 190),
        );
        p.text(
            pos2(r.left() + 28.0, y + 8.0),
            Align2::LEFT_CENTER,
            format!("{:.1}", peak_db),
            FontId::proportional(11.0),
            Color32::from_rgb(150, 150, 160),
        );
        // Background track + gradient fill to current RMS.
        p.rect_filled(bar, Rounding::same(3.0), Color32::from_rgb(24, 24, 28));
        let t = ((rms_db + 60.0) / 60.0).clamp(0.0, 1.0);
        let fill = Rect::from_min_max(bar.min, pos2(bar.left() + bar.width() * t, bar.bottom()));
        meter_gradient(p, fill);
        // Peak-hold tick.
        let pk_t = ((*peak_db + 60.0) / 60.0).clamp(0.0, 1.0);
        let px = bar.left() + bar.width() * pk_t;
        p.line_segment(
            [pos2(px, bar.top()), pos2(px, bar.bottom())],
            Stroke::new(2.0, Color32::WHITE),
        );
    }

    // dB scale along the bottom.
    for db in [-60, -48, -36, -24, -12, -6, 0] {
        let t = ((db as f32 + 60.0) / 60.0).clamp(0.0, 1.0);
        let x = (r.left() + 28.0) + (r.right() - 8.0 - (r.left() + 28.0)) * t;
        p.text(
            pos2(x, r.bottom() - 2.0),
            Align2::CENTER_BOTTOM,
            format!("{db}"),
            FontId::proportional(9.0),
            Color32::from_rgb(100, 100, 110),
        );
    }
}

// ---------------------------------------------------------------------------
// Drawing helpers
// ---------------------------------------------------------------------------

/// Faint background grid shared by the spectrum/oscilloscope cards.
fn grid(p: &egui::Painter, r: Rect) {
    let col = Color32::from_rgb(28, 28, 34);
    for i in 1..4 {
        let x = r.left() + r.width() * i as f32 / 4.0;
        p.line_segment([pos2(x, r.top()), pos2(x, r.bottom())], Stroke::new(1.0, col));
    }
    for i in 1..3 {
        let y = r.top() + r.height() * i as f32 / 3.0;
        p.line_segment([pos2(r.left(), y), pos2(r.right(), y)], Stroke::new(1.0, col));
    }
}

/// Green→yellow→red horizontal gradient used by the loudness/level meters.
fn meter_gradient(p: &egui::Painter, r: Rect) {
    let cols = r.width().max(1.0) as usize;
    for x in 0..cols {
        let t = x as f32 / cols as f32;
        let col = if t < 0.6 {
            lerp_color(
                Color32::from_rgb(40, 200, 120),
                Color32::from_rgb(220, 210, 70),
                t / 0.6,
            )
        } else {
            lerp_color(
                Color32::from_rgb(220, 210, 70),
                Color32::from_rgb(230, 70, 60),
                (t - 0.6) / 0.4,
            )
        };
        let px = r.left() + x as f32;
        p.line_segment([pos2(px, r.top()), pos2(px, r.bottom())], Stroke::new(1.0, col));
    }
}

// ---------------------------------------------------------------------------
// Spectrogram texture upload (called from the App glue each frame)
// ---------------------------------------------------------------------------

/// Rebuilds the spectrogram texture from the column history. Must run with the
/// viewport context *before* the scroll area draws the spectrogram card.
pub fn update_spectrogram_texture(ctx: &egui::Context, st: &mut LabState) {
    if !st.show[2] {
        return;
    }
    let w = SPECTRO_COLS;
    let h = SPECTRO_BINS;
    let mut pixels = vec![Color32::from_rgb(8, 6, 12); w * h];
    let cols = st.spectro.len();
    for (cx, col) in st.spectro.iter().enumerate() {
        // Right-align newest columns so history scrolls in from the right.
        let x = w - cols + cx;
        for bin in 0..h {
            // Row 0 (top) = highest frequency.
            let y = h - 1 - bin;
            let v = col[bin] as f32 / 255.0;
            pixels[y * w + x] = magma(v);
        }
    }
    let image = egui::ColorImage {
        size: [w, h],
        pixels,
    };
    match &mut st.spectro_tex {
        Some(tex) => tex.set(image, TextureOptions::LINEAR),
        None => {
            st.spectro_tex =
                Some(ctx.load_texture("lab_spectrogram", image, TextureOptions::LINEAR));
        }
    }
}

// ---------------------------------------------------------------------------
// Small math
// ---------------------------------------------------------------------------

fn to_db(x: f32) -> f32 {
    20.0 * (x.max(1e-7)).log10()
}

fn db_to_lin(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    Color32::from_rgb(
        lerp(a.r() as f32, b.r() as f32, t) as u8,
        lerp(a.g() as f32, b.g() as f32, t) as u8,
        lerp(a.b() as f32, b.b() as f32, t) as u8,
    )
}

/// Approximate "magma" colormap (black → purple → red → orange → yellow).
fn magma(t: f32) -> Color32 {
    let stops = [
        (0.0, (0, 0, 4)),
        (0.25, (60, 15, 90)),
        (0.5, (160, 40, 90)),
        (0.75, (240, 100, 60)),
        (1.0, (252, 230, 160)),
    ];
    let t = t.clamp(0.0, 1.0);
    for w in stops.windows(2) {
        let (t0, c0) = w[0];
        let (t1, c1) = w[1];
        if t <= t1 {
            let f = (t - t0) / (t1 - t0).max(1e-6);
            return Color32::from_rgb(
                lerp(c0.0 as f32, c1.0 as f32, f) as u8,
                lerp(c0.1 as f32, c1.1 as f32, f) as u8,
                lerp(c0.2 as f32, c1.2 as f32, f) as u8,
            );
        }
    }
    Color32::from_rgb(252, 230, 160)
}
