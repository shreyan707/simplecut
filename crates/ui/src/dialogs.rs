//! File dialogs, behind a trait so the window can be driven by tests with no operating-system
//! dialogs. The app supplies the real ones.

use std::path::PathBuf;

pub trait Dialogs {
    /// Choose video / audio files to import. Empty: cancelled.
    fn pick_media(&self) -> Vec<PathBuf>;
    /// Choose a project to open.
    fn pick_project_to_open(&self) -> Option<PathBuf>;
    /// Choose where to save the project; `suggested_name` is a file name to offer.
    fn pick_project_save_path(&self, suggested_name: &str) -> Option<PathBuf>;
}

/// Dialogs that are always cancelled.
pub struct NoDialogs;

impl Dialogs for NoDialogs {
    fn pick_media(&self) -> Vec<PathBuf> {
        Vec::new()
    }

    fn pick_project_to_open(&self) -> Option<PathBuf> {
        None
    }

    fn pick_project_save_path(&self, _suggested_name: &str) -> Option<PathBuf> {
        None
    }
}
