//! Remove Retakes: start an analysis, review what it found, apply the cuts you approve.
//!
//! The editor does not know how detection works. An [`Analyzer`] gives back a document in the
//! retake contract, which is read and checked like a file from disk. Nothing is cut until the user
//! approves it, and applying is one undo step.

use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;

use egui::{Color32, CornerRadius, RichText, WidgetInfo, WidgetType};
use simplecut_core::retake::{self, Analysis, CutProposal, CutScope, Decision, MAX_DOCUMENT_BYTES};
use simplecut_core::{AnalysisJob, AnalysisRequest, Analyzer, ItemId, JobState, Tick};

use crate::app::{Action, SimpleCutApp, Source};
use crate::{format, theme};

/// Proposals the detector is this sure of (and recommends removing) start out approved.
pub const AUTO_APPROVE: f32 = 0.8;

pub(crate) struct Running {
    job: Box<dyn AnalysisJob>,
    queue: VecDeque<AnalysisRequest>,
    current: AnalysisRequest,
    done: Vec<Analysis>,
    pub status: String,
    pub fraction: f32,
}

pub(crate) struct Review {
    pub analyses: Vec<Analysis>,
    pub approved: HashSet<(ItemId, String)>,
}

impl Review {
    fn new(analyses: Vec<Analysis>) -> Self {
        let mut r = Self { analyses, approved: HashSet::new() };
        r.approve_sure();
        r
    }

    fn approve_sure(&mut self) {
        self.approved = self
            .analyses
            .iter()
            .flat_map(|a| a.proposals.iter().map(move |p| (a.item, p)))
            .filter(|(_, p)| p.decision == Decision::Remove && p.confidence >= AUTO_APPROVE)
            .map(|(item, p)| (item, p.id.clone()))
            .collect();
    }

    fn total(&self) -> usize {
        self.analyses.iter().map(|a| a.proposals.len()).sum()
    }

    fn rejected(&self) -> Vec<(String, String)> {
        self.analyses.iter().flat_map(|a| a.rejected.iter().map(|r| (r.id.clone(), r.reason.clone()))).collect()
    }

    fn find(&self, item: ItemId, id: &str) -> Option<&CutProposal> {
        self.analyses.iter().find(|a| a.item == item)?.proposals.iter().find(|p| p.id == id)
    }
}

#[derive(Default)]
pub(crate) struct RetakeUi {
    pub running: Option<Running>,
    pub review: Option<Review>,
}

impl RetakeUi {
    pub fn visible(&self) -> bool {
        self.running.is_some() || self.review.is_some()
    }
}

impl SimpleCutApp {
    /// Connect something that can find retakes. Until then the "Remove Retakes" button is hidden.
    pub fn set_analyzer(&mut self, analyzer: Option<Box<dyn Analyzer>>) {
        self.analyzer = analyzer;
    }

    pub fn has_analyzer(&self) -> bool {
        self.analyzer.is_some()
    }

    pub fn is_analyzing(&self) -> bool {
        self.retakes.running.is_some()
    }

    /// (suggestions, approved) of the open review.
    pub fn review_summary(&self) -> Option<(usize, usize)> {
        self.retakes.review.as_ref().map(|r| (r.total(), r.approved.len()))
    }

    /// Ids of the suggestions in the open review, in order.
    pub fn review_ids(&self) -> Vec<String> {
        self.retakes.review.iter().flat_map(|r| r.analyses.iter().flat_map(|a| a.proposals.iter().map(|p| p.id.clone()))).collect()
    }

    pub fn review_is_approved(&self, id: &str) -> bool {
        self.retakes.review.as_ref().is_some_and(|r| r.approved.iter().any(|(_, i)| i == id))
    }

    /// How many suggestions were ignored as unusable.
    pub fn review_ignored(&self) -> usize {
        self.retakes.review.as_ref().map_or(0, |r| r.rejected().len())
    }

    /// Where the suggestions are on the timeline: (start, end, approved), for drawing.
    pub(crate) fn review_ranges(&self) -> Vec<(Tick, Tick, bool)> {
        let Some(r) = &self.retakes.review else { return Vec::new() };
        let mut out = Vec::new();
        for a in &r.analyses {
            for p in &a.proposals {
                let approved = r.approved.contains(&(a.item, p.id.clone()));
                out.extend(self.editor.timeline_ranges(a.item, p).into_iter().map(|(s, e)| (s, e, approved)));
            }
        }
        out
    }

    /// Screen rectangles of the suggestion markers on the video lane as last drawn.
    pub fn review_marker_rects(&self) -> Vec<egui::Rect> {
        let Some(g) = self.tl.last.as_ref() else { return Vec::new() };
        let Some(lane) = g.lanes.first() else { return Vec::new() };
        self.review_ranges()
            .into_iter()
            .map(|(s, e, _)| egui::Rect::from_min_max(egui::pos2(g.axis.x_of(s), lane.top()), egui::pos2(g.axis.x_of(e), lane.bottom())))
            .collect()
    }

    pub(crate) fn perform_retake(&mut self, action: Action) {
        match action {
            Action::StartRetakeAnalysis => self.start_analysis(),
            Action::CancelAnalysis => {
                if let Some(r) = &mut self.retakes.running {
                    r.job.cancel();
                }
            }
            Action::OpenAnalysisFile => {
                if let Some(path) = self.dialogs.pick_analysis_file() {
                    self.load_analysis(path);
                }
            }
            Action::LoadAnalysis(path) => self.load_analysis(path),
            Action::ToggleProposal(item, id) => {
                if let Some(r) = &mut self.retakes.review {
                    let key = (item, id);
                    if !r.approved.remove(&key) {
                        r.approved.insert(key);
                    }
                }
            }
            Action::ApproveSure => {
                if let Some(r) = &mut self.retakes.review {
                    r.approve_sure();
                }
            }
            Action::ApproveNone => {
                if let Some(r) = &mut self.retakes.review {
                    r.approved.clear();
                }
            }
            Action::GoToProposal(item, id) => self.go_to_proposal(item, &id),
            Action::ApplyRetakes => self.apply_retakes(),
            Action::CloseReview => self.retakes.review = None,
            _ => {}
        }
    }

    fn start_analysis(&mut self) {
        if self.retakes.running.is_some() {
            return;
        }
        let Some(analyzer) = &self.analyzer else {
            self.fail_text("No retake detector is connected.");
            return;
        };
        let mut queue: VecDeque<AnalysisRequest> = match self.editor.analysis_requests() {
            Ok(r) => r.into(),
            Err(e) => {
                self.fail_text(&e.to_string());
                return;
            }
        };
        let Some(current) = queue.pop_front() else { return };
        let job = analyzer.start(current.clone());
        self.player.pause();
        self.retakes.running = Some(Running { job, queue, current, done: Vec::new(), status: "Starting…".into(), fraction: 0.0 });
    }

    /// Called once per frame: follow the running analysis.
    pub(crate) fn pump_retakes(&mut self) {
        let Some(mut run) = self.retakes.running.take() else { return };
        match run.job.poll() {
            JobState::Running { fraction, status } => {
                run.fraction = fraction;
                run.status = status;
                self.retakes.running = Some(run);
            }
            JobState::Cancelled => self.info_text("Analysis cancelled."),
            JobState::Failed(why) => self.fail_text(&format!("The retake detector could not finish: {why}")),
            JobState::Done(json) => match self.editor.read_analysis(&json, run.current.item) {
                Err(e) => self.fail_text(&e.to_string()),
                Ok(analysis) => {
                    run.done.push(analysis);
                    match run.queue.pop_front() {
                        Some(next) => {
                            if let Some(analyzer) = &self.analyzer {
                                run.job = analyzer.start(next.clone());
                                run.current = next;
                                run.fraction = 0.0;
                                self.retakes.running = Some(run);
                            }
                        }
                        None => self.finish_analysis(run.done),
                    }
                }
            },
        }
    }

    fn finish_analysis(&mut self, analyses: Vec<Analysis>) {
        let found: usize = analyses.iter().map(|a| a.proposals.len()).sum();
        let ignored: usize = analyses.iter().map(|a| a.rejected.len()).sum();
        if found == 0 {
            let extra = if ignored > 0 { format!(" ({ignored} unusable suggestions were ignored)") } else { String::new() };
            self.info_text(format!("No retakes found.{extra}"));
            return;
        }
        let review = Review::new(analyses);
        let approved = review.approved.len();
        self.retakes.review = Some(review);
        self.info_text(format!("Found {found} possible retake{}; {approved} selected. Review them, then remove.", if found == 1 { "" } else { "s" }));
    }

    fn load_analysis(&mut self, path: PathBuf) {
        let text = match std::fs::metadata(&path) {
            Ok(m) if m.len() as usize > MAX_DOCUMENT_BYTES => Err("That file is too large to be a retake analysis.".to_string()),
            Ok(_) => std::fs::read_to_string(&path).map_err(|e| format!("Could not read {}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(format!("The file could not be found: {}", path.display())),
            Err(e) => Err(format!("Could not read {}: {e}", path.display())),
        };
        let result = text.and_then(|t| {
            let doc = retake::parse(&t).map_err(|e| e.to_string())?;
            let item = self.editor.find_media_for(&doc).map_err(|e| e.to_string())?;
            let name = self.editor.media_entry(item).map(|m| m.name).unwrap_or_default();
            self.editor.read_analysis(&t, item).map(|a| (a, name)).map_err(|e| e.to_string())
        });
        match result {
            Ok((analysis, name)) => {
                self.player.pause();
                self.finish_analysis(vec![analysis]);
                let _ = name;
            }
            Err(msg) => self.fail_text(&msg),
        }
    }

    fn go_to_proposal(&mut self, item: ItemId, id: &str) {
        let Some(p) = self.retakes.review.as_ref().and_then(|r| r.find(item, id)).cloned() else { return };
        match self.editor.timeline_ranges(item, &p).first() {
            Some((start, _)) => self.queue(Action::SeekTimeline(*start)),
            None => self.info_text("That part of the video is not on the timeline."),
        }
    }

    fn apply_retakes(&mut self) {
        let Some(review) = &self.retakes.review else { return };
        let sets: Vec<(ItemId, Vec<CutProposal>)> = review
            .analyses
            .iter()
            .map(|a| (a.item, a.proposals.iter().filter(|p| review.approved.contains(&(a.item, p.id.clone()))).cloned().collect::<Vec<_>>()))
            .collect();
        let applied: HashSet<(ItemId, String)> = review.approved.clone();
        match self.editor.apply_cut_sets(&sets, CutScope::MainTracks) {
            Ok(report) => {
                if let Some(r) = &mut self.retakes.review {
                    for a in &mut r.analyses {
                        let item = a.item;
                        a.proposals.retain(|p| !applied.contains(&(item, p.id.clone())));
                    }
                    r.approved.clear();
                    if r.total() == 0 {
                        self.retakes.review = None;
                    }
                }
                self.source = Source::Timeline;
                let secs = report.removed.0 as f64 / 254_016_000_000.0;
                let mut text = format!("Removed {} retake{} ({secs:.1} s). Ctrl+Z puts them back.", report.cuts, if report.cuts == 1 { "" } else { "s" });
                if let Some(n) = report.notes.first() {
                    text.push(' ');
                    text.push_str(n);
                }
                self.info_text(text);
            }
            Err(e) => self.fail_text(&e.to_string()),
        }
    }
}

// ── the review column ──────────────────────────────────────────────────────

struct Row {
    item: ItemId,
    id: String,
    start: Tick,
    end: Tick,
    confidence: f32,
    decision: Decision,
    reason: String,
    removed: String,
    kept: String,
    approved: bool,
    file: Option<String>,
}

fn shorten(text: &str, max: usize) -> String {
    let t = text.trim();
    if t.chars().count() <= max { t.to_string() } else { format!("{}…", t.chars().take(max).collect::<String>().trim_end()) }
}

pub(crate) fn panel(app: &mut SimpleCutApp, ui: &mut egui::Ui) {
    let t = theme::tokens();
    ui.horizontal(|ui| {
        ui.heading("Retakes");
        if app.retakes.review.is_some() {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Close").on_hover_text("Hide this list without cutting anything").clicked() {
                    app.queue(Action::CloseReview);
                }
            });
        }
    });
    ui.separator();

    if let Some(run) = &app.retakes.running {
        ui.label(RichText::new(&run.status).color(t.text));
        ui.add(egui::ProgressBar::new(run.fraction.clamp(0.0, 1.0)).desired_width(ui.available_width()));
        ui.add_space(6.0);
        if ui.button("Cancel analysis").clicked() {
            app.queue(Action::CancelAnalysis);
        }
        return;
    }
    let fps = app.editor.sequence().map_or(30.0, |s| s.fps);
    let many = app.retakes.review.as_ref().is_some_and(|r| r.analyses.len() > 1);
    let (rows, ignored, total, approved): (Vec<Row>, Vec<(String, String)>, usize, usize) = match &app.retakes.review {
        Some(r) => {
            let mut rows = Vec::new();
            for a in &r.analyses {
                let file = many.then(|| app.editor.media_entry(a.item).map(|m| m.name)).flatten();
                for p in &a.proposals {
                    rows.push(Row {
                        item: a.item,
                        id: p.id.clone(),
                        start: p.start,
                        end: p.end,
                        confidence: p.confidence,
                        decision: p.decision,
                        reason: p.reason.clone(),
                        removed: p.removed_text.clone(),
                        kept: p.kept_text.clone(),
                        approved: r.approved.contains(&(a.item, p.id.clone())),
                        file: file.clone(),
                    });
                }
            }
            (rows, r.rejected(), r.total(), r.approved.len())
        }
        None => return,
    };

    ui.label(RichText::new(format!("{total} possible retake{} · {approved} selected", if total == 1 { "" } else { "s" })).color(t.text_dim));
    ui.horizontal_wrapped(|ui| {
        if ui.small_button("Select sure ones").on_hover_text("Select what the detector is at least 80 % sure about").clicked() {
            app.queue(Action::ApproveSure);
        }
        if ui.small_button("Select none").clicked() {
            app.queue(Action::ApproveNone);
        }
    });
    ui.add_space(4.0);

    let list_height = (ui.available_height() - 64.0).max(80.0);
    egui::ScrollArea::vertical().max_height(list_height).auto_shrink([false, false]).show(ui, |ui| {
        for row in &rows {
            egui::Frame::new().fill(t.panel_raised).corner_radius(CornerRadius::same(6)).inner_margin(8).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let mut checked = row.approved;
                    let label = format!("{} – {}", format::timecode(row.start, fps), format::timecode(row.end, fps));
                    let r = ui.checkbox(&mut checked, label);
                    r.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, true, row.approved, format!("Select suggestion {}", row.id)));
                    if r.clicked() {
                        app.queue(Action::ToggleProposal(row.item, row.id.clone()));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let sure = if row.confidence >= AUTO_APPROVE {
                            t.accent
                        } else if row.confidence >= 0.5 {
                            t.text
                        } else {
                            t.text_dim
                        };
                        ui.label(RichText::new(format!("{:.0}%", row.confidence * 100.0)).color(sure).strong());
                        if row.decision == Decision::Review {
                            ui.label(RichText::new("REVIEW").color(Color32::from_rgb(0xf0, 0xa8, 0x40)).small());
                        }
                    });
                });
                if let Some(file) = &row.file {
                    ui.label(RichText::new(file).color(t.text_dim).small());
                }
                if !row.reason.is_empty() {
                    ui.label(RichText::new(&row.reason).color(t.text_dim));
                }
                if !row.removed.is_empty() {
                    ui.label(RichText::new(format!("Cut: “{}”", shorten(&row.removed, 120))).color(t.danger));
                }
                if !row.kept.is_empty() {
                    ui.label(RichText::new(format!("Keeps: “{}”", shorten(&row.kept, 120))).color(t.text));
                }
                let go = ui.small_button("Go to").on_hover_text("Move the playhead here");
                go.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, format!("Go to {}", row.id)));
                if go.clicked() {
                    app.queue(Action::GoToProposal(row.item, row.id.clone()));
                }
            });
            ui.add_space(4.0);
        }
        if !ignored.is_empty() {
            ui.collapsing(format!("{} unusable suggestion{} ignored", ignored.len(), if ignored.len() == 1 { "" } else { "s" }), |ui| {
                for (id, why) in &ignored {
                    ui.label(RichText::new(format!("{id}: {why}")).color(t.text_dim).small());
                }
            });
        }
    });

    ui.add_space(6.0);
    let label = format!("Remove {approved} retake{}", if approved == 1 { "" } else { "s" });
    if ui
        .add_enabled(approved > 0, egui::Button::new(RichText::new(label).strong()).min_size(egui::vec2(ui.available_width(), 30.0)))
        .on_hover_text("Cut the selected ones out of the video (one undo step)")
        .clicked()
    {
        app.queue(Action::ApplyRetakes);
    }
}
