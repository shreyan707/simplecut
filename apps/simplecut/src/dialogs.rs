//! The operating system's file dialogs.

use std::path::PathBuf;

use simplecut_ui::Dialogs;

const MEDIA: &[&str] =
    &["mp4", "mov", "m4v", "mkv", "webm", "avi", "ts", "mts", "m2ts", "mxf", "mp3", "wav", "aac", "m4a", "flac", "ogg", "opus", "aif", "aiff"];

pub struct NativeDialogs;

impl Dialogs for NativeDialogs {
    fn pick_media(&self) -> Vec<PathBuf> {
        rfd::FileDialog::new()
            .set_title("Import video or audio")
            .add_filter("Video and audio", MEDIA)
            .add_filter("All files", &["*"])
            .pick_files()
            .unwrap_or_default()
    }

    fn pick_project_to_open(&self) -> Option<PathBuf> {
        rfd::FileDialog::new().set_title("Open project").add_filter("Projects", &["fcproj"]).pick_file()
    }

    fn pick_project_save_path(&self, suggested_name: &str) -> Option<PathBuf> {
        let mut path = rfd::FileDialog::new().set_title("Save project").add_filter("Projects", &["fcproj"]).set_file_name(suggested_name).save_file()?;
        if path.extension().is_none() {
            path.set_extension("fcproj");
        }
        Some(path)
    }
}
