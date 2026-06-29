//! Glue between the `App` and the standalone Lab window.
//!
//! The Lab itself is a self-contained renderer in [`crate::lab`]. This module
//! only opens the second OS window (an egui *immediate* viewport, so the
//! closure can borrow live app state) and feeds it the current audio handles
//! and playback position each frame.

use crate::app::App;
use eframe::egui;

impl App {
    /// Renders the Lab in its own window while [`App::show_lab`] is set.
    pub(in crate::app) fn ui_lab(&mut self, ctx: &egui::Context) {
        if !self.show_lab {
            return;
        }

        // Drop a waveform left over from a previous track (e.g. a slow decode
        // that finished after the user skipped) so the Lab only ever shows the
        // current song's shape.
        if let Ok(mut wf) = self.waveform.lock() {
            if wf.as_ref().is_some_and(|w| w.path != self.current_song) {
                *wf = None;
            }
        }

        // Pull everything the renderer needs out of `self` up front so the
        // viewport closure only has to borrow the two Lab-specific fields.
        let analysis = self.player.analysis();
        let waveform = self.waveform.clone();
        let elapsed = self.elapsed_duration.as_secs_f32();
        let total = self.total_duration.map(|d| d.as_secs_f32()).unwrap_or(0.0);
        let playing = self.is_playing;
        // A track is loaded but the waveform slot is still empty → it is being
        // rendered on the background thread spawned by `play_track`.
        let waveform_loading = !self.current_song.is_empty()
            && self.waveform.lock().map(|w| w.is_none()).unwrap_or(false);
        let lang = self.language;
        let title = format!("Elysium — {}", crate::lang::strings(lang).lab_button);
        let lab_state = &mut self.lab_state;

        let mut keep_open = true;

        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("elysium_lab_window"),
            egui::ViewportBuilder::default()
                .with_title(title)
                .with_inner_size([1400.0, 900.0])
                .with_min_inner_size([760.0, 480.0]),
            |vctx, _class| {
                // Honor the window's close button.
                if vctx.input(|i| i.viewport().close_requested()) {
                    keep_open = false;
                }

                // Refresh the spectrogram texture against THIS viewport's
                // context before the renderer draws it.
                crate::lab::update_spectrogram_texture(vctx, lab_state);

                let inp = crate::lab::LabInputs {
                    analysis: &analysis,
                    waveform: &waveform,
                    elapsed,
                    total,
                    playing,
                    waveform_loading,
                    lang,
                };

                egui::CentralPanel::default()
                    .frame(egui::Frame::none().fill(egui::Color32::from_rgb(10, 10, 13)).inner_margin(14.0))
                    .show(vctx, |ui| {
                        crate::lab::draw_lab(ui, lab_state, &inp);
                    });

                vctx.request_repaint();
            },
        );

        if !keep_open {
            self.show_lab = false;
        }
    }
}
