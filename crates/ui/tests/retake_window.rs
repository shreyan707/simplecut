//! Remove Retakes in the window: the button, the analysis, the review list, the markers on the
//! timeline, applying the approved cuts, and loading an analysis file by hand. The detector is the
//! stand-in one; everything around it is real.
//!
//! Skipped (with a message) when ffmpeg / ffprobe are not installed.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::time::Duration;

use common::harness::{Script, click, item, skip, step_until, window, window_with, with_media_and_detector};
use egui_kittest::kittest::{NodeT, Queryable};
use simplecut_core::{FakeAnalyzer, Lane, Tick};
use simplecut_ui::{Action, Source};

const TPS: f64 = 254_016_000_000.0;

fn s_of(t: Tick) -> f64 {
    t.0 as f64 / TPS
}

type Win = common::harness::Window;

/// The 4 s clip placed on the timeline, a detector connected, ready to look for retakes.
fn ready(detector: FakeAnalyzer) -> Win {
    let (mut h, _, _) = with_media_and_detector(detector);
    let video = item(&h, "talk.mp4");
    h.state_mut().perform(Action::AddToTimeline(video, Lane::Main));
    h.state_mut().perform(Action::SelectClips { clips: vec![], add: false });
    h.step();
    h.step();
    h
}

fn analyse(h: &mut Win) {
    click(h, "Remove Retakes");
    step_until(h, "the review", |a| a.review_summary().is_some());
}

fn video_pieces(h: &Win) -> Vec<(f64, f64)> {
    let v = h.state().editor().sequence().unwrap();
    v.tracks[0].clips.iter().map(|c| (s_of(c.duration), s_of(c.source_in))).collect()
}

#[test]
fn the_button_appears_only_with_a_detector_and_a_video_on_the_timeline() {
    if skip() {
        return;
    }
    let mut plain = window(Script::default());
    plain.step();
    assert!(plain.query_by_label("Remove Retakes").is_none(), "no detector, no button");

    let mut with = window_with(Script::default(), Some(Box::new(FakeAnalyzer::default())));
    with.step();
    assert!(with.get_by_label("Remove Retakes").accesskit_node().is_disabled(), "nothing on the timeline to look at");

    let mut h = ready(FakeAnalyzer::default());
    assert!(!h.get_by_label("Remove Retakes").accesskit_node().is_disabled());
    h.step();
}

#[test]
fn finding_reviewing_and_removing_retakes_is_one_undo_step() {
    if skip() {
        return;
    }
    let mut h = ready(FakeAnalyzer::default());
    let before = video_pieces(&h);
    let steps = h.state().editor().session().history.undo.len();

    click(&mut h, "Remove Retakes");
    assert!(h.state().is_analyzing());
    h.get_by_label("Cancel analysis");
    step_until(&mut h, "the review", |a| a.review_summary().is_some());
    assert!(!h.state().is_analyzing());

    // the sure ones are selected, the unsure one is left for the user
    assert_eq!(h.state().review_summary(), Some((3, 2)));
    assert!(h.state().review_is_approved("fake_0:0") && h.state().review_is_approved("fake_2:0"));
    assert!(!h.state().review_is_approved("fake_1:0"), "a 'review' suggestion is never pre-selected");
    assert!(h.state().banner().is_some_and(|b| !b.error && b.text.contains("Found 3") && b.text.contains("2 selected")));
    h.get_by_label("Remove 2 retakes");

    // markers on the timeline sit on the video lane
    let lane = h.state().timeline_lane(0).unwrap();
    let markers = h.state().review_marker_rects();
    assert_eq!(markers.len(), 3);
    assert!(markers.iter().all(|m| m.top() >= lane.top() - 1.0 && m.bottom() <= lane.bottom() + 1.0 && m.width() > 1.0));

    click(&mut h, "Remove 2 retakes");
    let after = video_pieces(&h);
    // the file's 0.4-0.6 s and 2.8-2.88 s are gone: it plays 0-0.4, 0.6-2.8 and 2.88-4
    assert_eq!(after.len(), 3, "{after:?}");
    for (got, want) in after.iter().map(|p| p.1).zip([0.0, 0.6, 2.88]) {
        assert!((got - want).abs() < 0.05, "{after:?}");
    }
    let total: f64 = after.iter().map(|p| p.0).sum();
    assert!((total - (4.0 - 0.2 - 0.08)).abs() < 0.08, "{total}");
    assert!(h.state().banner().is_some_and(|b| !b.error && b.text.contains("Removed 2 retakes")), "{:?}", h.state().banner());
    assert_eq!(h.state().editor().session().history.undo.len(), steps + 1, "one undo step");
    assert_eq!(h.state().review_summary(), Some((1, 0)), "the one left for review is still listed");
    assert_eq!(h.state().review_marker_rects().len(), 1);

    // undo puts everything back
    click(&mut h, "Undo");
    assert_eq!(video_pieces(&h).len(), before.len());
    assert!((video_pieces(&h).iter().map(|p| p.0).sum::<f64>() - 4.0).abs() < 0.05);
}

#[test]
fn suggestions_can_be_toggled_and_selected_in_bulk() {
    if skip() {
        return;
    }
    let mut h = ready(FakeAnalyzer::default());
    analyse(&mut h);
    click(&mut h, "Select suggestion fake_1:0");
    assert_eq!(h.state().review_summary(), Some((3, 3)));
    click(&mut h, "Select suggestion fake_1:0");
    assert_eq!(h.state().review_summary(), Some((3, 2)));
    click(&mut h, "Select none");
    assert_eq!(h.state().review_summary(), Some((3, 0)));
    assert!(h.get_by_label("Remove 0 retakes").accesskit_node().is_disabled(), "nothing selected, nothing to remove");
    click(&mut h, "Select sure ones");
    assert_eq!(h.state().review_summary(), Some((3, 2)));
    let clips = h.state().editor().sequence().unwrap().clip_count();
    assert_eq!(clips, 2, "reviewing cuts nothing");
}

#[test]
fn go_to_moves_the_playhead_to_the_suggestion() {
    if skip() {
        return;
    }
    let mut h = ready(FakeAnalyzer::default());
    analyse(&mut h);
    click(&mut h, "Go to fake_2:0");
    h.step();
    assert!((s_of(h.state().editor().playhead()) - 2.8).abs() < 0.1, "{:?}", h.state().editor().playhead());
    assert_eq!(h.state().source(), Source::Timeline);
}

#[test]
fn closing_the_review_cuts_nothing() {
    if skip() {
        return;
    }
    let mut h = ready(FakeAnalyzer::default());
    analyse(&mut h);
    click(&mut h, "Close");
    assert_eq!(h.state().review_summary(), None);
    assert!(h.state().review_marker_rects().is_empty());
    assert_eq!(video_pieces(&h).len(), 1);
    assert_eq!(h.state().editor().sequence().unwrap().clip_count(), 2, "the picture and its sound, untouched");
}

#[test]
fn an_analysis_can_be_cancelled_and_nothing_is_left_behind() {
    if skip() {
        return;
    }
    let mut h = ready(FakeAnalyzer { steps: 200, step_delay: Duration::from_millis(30), ..Default::default() });
    click(&mut h, "Remove Retakes");
    assert!(h.state().is_analyzing());
    assert!(h.get_by_label("Remove Retakes").accesskit_node().is_disabled(), "one analysis at a time");
    click(&mut h, "Cancel analysis");
    step_until(&mut h, "the cancel", |a| !a.is_analyzing());
    assert_eq!(h.state().review_summary(), None);
    assert_eq!(h.state().banner().map(|b| b.text.as_str()), Some("Analysis cancelled."));
}

#[test]
fn a_detector_that_fails_or_answers_for_another_file_is_reported() {
    if skip() {
        return;
    }
    let mut h = ready(FakeAnalyzer { fail_with: Some("the speech model is not installed".into()), ..Default::default() });
    click(&mut h, "Remove Retakes");
    step_until(&mut h, "the failure", |a| a.banner().is_some_and(|b| b.error));
    let b = h.state().banner().unwrap().clone();
    assert!(b.text.contains("could not finish") && b.text.contains("speech model is not installed"), "{b:?}");
    assert_eq!(h.state().review_summary(), None);

    let mut h = ready(FakeAnalyzer { wrong_length: true, ..Default::default() });
    click(&mut h, "Remove Retakes");
    step_until(&mut h, "the refusal", |a| a.banner().is_some_and(|b| b.error));
    assert!(h.state().banner().unwrap().text.contains("different file"));
    assert_eq!(h.state().review_summary(), None);
}

#[test]
fn an_analysis_file_made_by_hand_can_be_loaded() {
    if skip() {
        return;
    }
    let (mut h, script, dir) = common::harness::with_media();
    let video = item(&h, "talk.mp4");
    h.state_mut().perform(Action::AddToTimeline(video, Lane::Main));
    h.step();

    let good = dir.join("talk.retakes.json");
    let json = serde_json::json!({
        "schema": "simplecut.retakes/1",
        "media": { "path": "D:/elsewhere/TALK.mp4", "duration_seconds": 4.0 },
        "proposals": [
            { "id": "h:0", "start": 1.0, "end": 1.5, "confidence": 0.9, "reason": "stumbled" },
            { "id": "h:1", "start": 3.0, "end": 2.0, "confidence": 0.9 }
        ]
    });
    std::fs::write(&good, json.to_string()).unwrap();
    script.analysis.lock().unwrap().push_back(good);
    h.state_mut().perform(Action::OpenAnalysisFile);
    h.step();
    assert_eq!(h.state().review_summary(), Some((1, 1)));
    assert_eq!(h.state().review_ignored(), 1, "the reversed one is ignored and reported");
    assert_eq!(h.state().review_ids(), ["h:0"]);

    // a file that is for a different video, one that is not JSON, and one that is missing
    let other = dir.join("other.json");
    std::fs::write(&other, json.to_string().replace("4.0", "99.0")).unwrap();
    h.state_mut().perform(Action::LoadAnalysis(other));
    h.step();
    assert!(h.state().banner().unwrap().text.contains("None of the files"), "{:?}", h.state().banner());
    let junk = dir.join("junk.json");
    std::fs::write(&junk, "not json").unwrap();
    h.state_mut().perform(Action::LoadAnalysis(junk));
    h.step();
    assert!(h.state().banner().unwrap().error);
    h.state_mut().perform(Action::LoadAnalysis(dir.join("nope.json")));
    h.step();
    assert!(h.state().banner().unwrap().text.contains("could not be found"));
    assert_eq!(h.state().review_summary(), Some((1, 1)), "failed loads leave the open review alone");
}

#[test]
fn undo_and_redo_with_review_open_brings_suggestions_back() {
    if skip() {
        return;
    }
    let mut h = ready(FakeAnalyzer::default());
    analyse(&mut h);
    assert_eq!(h.state().review_summary(), Some((3, 2)));
    assert_eq!(h.state().review_marker_rects().len(), 3);

    click(&mut h, "Remove 2 retakes");
    assert_eq!(h.state().review_summary(), Some((1, 0)));
    assert_eq!(h.state().review_marker_rects().len(), 1);

    click(&mut h, "Undo");
    assert_eq!(h.state().review_summary(), Some((3, 2)));
    assert!(h.state().review_is_approved("fake_0:0"));
    assert!(h.state().review_is_approved("fake_2:0"));
    assert_eq!(h.state().review_marker_rects().len(), 3);

    click(&mut h, "Redo");
    assert_eq!(h.state().review_summary(), Some((1, 0)));
    assert_eq!(h.state().review_marker_rects().len(), 1);
}

#[test]
fn rerun_analysis_on_cut_video_lists_cut_ones_as_already_removed_without_sliver() {
    if skip() {
        return;
    }
    let mut h = ready(FakeAnalyzer::default());
    analyse(&mut h);
    click(&mut h, "Remove 2 retakes");

    click(&mut h, "Remove Retakes");
    step_until(&mut h, "the re-analysis", |a| !a.is_analyzing());

    assert_eq!(h.state().review_summary(), Some((1, 0)));
    assert_eq!(h.state().review_marker_rects().len(), 1);
    assert!(h.get_by_label("Remove 0 retakes").accesskit_node().is_disabled());
    assert!(h.state().banner().is_some_and(|b| !b.error));
}

#[test]
fn a_suggestion_more_than_90_percent_cut_is_off_and_has_no_marker() {
    if skip() {
        return;
    }
    let (mut h, script, dir) = common::harness::with_media();
    let video = item(&h, "talk.mp4");
    h.state_mut().perform(Action::AddToTimeline(video, Lane::Main));
    h.step();

    // Trim the start by 0.95 s of a 1.0 s suggestion (leaves only 5% of it on timeline)
    let clip = h.state().editor().sequence().unwrap().tracks[0].clips[0].id;
    h.state_mut().perform(Action::TrimClip { clip, edge: simplecut_core::Edge::Start, delta: Tick::from_seconds_f64(0.95), ripple: true });
    h.step();

    let path = dir.join("sliver.retakes.json");
    let json = serde_json::json!({
        "schema": "simplecut.retakes/1",
        "media": { "path": "D:/talk.mp4", "duration_seconds": 4.0 },
        "proposals": [
            { "id": "s:0", "start": 0.0, "end": 1.0, "confidence": 0.9 }
        ]
    });
    std::fs::write(&path, json.to_string()).unwrap();
    script.analysis.lock().unwrap().push_back(path);
    h.state_mut().perform(Action::OpenAnalysisFile);
    h.step();

    // Only 0.05s (5%) is on the timeline: should be Off, so review_summary is (0, 0) and 0 markers
    assert_eq!(h.state().review_summary(), Some((0, 0)));
    assert_eq!(h.state().review_marker_rects().len(), 0);
}

#[test]
fn trimming_clip_moves_suggestion_to_outside_used_part_and_undo_returns_it() {
    if skip() {
        return;
    }
    let (mut h, script, dir) = common::harness::with_media();
    let video = item(&h, "talk.mp4");
    h.state_mut().perform(Action::AddToTimeline(video, Lane::Main));
    h.step();

    let path = dir.join("trim.retakes.json");
    let json = serde_json::json!({
        "schema": "simplecut.retakes/1",
        "media": { "path": "D:/talk.mp4", "duration_seconds": 4.0 },
        "proposals": [
            { "id": "t:0", "start": 0.5, "end": 1.0, "confidence": 0.9 },
            { "id": "t:1", "start": 2.5, "end": 3.0, "confidence": 0.9 }
        ]
    });
    std::fs::write(&path, json.to_string()).unwrap();
    script.analysis.lock().unwrap().push_back(path);
    h.state_mut().perform(Action::OpenAnalysisFile);
    h.step();

    assert_eq!(h.state().review_summary(), Some((2, 2)));

    // Trim the start of the clip past 1.0s (1.5s delta ripple): t:0 is outside the used part
    let clip = h.state().editor().sequence().unwrap().tracks[0].clips[0].id;
    h.state_mut().perform(Action::TrimClip { clip, edge: simplecut_core::Edge::Start, delta: Tick::from_seconds_f64(1.5), ripple: true });
    h.step();

    assert_eq!(h.state().review_summary(), Some((1, 1)));
    assert_eq!(h.state().review_marker_rects().len(), 1);

    // Undo returns t:0 to On
    click(&mut h, "Undo");
    assert_eq!(h.state().review_summary(), Some((2, 2)));
    assert_eq!(h.state().review_marker_rects().len(), 2);
}

#[test]
fn counts_and_button_label_ignore_off_suggestions() {
    if skip() {
        return;
    }
    let mut h = ready(FakeAnalyzer::default());
    analyse(&mut h);
    assert_eq!(h.state().review_summary(), Some((3, 2)));
    h.get_by_label("Remove 2 retakes");

    // Remove the 2 retakes
    click(&mut h, "Remove 2 retakes");
    // Now 2 are Off, only 1 is On (and it was a review one, unselected)
    assert_eq!(h.state().review_summary(), Some((1, 0)));
    assert!(h.get_by_label("Remove 0 retakes").accesskit_node().is_disabled());
    assert_eq!(h.state().review_marker_rects().len(), 1);

    // Select the 1 remaining suggestion
    click(&mut h, "Select suggestion fake_1:0");
    assert_eq!(h.state().review_summary(), Some((1, 1)));
    assert!(!h.get_by_label("Remove 1 retake").accesskit_node().is_disabled());
}
