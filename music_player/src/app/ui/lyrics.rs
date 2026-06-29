//! Drag-and-drop hint overlay.

use crate::app::App;
use crate::lang::Lang;
use eframe::egui;
use egui::{Color32, FontId};

impl App {
    /// Draws a full-screen hint while the user is dragging files over the window.
    pub(in crate::app) fn ui_drop_hint(&mut self, ctx: &egui::Context) {
        let hovering_files = ctx.input(|i| !i.raw.hovered_files.is_empty());
        if !hovering_files {
            return;
        }

        let screen = ctx.screen_rect();
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("drop_files_overlay"),
        ));
        painter.rect_filled(screen, egui::Rounding::same(0.0), Color32::from_black_alpha(150));
        let text = match self.language {
            Lang::Ru => "Отпустите, чтобы добавить в Elysium",
            Lang::Uk => "Відпустіть, щоб додати в Elysium",
            Lang::En => "Drop to add to Elysium",
        };
        painter.text(
            screen.center(),
            egui::Align2::CENTER_CENTER,
            text,
            FontId::proportional(28.0),
            Color32::WHITE,
        );
    }
}
