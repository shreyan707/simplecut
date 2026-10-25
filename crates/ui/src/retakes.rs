//! Remove Retakes: start an analysis, review what it found, apply the cuts you approve.
//!
//! The editor does not know how detection works. An [`Analyzer`] gives back a document in the
//! retake contract, which is read and checked like a file from disk. Nothing is cut until the user
//! approves it, and applying is one undo step.

use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;

use egui::{Color32, CornerRadius, RichText, WidgetInfo, WidgetType};
use simplecut_core::retake::{self, Analysis, CutProposal, CutScope, Decision, MAX_DOCUMENT_BYTES, Presence};
use simplecut_core::{AnalysisJob, AnalysisRequest, Analyzer, Editor, ItemId, JobState, Tick};

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
    pub seen_on: HashSet<(ItemId, String)>,
    pub applied: HashSet<(ItemId, String)>,
}

impl Review {
    fn new(analyses: Vec<Analysis>, editor: &Editor) -> Self {
        let mut seen_on = HashSet::new();
        for a in &analyses {
            for p in &a.proposals {
                if editor.presence(a.item, p) == Presence::On {
                    seen_on.insert((a.item, p.id.clone()));
                }
            }
        }
        let mut r = Self { analyses, approved: HashSet::new(), seen_on, applied: HashSet::new() };
        r.approve_sure(editor);
        r
    }

    fn approve_sure(&mut self, editor: &Editor) {
        for a in &self.analyses {
            for p in &a.proposals {
                if editor.presence(a.item, p) == Presence::On && p.decision == Decision::Remove && p.confidence >= AUTO_APPROVE {
                    self.approved.insert((a.item, p.id.clone()));
                }
            }
        }
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

    /// (suggestions, approved) of the open review, counting only suggestions currently On the timeline.
    pub fn review_summary(&self) -> Option<(usize, usize)> {
        let r = self.retakes.review.as_ref()?;
        let mut total = 0;
        let mut approved = 0;
        for a in &r.analyses {
            for p in &a.proposals {
                if self.editor.presence(a.item, p) == Presence::On {
                    total += 1;
                    if r.approved.contains(&(a.item, p.id.clone())) {
                        approved += 1;
                    }
                }
            }
        }
        Some((total, approved))
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

    /// Where the suggestions are on the timeline: (start, end, approved, color_index), for drawing.
    pub(crate) fn review_ranges(&self) -> Vec<(Tick, Tick, bool, usize)> {
        let Some(r) = &self.retakes.review else { return Vec::new() };
        let mut out = Vec::new();
        let mut offset = 0;
        for a in &r.analyses {
            let grps = retake::groups_with_offset(a, offset);
            offset += grps.len();
            let mut prop_color = vec![0; a.proposals.len()];
            for g in &grps {
                for &idx in &g.members {
                    prop_color[idx] = g.color;
                }
            }
            for (idx, p) in a.proposals.iter().enumerate() {
                if self.editor.presence(a.item, p) == Presence::On {
                    let approved = r.approved.contains(&(a.item, p.id.clone()));
                    let color = prop_color[idx];
                    out.extend(self.editor.timeline_ranges(a.item, p).into_iter().map(|(s, e)| (s, e, approved, color)));
                }
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
            .map(|(s, e, _, _)| egui::Rect::from_min_max(egui::pos2(g.axis.x_of(s), lane.top()), egui::pos2(g.axis.x_of(e), lane.bottom())))
            .collect()
    }

    /// Colour indexes of the suggestion markers on the timeline in the same order as [`review_marker_rects`].
    pub fn review_marker_colors(&self) -> Vec<usize> {
        self.review_ranges().into_iter().map(|(_, _, _, c)| c).collect()
    }

    /// Groups currently visible in the live review list: (label, colour_index, [suggestion id]).
    /// Groups with only Off suggestions are omitted.
    pub fn review_groups(&self) -> Vec<(String, usize, Vec<String>)> {
        let Some(r) = &self.retakes.review else { return Vec::new() };
        let mut out = Vec::new();
        let mut offset = 0;
        for a in &r.analyses {
            let grps = retake::groups_with_offset(a, offset);
            offset += grps.len();
            for g in grps {
                let on_ids: Vec<String> =
                    g.members.iter().map(|&idx| &a.proposals[idx]).filter(|p| self.editor.presence(a.item, p) == Presence::On).map(|p| p.id.clone()).collect();
                if !on_ids.is_empty() {
                    out.push((g.label, g.color, on_ids));
                }
            }
        }
        out
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
            Action::ToggleGroup { item, ids, approve } => {
                if let Some(r) = &mut self.retakes.review {
                    for id in ids {
                        if approve {
                            r.approved.insert((item, id));
                        } else {
                            r.approved.remove(&(item, id));
                        }
                    }
                }
            }
            Action::ApproveSure => {
                if let Some(r) = &mut self.retakes.review {
                    r.approve_sure(&self.editor);
                }
            }
            Action::ApproveNone => {
                if let Some(r) = &mut self.retakes.review {
                    for a in &r.analyses {
                        for p in &a.proposals {
                            if self.editor.presence(a.item, p) == Presence::On {
                                r.approved.remove(&(a.item, p.id.clone()));
                            }
                        }
                    }
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
        let prev_applied = self.retakes.review.as_ref().map(|r| r.applied.clone()).unwrap_or_default();
        let mut already_removed = 0;
        for a in &analyses {
            for p in &a.proposals {
                if self.editor.presence(a.item, p) == Presence::Off {
                    let is_removed = prev_applied.contains(&(a.item, p.id.clone())) || !self.editor.is_outside_used(a.item, p);
                    if is_removed {
                        already_removed += 1;
                    }
                }
            }
        }
        let mut review = Review::new(analyses, &self.editor);
        if let Some(prev) = &self.retakes.review {
            for key in &prev.seen_on {
                review.seen_on.insert(key.clone());
            }
            for key in &prev.applied {
                review.applied.insert(key.clone());
            }
        }
        let approved = review.approved.len();
        self.retakes.review = Some(review);
        let extra = if already_removed > 0 { format!(" ({already_removed} already removed)") } else { String::new() };
        self.info_text(format!("Found {found} possible retake{}{extra}; {approved} selected. Review them, then remove.", if found == 1 { "" } else { "s" }));
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
            .map(|a| {
                (
                    a.item,
                    a.proposals
                        .iter()
                        .filter(|p| self.editor.presence(a.item, p) == Presence::On && review.approved.contains(&(a.item, p.id.clone())))
                        .cloned()
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        let applied_keys: Vec<(ItemId, String)> = sets.iter().flat_map(|(item, ps)| ps.iter().map(|p| (*item, p.id.clone()))).collect();
        match self.editor.apply_cut_sets(&sets, CutScope::MainTracks) {
            Ok(report) => {
                if let Some(r) = &mut self.retakes.review {
                    for key in applied_keys {
                        r.applied.insert(key);
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

struct GroupView {
    item: ItemId,
    key: String,
    label: String,
    color: Color32,
    members: Vec<Row>,
    kept: Option<String>,
    start: Tick,
    end: Tick,
}

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
    is_removed: bool,
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

    if let Some(r) = &mut app.retakes.review {
        for a in &r.analyses {
            for p in &a.proposals {
                if app.editor.presence(a.item, p) == Presence::On {
                    r.seen_on.insert((a.item, p.id.clone()));
                }
            }
        }
    }

    let fps = app.editor.sequence().map_or(30.0, |s| s.fps);
    let many = app.retakes.review.as_ref().is_some_and(|r| r.analyses.len() > 1);
    let (live_groups, off_rows, ignored): (Vec<GroupView>, Vec<Row>, Vec<(String, String)>) = match &app.retakes.review {
        Some(r) => {
            let mut live_groups = Vec::new();
            let mut off_rows = Vec::new();
            let mut offset = 0;
            for a in &r.analyses {
                let file = many.then(|| app.editor.media_entry(a.item).map(|m| m.name)).flatten();
                let grps = retake::groups_with_offset(a, offset);
                offset += grps.len();
                for g in grps {
                    let mut group_on = Vec::new();
                    let kept = g.members.iter().map(|&idx| &a.proposals[idx].kept_text).find(|t| !t.trim().is_empty()).cloned();
                    for &idx in &g.members {
                        let p = &a.proposals[idx];
                        let on = app.editor.presence(a.item, p) == Presence::On;
                        let is_removed = r.applied.contains(&(a.item, p.id.clone())) || !app.editor.is_outside_used(a.item, p);
                        let row = Row {
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
                            is_removed,
                            file: file.clone(),
                        };
                        if on {
                            group_on.push(row);
                        } else {
                            off_rows.push(row);
                        }
                    }
                    if !group_on.is_empty() {
                        let start = group_on.iter().map(|m| m.start).min().unwrap_or(Tick::ZERO);
                        let end = group_on.iter().map(|m| m.end).max().unwrap_or(Tick::ZERO);
                        let color = theme::GROUP_COLORS[g.color % theme::GROUP_COLORS.len()];
                        live_groups.push(GroupView { item: a.item, key: g.key, label: g.label, color, members: group_on, kept, start, end });
                    }
                }
            }
            (live_groups, off_rows, r.rejected())
        }
        None => return,
    };

    let total: usize = live_groups.iter().map(|g| g.members.len()).sum();
    let approved: usize = live_groups.iter().flat_map(|g| &g.members).filter(|m| m.approved).count();

    if total == 0 {
        ui.label(RichText::new("Everything in this list is already removed. Ctrl+Z brings it back.").color(t.text_dim));
    } else {
        ui.label(RichText::new(format!("{total} possible retake{} · {approved} selected", if total == 1 { "" } else { "s" })).color(t.text_dim));
        ui.horizontal_wrapped(|ui| {
            if ui.small_button("Select sure ones").on_hover_text("Select what the detector is at least 80 % sure about").clicked() {
                app.queue(Action::ApproveSure);
            }
            if ui.small_button("Select none").clicked() {
                app.queue(Action::ApproveNone);
            }
        });
    }
    ui.add_space(4.0);

    let list_height = (ui.available_height() - 64.0).max(80.0);
    egui::ScrollArea::vertical().max_height(list_height).auto_shrink([false, false]).show(ui, |ui| {
        for group in &live_groups {
            let id = ui.make_persistent_id(format!("retake_group_{}_{}", group.item.0, group.key));
            let state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true);

            let count = group.members.len();
            let on_member_ids: Vec<String> = group.members.iter().map(|m| m.id.clone()).collect();
            let all_approved = group.members.iter().all(|m| m.approved);
            let some_approved = group.members.iter().any(|m| m.approved);
            let indeterminate = !all_approved && some_approved;
            let group_label = group.label.clone();
            let group_key = group.key.clone();
            let group_color = group.color;
            let item = group.item;

            let mut toggle_group_action = None;
            let mut title_clicked = false;

            let mut res = state.show_header(ui, |ui| {
                ui.horizontal(|ui| {
                    let (dot_rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                    ui.painter().circle_filled(dot_rect.center(), 4.0, group_color);

                    let title_resp =
                        ui.add(egui::Label::new(RichText::new(&group_label).strong().color(t.text)).sense(egui::Sense::click())).on_hover_text(&group_key);
                    if title_resp.clicked() {
                        title_clicked = true;
                    }

                    let count_text = format!("({} suggestion{})", count, if count == 1 { "" } else { "s" });
                    ui.label(RichText::new(count_text).color(t.text_dim));

                    let span_text = format!("{} – {}", format::timecode(group.start, fps), format::timecode(group.end, fps));
                    ui.label(RichText::new(span_text).color(t.text_dim));

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(20.0);
                        let mut checked = all_approved;
                        let cb = ui.add(egui::Checkbox::without_text(&mut checked).indeterminate(indeterminate));
                        let acc_label = format!("Select group {group_label}");
                        cb.widget_info(move || {
                            if indeterminate {
                                WidgetInfo::labeled(WidgetType::Checkbox, true, acc_label.clone())
                            } else {
                                WidgetInfo::selected(WidgetType::Checkbox, true, checked, acc_label.clone())
                            }
                        });
                        if cb.clicked() {
                            toggle_group_action = Some(Action::ToggleGroup { item, ids: on_member_ids.clone(), approve: !all_approved });
                        }
                    });
                });
            });

            if title_clicked {
                res.toggle();
            }
            if let Some(act) = toggle_group_action {
                app.queue(act);
            }

            let (toggle_btn, _, _) = res.body(|ui| {
                if let Some(kept) = &group.kept {
                    ui.add_space(2.0);
                    ui.label(RichText::new(format!("Keeps: “{}”", shorten(kept, 120))).color(t.text));
                    ui.add_space(2.0);
                }

                for row in &group.members {
                    let frame_resp = egui::Frame::new().fill(t.panel_raised).corner_radius(CornerRadius::same(6)).inner_margin(8).show(ui, |ui| {
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
                        let go = ui.small_button("Go to").on_hover_text("Move the playhead here");
                        go.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, format!("Go to {}", row.id)));
                        if go.clicked() {
                            app.queue(Action::GoToProposal(row.item, row.id.clone()));
                        }
                    });

                    let r = frame_resp.response.rect;
                    let stripe = egui::Rect::from_min_max(r.left_top(), egui::pos2(r.left() + 4.0, r.bottom()));
                    ui.painter().rect_filled(stripe, CornerRadius { nw: 6, sw: 6, ne: 0, se: 0 }, group_color);

                    ui.add_space(4.0);
                }
            });
            toggle_btn.widget_info(|| WidgetInfo::labeled(WidgetType::CollapsingHeader, true, format!("Group {group_label}")));
            ui.add_space(6.0);
        }

        if !off_rows.is_empty() {
            ui.add_space(4.0);
            egui::CollapsingHeader::new(RichText::new(format!("Already removed or not on the timeline ({})", off_rows.len())).color(t.text_dim))
                .default_open(false)
                .show(ui, |ui| {
                    for row in &off_rows {
                        egui::Frame::new().fill(t.panel_raised).corner_radius(CornerRadius::same(6)).inner_margin(8).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                let mut checked = row.approved;
                                let label = format!("{} – {}", format::timecode(row.start, fps), format::timecode(row.end, fps));
                                let r = ui.add_enabled(false, egui::Checkbox::new(&mut checked, RichText::new(label).color(t.text_dim)));
                                r.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, false, row.approved, format!("Select suggestion {}", row.id)));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.label(RichText::new(format!("{:.0}%", row.confidence * 100.0)).color(t.text_dim));
                                });
                            });
                            let status = if row.is_removed { "Already removed" } else { "Outside the part of the video you used" };
                            ui.label(RichText::new(status).color(t.text_dim).small());
                            if let Some(file) = &row.file {
                                ui.label(RichText::new(file).color(t.text_dim).small());
                            }
                            if !row.reason.is_empty() {
                                ui.label(RichText::new(&row.reason).color(t.text_dim).small());
                            }
                            if !row.removed.is_empty() {
                                ui.label(RichText::new(format!("Cut: “{}”", shorten(&row.removed, 120))).color(t.text_dim).small());
                            }
                            if !row.kept.is_empty() {
                                ui.label(RichText::new(format!("Keeps: “{}”", shorten(&row.kept, 120))).color(t.text_dim).small());
                            }
                        });
                        ui.add_space(4.0);
                    }
                });
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
