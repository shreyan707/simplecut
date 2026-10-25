//! The top bar: project, import, undo / redo and the editing buttons.

use egui::{Align, Layout, RichText};

use crate::app::{Action, SimpleCutApp};
use crate::theme;

pub(crate) fn show(app: &mut SimpleCutApp, ui: &mut egui::Ui) {
    let t = theme::tokens();
    let can_undo = app.editor.can_undo();
    let can_redo = app.editor.can_redo();
    let has_selection = !app.editor.selection().is_empty();
    let has_timeline = app.editor.sequence().is_some_and(|s| s.clip_count() > 0);
    ui.horizontal(|ui| {
        ui.add_space(2.0);
        button(app, ui, "New", "Start a new project (Ctrl+N)", Action::NewProject, true);
        button(app, ui, "Open", "Open a project (Ctrl+O)", Action::OpenProject, true);
        button(app, ui, "Save", "Save the project (Ctrl+S)", Action::Save, true);
        ui.separator();
        button(app, ui, "Import", "Add video or audio files to the project (Ctrl+I)", Action::Import, true);
        ui.separator();
        button(app, ui, "Undo", "Undo the last change (Ctrl+Z)", Action::Undo, can_undo);
        button(app, ui, "Redo", "Redo (Ctrl+Shift+Z)", Action::Redo, can_redo);
        ui.separator();
        button(app, ui, "Split", "Cut the clip at the playhead (S)", Action::Split, has_timeline);
        button(app, ui, "Delete", "Delete the selected clips and close the gap (Delete)", Action::Delete, has_selection);

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_enabled(false, egui::Button::new("Export")).on_disabled_hover_text("Exporting arrives in the next update.");
            ui.add_space(8.0);
            let dirty = app.editor.is_dirty();
            let name = app.editor.project_name().to_string();
            let label = if dirty { format!("{name} •") } else { name };
            ui.label(RichText::new(label).color(if dirty { t.text } else { t.text_dim }));
        });
    });
}

fn button(app: &mut SimpleCutApp, ui: &mut egui::Ui, label: &str, tip: &str, action: Action, enabled: bool) {
    let r = ui.add_enabled(enabled, egui::Button::new(label)).on_hover_text(tip);
    if r.clicked() {
        app.queue(action);
    }
}
