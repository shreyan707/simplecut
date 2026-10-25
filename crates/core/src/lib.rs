//! SimpleCut's headless editor layer.
//!
//! No UI crates here (the layering rule FilmCraft enforces): everything in this crate runs and is
//! tested without a window. [`Editor`] is a thin, typed front over FilmCraft's engine `Session`,
//! so projects are FilmCraft's native `.fcproj` and undo/redo is the engine's.

mod editor;
mod error;
mod player;
mod renderer;
mod snapshot;
mod view;

pub use editor::{Edge, Editor, ItemId, Lane, PlaceAt, TrimMode};
pub use error::{EditorError, Result};
pub use player::{AudioOutput, FillFn, PlayState, Player};
pub use renderer::{PreviewRenderer, RenderedFrame};
pub use snapshot::{PreviewFrame, Snapshot};
pub use view::{ClipView, ImportReport, MediaEntry, SequenceView, TrackView};

pub use filmcraft_media::MediaKind;
pub use filmcraft_project::{ClipId, TrackId, TrackKind};
pub use filmcraft_time::Tick;

/// The FilmCraft commit this crate is built and tested against.
pub const FILMCRAFT_BASE_COMMIT: &str = "a504f3c5";
