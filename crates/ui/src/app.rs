//! The application: state, actions, and the per-frame loop.
//!
//! Everything the user can do is an [`Action`]. Buttons, shortcuts and tests all go through
//! [`SimpleCutApp::perform`], so there is one path for each thing and one place that reports its
//! errors. The panels only draw and queue actions.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use egui::{Align2, Key, Modifiers};
use simplecut_core::{
    AudioOutput, Editor, EditorError, FrameRate, ItemId, Lane, MediaEntry, PlaceAt, PlayState, Player, PreviewFrame, PreviewRenderer, Snapshot, Target,
    Thumbnails, Tick,
};

use crate::dialogs::Dialogs;
use crate::{library, preview, theme, timeline, toolbar};

/// What the preview area shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The edit, at the playhead.
    Timeline,
    /// One imported file, before it is placed.
    Media(ItemId),
}

/// Everything the user can ask for.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    NewProject,
    OpenProject,
    Save,
    SaveAs,
    /// Choose files with the file dialog, then import them.
    Import,
    ImportPaths(Vec<PathBuf>),
    Undo,
    Redo,
    TogglePlay,
    SeekStart,
    SeekEnd,
    /// Move by this many frames (negative: back).
    Step(i64),
    /// Go to a time in whatever the preview shows.
    Seek(Tick),
    /// Go to a time on the timeline and show the timeline.
    SeekTimeline(Tick),
    PreviewMedia(ItemId),
    ShowTimeline,
    AddToTimeline(ItemId, Lane),
    Split,
    Delete,
    DismissBanner,
}

/// A line of feedback under the work area.
#[derive(Debug, Clone, PartialEq)]
pub struct Banner {
    pub text: String,
    pub error: bool,
}

#[derive(Debug, Clone, PartialEq)]
enum Then {
    New,
    Open(PathBuf),
    Close,
}

#[derive(Debug, Clone, PartialEq)]
enum Modal {
    Unsaved(Then),
}

/// A library thumbnail.
pub(crate) enum Thumb {
    Pending,
    /// The file has no picture, or it could not be read.
    None,
    Ready(egui::TextureHandle),
}

/// What the preview picture was last drawn for; a different one means ask again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Want {
    source: Source,
    revision: u64,
    frame: i64,
    scale_step: u8,
}

pub struct SimpleCutApp {
    pub(crate) editor: Editor,
    pub(crate) player: Player,
    renderer: PreviewRenderer,
    thumbs: Thumbnails,
    dialogs: Box<dyn Dialogs>,

    pub(crate) source: Source,
    /// Position in the previewed file (the timeline's is the editor's playhead).
    pub(crate) media_time: Tick,
    pub(crate) thumb_state: HashMap<ItemId, Thumb>,
    media_cache: (u64, Option<std::time::Instant>, Vec<MediaEntry>),
    pub(crate) preview_texture: Option<egui::TextureHandle>,
    last_asked: Option<Want>,
    /// Width in points of the preview area as last drawn, to pick a picture size.
    pub(crate) preview_width: f32,

    pub(crate) banner: Option<Banner>,
    modal: Option<Modal>,
    close_now: bool,
    title: String,
    queue: Vec<Action>,
}

impl SimpleCutApp {
    /// A window on a new empty project. `output` is the sound card (None: no sound).
    pub fn new(ctx: &egui::Context, dialogs: Box<dyn Dialogs>, output: Option<Box<dyn AudioOutput>>) -> Self {
        theme::apply(ctx);
        let mut editor = Editor::new();
        let mut banner = None;
        if let Err(e) = editor.new_project("Untitled") {
            banner = Some(Banner { text: format!("Could not start a new project: {e}"), error: true });
        }
        Self {
            editor,
            player: Player::new(output),
            renderer: PreviewRenderer::new(),
            thumbs: Thumbnails::new(),
            dialogs,
            source: Source::Timeline,
            media_time: Tick::ZERO,
            thumb_state: HashMap::new(),
            media_cache: (u64::MAX, None, Vec::new()),
            preview_texture: None,
            last_asked: None,
            preview_width: 640.0,
            banner,
            modal: None,
            close_now: false,
            title: String::new(),
            queue: Vec::new(),
        }
    }

    // ── for tests and the app shell ────────────────────────────────────────

    pub fn editor(&self) -> &Editor {
        &self.editor
    }

    pub fn editor_mut(&mut self) -> &mut Editor {
        &mut self.editor
    }

    pub fn player(&self) -> &Player {
        &self.player
    }

    pub fn source(&self) -> Source {
        self.source
    }

    pub fn banner(&self) -> Option<&Banner> {
        self.banner.as_ref()
    }

    /// Size in pixels of the picture currently on screen in the preview.
    pub fn preview_size(&self) -> Option<[usize; 2]> {
        self.preview_texture.as_ref().map(|t| t.size())
    }

    /// Whether an "unsaved changes" question is waiting for an answer.
    pub fn asking_about_unsaved(&self) -> bool {
        self.modal.is_some()
    }

    /// The thumbnail state of a library item: 'pending', 'none' or 'ready'.
    pub fn thumbnail_state(&self, item: ItemId) -> Option<&'static str> {
        self.thumb_state.get(&item).map(|t| match t {
            Thumb::Pending => "pending",
            Thumb::None => "none",
            Thumb::Ready(_) => "ready",
        })
    }

    /// Open a project file given at launch.
    pub fn open_project_path(&mut self, path: PathBuf) {
        self.open_now(path);
    }

    pub(crate) fn queue(&mut self, a: Action) {
        self.queue.push(a);
    }

    pub(crate) fn media(&mut self) -> Vec<MediaEntry> {
        let revision = self.editor.session().revision;
        let stale = self.media_cache.1.is_none_or(|t| t.elapsed() > Duration::from_secs(2));
        if self.media_cache.0 != revision || stale {
            self.media_cache = (revision, Some(std::time::Instant::now()), self.editor.media());
        }
        self.media_cache.2.clone()
    }

    // ── actions ────────────────────────────────────────────────────────────

    fn info(&mut self, text: impl Into<String>) {
        self.banner = Some(Banner { text: text.into(), error: false });
    }

    fn fail(&mut self, e: &EditorError) {
        self.banner = Some(Banner { text: e.to_string(), error: true });
    }

    /// Show an error if there is one; otherwise hand the value back.
    fn check<T>(&mut self, r: Result<T, EditorError>) -> Option<T> {
        match r {
            Ok(v) => Some(v),
            Err(e) => {
                self.fail(&e);
                None
            }
        }
    }

    pub fn perform(&mut self, action: Action) {
        match action {
            Action::NewProject => self.guard_unsaved(Then::New),
            Action::OpenProject => {
                if let Some(path) = self.dialogs.pick_project_to_open() {
                    self.guard_unsaved(Then::Open(path));
                }
            }
            Action::Save => {
                self.save_project();
            }
            Action::SaveAs => {
                self.save_project_as();
            }
            Action::Import => {
                let paths = self.dialogs.pick_media();
                if !paths.is_empty() {
                    self.import(paths);
                }
            }
            Action::ImportPaths(paths) => self.import(paths),
            Action::Undo => match self.editor.undo() {
                Some(what) => self.info(format!("Undid {}", what.to_lowercase())),
                None => self.info("Nothing to undo."),
            },
            Action::Redo => match self.editor.redo() {
                Some(what) => self.info(format!("Redid {}", what.to_lowercase())),
                None => self.info("Nothing to redo."),
            },
            Action::TogglePlay => self.toggle_play(),
            Action::SeekStart => self.seek_in_source(Tick::ZERO),
            Action::SeekEnd => {
                let end = self.source_duration();
                self.seek_in_source(end);
            }
            Action::Step(n) => {
                let fd = self.source_frame_duration();
                let now = self.source_time();
                self.seek_in_source(Tick(now.0.saturating_add(fd.0.saturating_mul(n))));
            }
            Action::Seek(t) => self.seek_in_source(t),
            Action::SeekTimeline(t) => {
                self.source = Source::Timeline;
                self.seek_in_source(t);
            }
            Action::PreviewMedia(item) => {
                if self.player.is_playing() {
                    self.player.pause();
                }
                self.source = Source::Media(item);
                self.media_time = Tick::ZERO;
            }
            Action::ShowTimeline => {
                if matches!(self.source, Source::Media(_)) {
                    self.player.pause();
                }
                self.source = Source::Timeline;
            }
            Action::AddToTimeline(item, lane) => self.add_to_timeline(item, lane),
            Action::Split => {
                let r = self.editor.split_at_playhead();
                if let Some(n) = self.check(r) {
                    self.info(if n == 1 { "Split the clip at the playhead.".to_string() } else { format!("Split {n} clips at the playhead.") });
                }
            }
            Action::Delete => {
                let r = self.editor.delete_selected();
                if self.check(r).is_some() {
                    self.info("Deleted the selected clips.");
                }
            }
            Action::DismissBanner => self.banner = None,
        }
    }

    fn import(&mut self, paths: Vec<PathBuf>) {
        let first_project_media = self.editor.media().is_empty();
        match self.editor.import(&paths) {
            Ok(report) => {
                let n = report.imported.len();
                if report.failed.is_empty() {
                    self.info(if n == 1 { "Imported 1 file.".to_string() } else { format!("Imported {n} files.") });
                } else {
                    self.banner = Some(Banner { text: format!("Imported {n}, but some could not be used: {}", report.failed.join("; ")), error: true });
                }
                if first_project_media && let Some(first) = report.imported.first() {
                    self.source = Source::Media(*first);
                    self.media_time = Tick::ZERO;
                }
            }
            Err(e) => self.fail(&e),
        }
    }

    fn add_to_timeline(&mut self, item: ItemId, lane: Lane) {
        let at = match lane {
            Lane::Main => PlaceAt::End,
            Lane::Music => PlaceAt::At(Tick::ZERO),
        };
        self.player.pause();
        let r = self.editor.place(item, lane, at);
        if let Some(clips) = self.check(r) {
            let _ = self.editor.select(&clips);
            self.source = Source::Timeline;
            self.info("Added to the timeline.");
        }
    }

    fn toggle_play(&mut self) {
        match self.source {
            Source::Timeline => {
                let r = self.player.toggle(&mut self.editor);
                if let Err(e) = r {
                    self.fail(&e);
                }
            }
            Source::Media(item) => {
                if self.player.is_playing() {
                    self.player.pause();
                } else {
                    let r = self.player.play_media(&self.editor, item, self.media_time);
                    if let Err(e) = r {
                        self.fail(&e);
                    }
                }
            }
        }
    }

    fn seek_in_source(&mut self, to: Tick) {
        match self.source {
            Source::Timeline => self.player.seek(&mut self.editor, to),
            Source::Media(item) => {
                self.player.seek_media(&self.editor, item, to);
                self.media_time = self.player.position();
            }
        }
    }

    /// Where the preview is now.
    pub(crate) fn source_time(&self) -> Tick {
        match self.source {
            Source::Timeline => self.editor.playhead(),
            Source::Media(_) => self.media_time,
        }
    }

    pub(crate) fn source_snapshot(&self) -> Option<Snapshot> {
        match self.source {
            Source::Timeline => Snapshot::of(&self.editor),
            Source::Media(item) => Snapshot::of_media(&self.editor, item),
        }
    }

    pub(crate) fn source_duration(&self) -> Tick {
        self.source_snapshot().map(|s| s.duration()).unwrap_or_default()
    }

    pub(crate) fn source_frame_rate(&self) -> FrameRate {
        self.source_snapshot().map(|s| s.frame_rate()).unwrap_or_default()
    }

    fn source_frame_duration(&self) -> Tick {
        self.source_frame_rate().frame_duration()
    }

    /// True while the player is playing what the preview shows.
    pub(crate) fn playing_source(&self) -> bool {
        self.player.is_playing()
            && match (self.source, self.player.target()) {
                (Source::Timeline, Some(Target::Sequence(_))) => true,
                (Source::Media(a), Some(Target::Media(b))) => a == b,
                _ => false,
            }
    }

    // ── project: new / open / save ─────────────────────────────────────────

    fn guard_unsaved(&mut self, then: Then) {
        if self.editor.is_dirty() {
            self.modal = Some(Modal::Unsaved(then));
        } else {
            self.run_then(then);
        }
    }

    fn run_then(&mut self, then: Then) {
        match then {
            Then::New => self.new_now(),
            Then::Open(path) => self.open_now(path),
            Then::Close => self.close_now = true,
        }
    }

    /// Forget everything tied to the old project's item ids.
    fn reset_views(&mut self) {
        self.player.pause();
        self.source = Source::Timeline;
        self.media_time = Tick::ZERO;
        self.thumb_state.clear();
        self.thumbs = Thumbnails::new();
        self.preview_texture = None;
        self.last_asked = None;
        self.media_cache = (u64::MAX, None, Vec::new());
    }

    fn new_now(&mut self) {
        self.reset_views();
        let r = self.editor.new_project("Untitled");
        if self.check(r).is_some() {
            self.info("Started a new project.");
        }
    }

    fn open_now(&mut self, path: PathBuf) {
        self.reset_views();
        match self.editor.open(&path) {
            Ok(()) => {
                let name = self.editor.project_name().to_string();
                self.info(format!("Opened {name}."));
            }
            Err(e) => {
                self.fail(&e);
                // the old project is still there; start it afresh if opening replaced it with nothing
                if self.editor.sequence().is_none() {
                    let _ = self.editor.new_project("Untitled");
                }
            }
        }
    }

    /// Save, asking where first if the project has no file yet. True when it was saved.
    fn save_project(&mut self) -> bool {
        if self.editor.project_path().is_none() {
            return self.save_project_as();
        }
        let r = self.editor.save();
        self.check(r).is_some()
    }

    fn save_project_as(&mut self) -> bool {
        let suggested = format!("{}.fcproj", self.editor.project_name());
        let Some(path) = self.dialogs.pick_project_save_path(&suggested) else { return false };
        let r = self.editor.save_as(&path);
        let ok = self.check(r).is_some();
        if ok {
            self.info(format!("Saved {}.", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
        }
        ok
    }

    // ── the frame ──────────────────────────────────────────────────────────

    /// Everything that happens between drawing frames: the clock, thumbnails and preview pictures.
    fn pump(&mut self, ctx: &egui::Context) {
        let state = self.player.tick(&mut self.editor);
        if matches!(self.player.target(), Some(Target::Media(_))) && matches!(self.source, Source::Media(_)) {
            self.media_time = self.player.position();
        }
        if state == PlayState::Ended {
            ctx.request_repaint();
        }

        // thumbnails
        for entry in self.media() {
            if !self.thumb_state.contains_key(&entry.id)
                && let Some(snap) = Snapshot::of(&self.editor)
            {
                self.thumbs.request(snap, entry.id);
                self.thumb_state.insert(entry.id, Thumb::Pending);
            }
        }
        for (id, frame) in self.thumbs.take_ready() {
            let state = match frame {
                Some(f) => Thumb::Ready(ctx.load_texture(format!("thumb-{}", id.0), to_image(&f), egui::TextureOptions::LINEAR)),
                None => Thumb::None,
            };
            self.thumb_state.insert(id, state);
        }

        // the preview picture
        self.ask_for_picture();
        if let Some(done) = self.renderer.take_result() {
            let image = to_image(&done.frame);
            match &mut self.preview_texture {
                Some(t) => t.set(image, egui::TextureOptions::LINEAR),
                None => self.preview_texture = Some(ctx.load_texture("preview", image, egui::TextureOptions::LINEAR)),
            }
        }

        let busy = self.player.is_playing() || !self.renderer.is_idle() || !self.thumbs.is_idle();
        if busy {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }

    fn ask_for_picture(&mut self) {
        let Some(snapshot) = (if self.playing_source() { self.player.snapshot().cloned() } else { self.source_snapshot() }) else {
            if self.preview_texture.take().is_some() {
                self.last_asked = None;
            }
            return;
        };
        let time = self.source_time();
        let fd = snapshot.frame_rate().frame_duration().0.max(1);
        let reference = match self.source {
            Source::Timeline => self.editor.sequence().map(|s| s.width).unwrap_or(1920),
            Source::Media(item) => self.editor.media_entry(item).and_then(|m| m.video).map(|v| v.0).unwrap_or(1280),
        };
        let wanted = self.preview_width.max(64.0) / reference.max(1) as f32;
        let step: u8 = match wanted {
            w if w <= 0.25 => 1,
            w if w <= 0.5 => 2,
            w if w <= 0.75 => 3,
            _ => 4,
        };
        let want = Want { source: self.source, revision: snapshot.revision, frame: time.0.div_euclid(fd), scale_step: step };
        if self.last_asked != Some(want) {
            self.last_asked = Some(want);
            self.renderer.request(snapshot, time, f32::from(step) * 0.25);
        }
    }

    fn handle_keys(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let mut actions = Vec::new();
        ctx.input_mut(|i| {
            let mut key = |m: Modifiers, k: Key, a: Action| {
                if i.consume_key(m, k) {
                    actions.push(a);
                }
            };
            key(Modifiers::NONE, Key::Space, Action::TogglePlay);
            key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z, Action::Redo);
            key(Modifiers::COMMAND, Key::Y, Action::Redo);
            key(Modifiers::COMMAND, Key::Z, Action::Undo);
            key(Modifiers::COMMAND | Modifiers::SHIFT, Key::S, Action::SaveAs);
            key(Modifiers::COMMAND, Key::S, Action::Save);
            key(Modifiers::COMMAND, Key::O, Action::OpenProject);
            key(Modifiers::COMMAND, Key::N, Action::NewProject);
            key(Modifiers::COMMAND, Key::I, Action::Import);
            key(Modifiers::NONE, Key::Home, Action::SeekStart);
            key(Modifiers::NONE, Key::End, Action::SeekEnd);
            key(Modifiers::NONE, Key::ArrowLeft, Action::Step(-1));
            key(Modifiers::NONE, Key::ArrowRight, Action::Step(1));
            key(Modifiers::NONE, Key::S, Action::Split);
            key(Modifiers::NONE, Key::Delete, Action::Delete);
            key(Modifiers::NONE, Key::Backspace, Action::Delete);
        });
        self.queue.extend(actions);
    }

    fn handle_drops(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if !dropped.is_empty() {
            self.queue.push(Action::ImportPaths(dropped));
        }
    }

    fn handle_close(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) && self.editor.is_dirty() && !self.close_now {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.modal = Some(Modal::Unsaved(Then::Close));
        }
        if self.close_now {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let title = format!("{}{} — SimpleCut", self.editor.project_name(), if self.editor.is_dirty() { " *" } else { "" });
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    fn modal_ui(&mut self, ctx: &egui::Context) {
        let Some(Modal::Unsaved(then)) = self.modal.clone() else { return };
        let mut answer: Option<&str> = None;
        egui::Window::new("Unsaved changes").collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.label(format!("Save changes to “{}” first?", self.editor.project_name()));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Save changes").clicked() {
                    answer = Some("save");
                }
                if ui.button("Discard changes").clicked() {
                    answer = Some("discard");
                }
                if ui.button("Cancel").clicked() {
                    answer = Some("cancel");
                }
            });
        });
        match answer {
            Some("save") => {
                self.modal = None;
                if self.save_project() {
                    self.run_then(then);
                }
            }
            Some("discard") => {
                self.modal = None;
                self.run_then(then);
            }
            Some(_) => self.modal = None,
            None => {}
        }
    }

    fn frame(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.handle_close(&ctx);
        self.handle_drops(&ctx);
        self.pump(&ctx);
        if self.modal.is_none() {
            self.handle_keys(&ctx);
        }

        egui::Panel::top("toolbar").show(ui, |ui| toolbar::show(self, ui));
        egui::Panel::bottom("status").show_separator_line(false).exact_size(28.0).show(ui, |ui| crate::status::show(self, ui));
        egui::Panel::bottom("timeline").resizable(true).default_size(250.0).min_size(170.0).show(ui, |ui| timeline::show(self, ui));
        egui::Panel::left("library").resizable(true).default_size(330.0).min_size(240.0).show(ui, |ui| library::show(self, ui));
        egui::CentralPanel::default().show(ui, |ui| preview::show(self, ui));

        self.modal_ui(&ctx);
        self.update_title(&ctx);

        let actions: Vec<Action> = std::mem::take(&mut self.queue);
        for a in actions {
            self.perform(a);
        }
    }
}

impl eframe::App for SimpleCutApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // a bug in one panel must not close the window and lose the user's work
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.frame(ui))).is_err() {
            log::error!("the interface hit an internal error and carried on");
            self.banner = Some(Banner { text: "Something went wrong in the interface. Your project is safe; try again.".into(), error: true });
            self.queue.clear();
        }
    }
}

pub(crate) fn to_image(f: &PreviewFrame) -> egui::ColorImage {
    // egui panics when the bytes do not match the size, so a damaged frame becomes black
    if f.width > 0 && f.height > 0 && f.rgba.len() == f.width * f.height * 4 {
        egui::ColorImage::from_rgba_unmultiplied([f.width, f.height], &f.rgba)
    } else {
        egui::ColorImage::from_rgba_unmultiplied([2, 2], &[0, 0, 0, 255].repeat(4))
    }
}
