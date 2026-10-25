//! The seam between the editor and whatever finds retakes.
//!
//! The editor never knows how detection works. It hands an [`Analyzer`] the file to look at and
//! gets back a document in the retake contract (`docs/retake-contract.md`) as text; the editor then
//! reads and checks it exactly as it would a file from disk. So a real pipeline (a local service),
//! a file someone made by hand, and the [`FakeAnalyzer`] below all go through the same checks.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use filmcraft_project::TrackKind;
use filmcraft_time::Tick;

use crate::editor::{Editor, ItemId};
use crate::error::{EditorError, Result};

/// One file to analyse.
#[derive(Debug, Clone, PartialEq)]
pub struct AnalysisRequest {
    pub item: ItemId,
    /// The project's name for the file.
    pub name: String,
    /// Where the file is on disk.
    pub path: String,
    /// The file's length.
    pub duration: Tick,
    /// The parts of the file that are on the timeline, in file time (merged, sorted). A detector
    /// may skip the rest.
    pub used: Vec<(Tick, Tick)>,
}

/// How an analysis is going.
#[derive(Debug, Clone, PartialEq)]
pub enum JobState {
    Running {
        fraction: f32,
        status: String,
    },
    /// Finished: the document, as text, in the retake contract.
    Done(String),
    Failed(String),
    Cancelled,
}

/// A running analysis. Polled from the interface once per frame, so it must not block.
pub trait AnalysisJob: Send {
    fn poll(&mut self) -> JobState;
    fn cancel(&mut self);
}

/// Something that can find retakes in a file.
pub trait Analyzer: Send + Sync {
    /// A short name for messages ("Retake detector").
    fn name(&self) -> String;
    /// Start looking. Returns at once; the work happens in the background.
    fn start(&self, request: AnalysisRequest) -> Box<dyn AnalysisJob>;
}

impl Editor {
    /// What to analyse for the video on the timeline: each file used on the main video track, once,
    /// with the parts of it that are used. Offline files are an error (there is nothing to read).
    pub fn analysis_requests(&self) -> Result<Vec<AnalysisRequest>> {
        let q = self.session().active_sequence().ok_or(EditorError::NoSequence)?;
        let track = q.video_tracks.first().ok_or(EditorError::NoSequence)?;
        debug_assert_eq!(track.kind, TrackKind::Video);
        let mut requests: Vec<AnalysisRequest> = Vec::new();
        for clip in &track.items {
            let used = (clip.source_in, clip.source_in + clip.duration);
            match requests.iter_mut().find(|r| r.item == clip.item) {
                Some(r) => r.used.push(used),
                None => {
                    let m = self.media_entry(clip.item).ok_or(EditorError::NoSuchMedia)?;
                    if m.offline {
                        return Err(EditorError::FileMissing(m.offline_reason.unwrap_or(m.name)));
                    }
                    let Some(path) = m.path else { continue };
                    requests.push(AnalysisRequest { item: clip.item, name: m.name, path, duration: m.duration, used: vec![used] });
                }
            }
        }
        for r in &mut requests {
            r.used.sort();
            let mut merged: Vec<(Tick, Tick)> = Vec::new();
            for (a, b) in r.used.drain(..) {
                match merged.last_mut() {
                    Some(last) if a <= last.1 => last.1 = last.1.max(b),
                    _ => merged.push((a, b)),
                }
            }
            r.used = merged;
        }
        if requests.is_empty() {
            return Err(EditorError::NotAvailable("There is no video on the timeline to look at yet.".into()));
        }
        Ok(requests)
    }
}

// ── a stand-in detector ────────────────────────────────────────────────────

/// A detector that finds invented retakes, to build and test everything around it. It takes
/// `steps` steps of `step_delay` each, then returns three proposals spread over the file: a sure
/// one, an unsure one marked "review", and another sure one.
#[derive(Debug, Clone)]
pub struct FakeAnalyzer {
    pub steps: u32,
    pub step_delay: Duration,
    /// Finish with this error instead of a result.
    pub fail_with: Option<String>,
    /// Report a video length that does not match the file (to test the "different file" refusal).
    pub wrong_length: bool,
}

impl Default for FakeAnalyzer {
    fn default() -> Self {
        Self { steps: 4, step_delay: Duration::from_millis(40), fail_with: None, wrong_length: false }
    }
}

impl FakeAnalyzer {
    /// The document this fake returns for `request`.
    pub fn document_for(&self, request: &AnalysisRequest) -> String {
        let total = request.duration.0 as f64 / 254_016_000_000.0;
        let at = |f: f64| (total * f * 100.0).round() / 100.0;
        let claimed = if self.wrong_length { total + 30.0 } else { total };
        serde_json::json!({
            "schema": "simplecut.retakes/1",
            "media": { "path": request.path, "duration_seconds": claimed },
            "generator": { "name": "fake", "version": "0" },
            "proposals": [
                { "id": "fake_0:0", "start": at(0.10), "end": at(0.15), "confidence": 0.93, "decision": "remove",
                  "reason": "The sentence was restarted; the later take is complete.",
                  "removed_text": "So today we're going to, um, so today", "kept_text": "So today we are going to talk about editing.",
                  "kept_start": at(0.25), "kept_end": at(0.32), "source": "llm", "group": "fake_0" },
                { "id": "fake_0:1", "start": at(0.18), "end": at(0.22), "confidence": 0.88, "decision": "remove",
                  "reason": "Another false start of the same sentence.",
                  "removed_text": "So today we will, uh", "kept_text": "So today we are going to talk about editing.",
                  "kept_start": at(0.25), "kept_end": at(0.32), "source": "llm", "group": "fake_0" },
                { "id": "fake_1:0", "start": at(0.40), "end": at(0.46), "confidence": 0.55, "decision": "review",
                  "reason": "Similar wording, but it may be a deliberate repeat.",
                  "removed_text": "and that is the key idea", "kept_text": "and that is the key idea of the whole video", "source": "llm", "group": "fake_1" },
                { "id": "fake_2:0", "start": at(0.70), "end": at(0.72), "confidence": 0.85, "decision": "remove", "source": "stage2", "group": "fake_2" }
            ]
        })
        .to_string()
    }
}

impl Analyzer for FakeAnalyzer {
    fn name(&self) -> String {
        "Demo detector".into()
    }

    fn start(&self, request: AnalysisRequest) -> Box<dyn AnalysisJob> {
        let state = Arc::new(Mutex::new(JobState::Running { fraction: 0.0, status: "Starting…".into() }));
        let cancel = Arc::new(AtomicBool::new(false));
        let (shared, stop, me) = (state.clone(), cancel.clone(), self.clone());
        let steps = me.steps.max(1);
        let worker = std::thread::Builder::new().name("simplecut-fake-analysis".into()).spawn(move || {
            let set = |s: JobState| *shared.lock().unwrap_or_else(PoisonError::into_inner) = s;
            for i in 0..steps {
                std::thread::sleep(me.step_delay);
                if stop.load(Ordering::Relaxed) {
                    set(JobState::Cancelled);
                    return;
                }
                set(JobState::Running {
                    fraction: (i + 1) as f32 / steps as f32, status: format!("Listening to “{}” ({} of {steps})", request.name, i + 1)
                });
            }
            set(match &me.fail_with {
                Some(why) => JobState::Failed(why.clone()),
                None => JobState::Done(me.document_for(&request)),
            });
        });
        if worker.is_err() {
            *state.lock().unwrap_or_else(PoisonError::into_inner) = JobState::Failed("Could not start the analysis.".into());
        }
        Box::new(FakeJob { state, cancel })
    }
}

struct FakeJob {
    state: Arc<Mutex<JobState>>,
    cancel: Arc<AtomicBool>,
}

impl AnalysisJob for FakeJob {
    fn poll(&mut self) -> JobState {
        self.state.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn cancel(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
