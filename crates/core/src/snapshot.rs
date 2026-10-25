//! A frozen view of the project for rendering.
//!
//! The project is copy-on-write (`Arc<Project>`), so a snapshot costs nothing to take and the
//! render and audio threads can keep using it while the user keeps editing. Playback and preview
//! only ever read from a snapshot.

use std::sync::Arc;

use filmcraft_engine::{MediaPool, Services};
use filmcraft_frame::AudioBuffer;
use filmcraft_project::{ItemId, Project};
use filmcraft_render::RenderOptions;
use filmcraft_time::{FrameRate, Tick};

use crate::editor::Editor;

/// One picture for the screen: opaque sRGB RGBA8, `width * height * 4` bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct PreviewFrame {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

impl PreviewFrame {
    /// A black picture, for when nothing can be drawn.
    pub fn black(width: usize, height: usize) -> Self {
        let (width, height) = (width.max(1), height.max(1));
        let mut rgba = vec![0u8; width * height * 4];
        for px in rgba.as_chunks_mut::<4>().0.iter_mut() {
            px[3] = 255;
        }
        Self { width, height, rgba }
    }
}

#[derive(Clone)]
pub struct Snapshot {
    project: Arc<Project>,
    sequence: ItemId,
    pool: Arc<MediaPool>,
    services: Arc<dyn Services>,
    /// The editor's revision when the snapshot was taken; a different one means the project changed.
    pub revision: u64,
}

impl Snapshot {
    /// The open sequence as it is now, or None when no sequence is open.
    pub fn of(editor: &Editor) -> Option<Snapshot> {
        let s = editor.session();
        let sequence = s.state.active_sequence?;
        s.project.sequence(sequence)?;
        Some(Snapshot { project: s.project.clone(), sequence, pool: s.media.clone(), services: s.services.clone(), revision: s.revision })
    }

    /// The sequence's length: the end of its last clip.
    pub fn duration(&self) -> Tick {
        self.project.sequence(self.sequence).map(|q| q.duration()).unwrap_or_default()
    }

    pub fn frame_rate(&self) -> FrameRate {
        self.project.sequence(self.sequence).map(|q| q.settings.frame_rate).unwrap_or_default()
    }

    /// The sequence's audio sample rate (never 0).
    pub fn sample_rate(&self) -> u32 {
        self.project.sequence(self.sequence).map(|q| q.settings.sample_rate).filter(|r| *r > 0).unwrap_or(48_000)
    }

    /// The sequence's picture at `t`, scaled (1.0 = full size), captions included.
    pub fn render_frame(&self, t: Tick, scale: f32) -> PreviewFrame {
        let provider = self.pool.provider(self.project.clone(), self.services.clone());
        let scale = if scale.is_finite() { scale.clamp(0.05, 1.0) } else { 0.5 };
        let opts = RenderOptions { scale, captions: true, ..Default::default() };
        let img = filmcraft_render::render_sequence(&self.project, self.sequence, Tick(t.0.max(0)), opts, &provider);
        PreviewFrame { width: img.w, height: img.h, rgba: img.over_black_rgba8() }
    }

    /// One media file's picture at media time `t` (the library preview), or None for audio-only.
    pub fn render_media_frame(&self, item: ItemId, t: Tick, scale: f32) -> Option<PreviewFrame> {
        let provider = self.pool.provider(self.project.clone(), self.services.clone());
        let scale = if scale.is_finite() { scale.clamp(0.05, 1.0) } else { 0.5 };
        let img = filmcraft_render::render_item(&self.project, item, Tick(t.0.max(0)), scale, &provider)?;
        Some(PreviewFrame { width: img.w, height: img.h, rgba: img.over_black_rgba8() })
    }

    /// `frames` stereo samples of the sequence's sound starting at sample `start` (at
    /// [`Self::sample_rate`]), through the full mixer: volumes, mutes and music included.
    pub fn mix_audio(&self, start: i64, frames: usize) -> Option<AudioBuffer> {
        let q = self.project.sequence(self.sequence)?;
        let provider = self.pool.provider(self.project.clone(), self.services.clone());
        Some(filmcraft_render::audio::mix_sequence(&self.project, q, start.max(0), frames, &provider))
    }
}
