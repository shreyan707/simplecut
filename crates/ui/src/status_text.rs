//! Words the interface says when there is nothing to show yet.

/// The preview of an empty timeline: a headline and what to do about it.
pub fn empty_timeline() -> (String, String) {
    ("Your timeline is empty".into(), "Import a video, then double-click it in the library to add it.".into())
}
