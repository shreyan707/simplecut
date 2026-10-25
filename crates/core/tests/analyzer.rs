//! The detector interface (with the stand-in detector), what the editor asks a detector to look at,
//! and the mapping from file time to the timeline used by the review list.
//!
//! Skipped (with a message) when ffmpeg / ffprobe are not installed.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::time::{Duration, Instant};

use common::{have, make_clip, scratch};
use simplecut_core::retake::{CutProposal, CutScope, Decision};
use simplecut_core::{AnalysisJob, Analyzer, Edge, Editor, EditorError, FakeAnalyzer, ItemId, JobState, Lane, PlaceAt, Tick, TrimMode};

const TPS: f64 = 254_016_000_000.0;

fn secs(s: f64) -> Tick {
    Tick::from_seconds_f64(s)
}

fn s_of(t: Tick) -> f64 {
    t.0 as f64 / TPS
}

fn proposal(id: &str, start: f64, end: f64) -> CutProposal {
    CutProposal {
        id: id.into(),
        start: secs(start),
        end: secs(end),
        confidence: 0.9,
        decision: Decision::Remove,
        reason: String::new(),
        removed_text: String::new(),
        kept_text: String::new(),
        kept: None,
        source: None,
        group: None,
    }
}

fn project(names: &[&str]) -> Option<(Editor, Vec<ItemId>)> {
    if !have("ffmpeg") || !have("ffprobe") {
        eprintln!("SKIPPED: ffmpeg / ffprobe are not installed");
        return None;
    }
    let dir = scratch("analyzer");
    let mut e = Editor::new();
    e.new_project("T").unwrap();
    let mut items = Vec::new();
    for n in names {
        let id = e.import(&[make_clip(&dir, n, 10, true)]).unwrap().imported[0];
        e.place(id, Lane::Main, PlaceAt::End).unwrap();
        items.push(id);
    }
    e.select(&[]).unwrap();
    Some((e, items))
}

fn finish(job: &mut Box<dyn AnalysisJob>) -> (Vec<f32>, JobState) {
    let mut fractions = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match job.poll() {
            JobState::Running { fraction, .. } => fractions.push(fraction),
            other => return (fractions, other),
        }
        assert!(Instant::now() < deadline, "the analysis never finished");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn the_stand_in_detector_reports_progress_and_returns_a_document_the_editor_accepts() {
    let Some((e, items)) = project(&["a.mp4"]) else { return };
    let request = e.analysis_requests().unwrap().remove(0);
    assert_eq!(request.item, items[0]);
    let mut job = FakeAnalyzer::default().start(request);
    let (fractions, end) = finish(&mut job);
    assert!(fractions.windows(2).all(|w| w[0] <= w[1]), "progress only goes forward: {fractions:?}");
    let JobState::Done(json) = end else { panic!("not done: {end:?}") };
    let a = e.read_analysis(&json, items[0]).unwrap();
    assert_eq!(a.proposals.len(), 3);
    assert!(a.rejected.is_empty());
    assert_eq!(a.proposals[1].decision, Decision::Review);
}

#[test]
fn an_analysis_can_be_cancelled_can_fail_and_can_be_for_the_wrong_file() {
    let Some((e, items)) = project(&["a.mp4"]) else { return };
    let req = || e.analysis_requests().unwrap().remove(0);

    let mut job = FakeAnalyzer { steps: 50, step_delay: Duration::from_millis(30), ..Default::default() }.start(req());
    std::thread::sleep(Duration::from_millis(100));
    job.cancel();
    assert_eq!(finish(&mut job).1, JobState::Cancelled);

    let mut job = FakeAnalyzer { fail_with: Some("the model is not installed".into()), ..Default::default() }.start(req());
    assert_eq!(finish(&mut job).1, JobState::Failed("the model is not installed".into()));

    let mut job = FakeAnalyzer { wrong_length: true, ..Default::default() }.start(req());
    let JobState::Done(json) = finish(&mut job).1 else { panic!("not done") };
    assert!(matches!(e.read_analysis(&json, items[0]), Err(EditorError::Invalid(m)) if m.contains("different file")));
}

#[test]
fn the_request_lists_each_file_once_with_the_parts_that_are_used() {
    let Some((mut e, items)) = project(&["a.mp4"]) else { return };
    e.place(items[0], Lane::Main, PlaceAt::End).unwrap(); // the same file again
    let clip = e.sequence().unwrap().tracks[0].clips[0].id;
    e.trim(clip, Edge::Start, secs(2.0), TrimMode::Ripple).unwrap(); // the first use shows 2 s to 10 s
    let r = e.analysis_requests().unwrap();
    assert_eq!(r.len(), 1, "one file, though used twice");
    assert_eq!(r[0].name, "a.mp4");
    assert!(r[0].path.ends_with("a.mp4"));
    assert_eq!(r[0].used.len(), 1, "2-10 and 0-10 overlap, so they are one used part: {:?}", r[0].used);
    assert!(s_of(r[0].used[0].0).abs() < 0.04 && (s_of(r[0].used[0].1) - 10.0).abs() < 0.04);

    // nothing on the timeline to look at
    let mut empty = Editor::new();
    empty.new_project("E").unwrap();
    assert!(matches!(empty.analysis_requests(), Err(EditorError::NotAvailable(_))));
}

#[test]
fn a_file_that_cannot_be_read_is_not_sent_to_the_detector() {
    let Some((mut e, _)) = project(&["a.mp4"]) else { return };
    let dir = scratch("analyzer-offline");
    let file = dir.join("p.fcproj");
    e.save_as(&file).unwrap();
    let media = e.media()[0].path.clone().unwrap();
    std::fs::rename(&media, format!("{media}.moved")).unwrap();
    let mut again = Editor::new();
    again.open(&file).unwrap();
    assert!(matches!(again.analysis_requests(), Err(EditorError::FileMissing(_))));
}

#[test]
fn file_time_maps_to_where_it_is_on_the_timeline() {
    let Some((mut e, items)) = project(&["a.mp4"]) else { return };
    let clip = e.sequence().unwrap().tracks[0].clips[0].id;
    e.trim(clip, Edge::Start, secs(1.0), TrimMode::Ripple).unwrap();
    let start = s_of(e.sequence().unwrap().tracks[0].clips[0].start);
    let r = e.timeline_ranges(items[0], &proposal("a", 3.0, 4.0));
    assert_eq!(r.len(), 1);
    assert!((s_of(r[0].0) - (start + 2.0)).abs() < 0.04 && (s_of(r[0].1) - (start + 3.0)).abs() < 0.04, "{r:?}");
    assert!(e.timeline_ranges(items[0], &proposal("gone", 0.1, 0.9)).is_empty(), "before the used part");
    assert!(e.timeline_ranges(ItemId(987_654), &proposal("x", 3.0, 4.0)).is_empty());
}

#[test]
fn an_analysis_file_finds_its_own_media_by_length_then_by_name() {
    let Some((e, items)) = project(&["first.mp4", "second.mp4"]) else { return };
    let doc = |path: &str, dur: f64| {
        let json = serde_json::json!({"schema": "simplecut.retakes/1", "media": {"path": path, "duration_seconds": dur}, "proposals": []});
        simplecut_core::retake::parse(&json.to_string()).unwrap()
    };
    // two files of the same length: the name decides (either kind of slash, any case)
    assert_eq!(e.find_media_for(&doc("C:\\videos\\SECOND.mp4", 10.0)).unwrap(), items[1]);
    assert_eq!(e.find_media_for(&doc("/home/me/first.mp4", 10.1)).unwrap(), items[0]);
    // same length, no name to go by: refuse to guess
    assert!(matches!(e.find_media_for(&doc("other.mp4", 10.0)), Err(EditorError::Invalid(m)) if m.contains("More than one")));
    // no file that long
    assert!(matches!(e.find_media_for(&doc("first.mp4", 99.0)), Err(EditorError::Invalid(m)) if m.contains("None of the files")));
}

#[test]
fn cuts_in_several_files_are_one_undo_step() {
    let Some((mut e, items)) = project(&["a.mp4", "b.mp4"]) else { return };
    let steps = e.session().history.undo.len();
    let r = e.apply_cut_sets(&[(items[0], vec![proposal("a", 2.0, 3.0)]), (items[1], vec![proposal("b", 5.0, 6.0)])], CutScope::MainTracks).unwrap();
    assert_eq!(r.cuts, 2);
    assert!((s_of(r.removed) - 2.0).abs() < 0.04);
    assert!((s_of(e.sequence().unwrap().duration) - 18.0).abs() < 0.05);
    assert_eq!(e.session().history.undo.len(), steps + 1);
    e.undo().unwrap();
    assert!((s_of(e.sequence().unwrap().duration) - 20.0).abs() < 0.05);
    assert_eq!(e.apply_cut_sets(&[(items[0], vec![])], CutScope::MainTracks).unwrap_err(), EditorError::NothingSelected);
}
