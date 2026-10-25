//! Remove Retakes in the window, step by step: the button, the questions, the groups, the keep
//! ticks, the markers on the timeline, removing what is not kept, and the optional AI. The service
//! is the stand-in one; everything around it is real.
//!
//! Skipped (with a message) when ffmpeg / ffprobe are not installed.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::harness::{Script, click, item, skip, step_until, window, window_with, with_media_and_service};
use egui_kittest::kittest::{NodeT, Queryable};
use simplecut_core::{FakeService, Lane, Tick, Verdict};
use simplecut_ui::Action;

const TPS: f64 = 254_016_000_000.0;

fn s_of(t: Tick) -> f64 {
    t.0 as f64 / TPS
}

type Win = common::harness::Window;

fn ready(service: FakeService) -> (Win, std::path::PathBuf) {
    let (mut h, _, dir) = with_media_and_service(service);
    let video = item(&h, "talk.mp4");
    h.state_mut().perform(Action::AddToTimeline(video, Lane::Main));
    h.state_mut().perform(Action::SelectClips { clips: vec![], add: false });
    h.step();
    h.step();
    (h, dir)
}

fn stage(h: &mut Win, what: &'static str) {
    step_until(h, what, |a| a.review_stage() == what);
}

/// Start, answer the way the documentation does, press Next and wait for the groups.
fn to_review(h: &mut Win) {
    click(h, "Remove Retakes");
    stage(h, "questions");
    let ids = h.state().question_ids();
    assert_eq!(ids, ["9:0|11:0", "13:0|14:0", "17:0|18:0", "21:0|22:0"]);
    h.state_mut().perform(Action::AnswerPair { pair: ids[0].clone(), verdict: Some(Verdict::Retake) });
    h.state_mut().perform(Action::AnswerPair { pair: ids[1].clone(), verdict: None });
    h.state_mut().perform(Action::AnswerPair { pair: ids[2].clone(), verdict: None });
    h.state_mut().perform(Action::AnswerPair { pair: ids[3].clone(), verdict: Some(Verdict::Different) });
    h.step();
    h.state_mut().perform(Action::QuestionsNext);
    h.step();
    h.state_mut().perform(Action::QuestionsSendAnyway);
    stage(h, "review");
}

fn video_total(h: &Win) -> f64 {
    let v = h.state().editor().sequence().unwrap();
    v.tracks[0].clips.iter().map(|c| s_of(c.duration)).sum()
}

#[test]
fn the_button_appears_only_with_a_service_and_a_video_on_the_timeline() {
    if skip() {
        return;
    }
    let mut plain = window(Script::default());
    plain.step();
    assert!(plain.query_by_label("Remove Retakes").is_none(), "no service, no button");

    let mut with = window_with(Script::default(), Some(Box::new(FakeService::default())));
    with.step();
    assert!(with.get_by_label("Remove Retakes").accesskit_node().is_disabled(), "nothing on the timeline to look at");

    let (h, _) = ready(FakeService::default());
    assert!(!h.get_by_label("Remove Retakes").accesskit_node().is_disabled());
}

#[test]
fn the_steps_go_questions_then_groups_and_nothing_is_cut_until_the_user_chooses() {
    if skip() {
        return;
    }
    let (mut h, dir) = ready(FakeService::default());
    h.state_mut().set_label_log(Some(dir.join("labels.jsonl")));
    let steps = h.state().editor().session().history.undo.len();
    let before = video_total(&h);

    click(&mut h, "Remove Retakes");
    assert!(h.state().is_analyzing());
    h.get_by_label("Cancel analysis");
    stage(&mut h, "questions");
    assert_eq!(h.state().question_summary(), Some((4, 0, 0, 0)));
    h.get_by_label("Same line");
    h.get_by_label("Different lines");

    // answering with the real buttons moves to the next card
    click(&mut h, "Same line");
    assert_eq!(h.state().question_summary(), Some((4, 1, 0, 1)));
    click(&mut h, "Skip");
    assert_eq!(h.state().question_summary(), Some((4, 1, 1, 2)));

    // Next with questions left over asks first
    click(&mut h, "Next");
    assert_eq!(h.state().review_stage(), "questions", "waiting for the user to confirm");
    h.get_by_label("Send anyway");
    click(&mut h, "Send anyway");
    assert!(h.state().is_analyzing(), "the model is working");
    stage(&mut h, "review");

    // the answers were logged before they were sent
    let log = std::fs::read_to_string(dir.join("labels.jsonl")).unwrap();
    assert_eq!(log.lines().count(), 1, "{log}");
    assert!(log.contains("\"label\":\"retake\"") && log.contains("9:0|11:0") && log.contains("\"source\":\"user\""), "{log}");

    // likely groups first; nothing is cut yet, all sentences are kept
    let groups = h.state().review_groups();
    assert!(groups.len() >= 5, "{groups:?}");
    assert!(groups[0].2 && groups.iter().any(|g| !g.2), "there are likely and possible groups");
    assert_eq!(h.state().review_cut_count(), 0);
    assert!(groups.iter().flat_map(|g| g.3.iter()).all(|id| h.state().review_kept(id) == Some(true)));
    assert!(h.state().review_kept_rects().len() >= 10, "kept sentences are outlined");
    assert!(h.state().review_band_rects().len() >= 5, "every group has a band");
    assert!(h.state().review_marker_rects().is_empty(), "nothing will be cut");
    assert_eq!(h.state().editor().session().history.undo.len(), steps);
    h.get_by_label("Remove 0 sentences");

    // untick the first take of the first group
    let first = groups[0].3[0].clone();
    h.state_mut().perform(Action::SetKeep { sentence: first.clone(), keep: false });
    h.step();
    h.step();
    assert_eq!(h.state().review_kept(&first), Some(false));
    assert_eq!(h.state().review_origin(&first), Some("user"));
    assert_eq!(h.state().review_cut_count(), 1);
    assert_eq!(h.state().review_marker_rects().len(), 1);
    assert_eq!(h.state().review_marker_colors(), [groups[0].1]);
    h.get_by_label("Remove 1 sentence");

    click(&mut h, "Remove 1 sentence");
    assert!(video_total(&h) < before - 0.02, "{} vs {before}", video_total(&h));
    assert_eq!(h.state().editor().session().history.undo.len(), steps + 1, "one undo step");
    assert!(h.state().banner().is_some_and(|b| !b.error && b.text.contains("Removed")), "{:?}", h.state().banner());

    click(&mut h, "Undo");
    assert!((video_total(&h) - before).abs() < 0.05);
    assert_eq!(h.state().review_stage(), "review");
    assert_eq!(h.state().review_groups().len(), groups.len(), "undo brings the group back");
}

#[test]
fn at_least_one_sentence_of_a_group_stays_and_the_helpers_work() {
    if skip() {
        return;
    }
    let (mut h, _) = ready(FakeService::default());
    to_review(&mut h);
    let groups = h.state().review_groups();
    let ids = groups[0].3.clone();
    assert_eq!(ids.len(), 3);

    // keep last only
    let gid = "span_0".to_string();
    h.state_mut().perform(Action::KeepLastOnly(gid.clone()));
    h.step();
    assert_eq!(h.state().review_kept(&ids[0]), Some(false));
    assert_eq!(h.state().review_kept(&ids[1]), Some(false));
    assert_eq!(h.state().review_kept(&ids[2]), Some(true));
    assert_eq!(h.state().review_cut_count(), 2);

    // unticking the last sentence now works, cutting the whole group
    h.state_mut().perform(Action::SetKeep { sentence: ids[2].clone(), keep: false });
    h.step();
    assert_eq!(h.state().review_kept(&ids[2]), Some(false));
    assert_eq!(h.state().review_cut_count(), ids.len());
    assert!(!h.state().banner().is_some_and(|b| b.error));

    // remove all helper works directly
    h.state_mut().perform(Action::KeepAllOf(gid.clone()));
    h.step();
    assert_eq!(h.state().review_cut_count(), 0);
    h.state_mut().perform(Action::RemoveAllOf(gid.clone()));
    h.step();
    assert_eq!(h.state().review_cut_count(), ids.len());
    assert_eq!(h.state().review_kept(&ids[0]), Some(false));
    assert_eq!(h.state().review_kept(&ids[1]), Some(false));
    assert_eq!(h.state().review_kept(&ids[2]), Some(false));

    // buttons exist
    h.get_by_label("Remove all in Retake 1");
    h.get_by_label("Keep all in Retake 1");
    h.get_by_label("Keep last only in Retake 1");

    // keep all undoes the choice
    h.state_mut().perform(Action::KeepAllOf(gid));
    h.step();
    assert_eq!(h.state().review_cut_count(), 0);

    // one click for every likely group
    h.state_mut().perform(Action::KeepLastInLikely);
    h.step();
    let likely_cuts: usize = h.state().review_groups().iter().filter(|g| g.2).map(|g| g.3.len() - 1).sum();
    assert_eq!(h.state().review_cut_count(), likely_cuts);
    assert!(h.state().review_cut_count() > 0);
    assert!(
        h.state().review_groups().iter().filter(|g| !g.2).flat_map(|g| g.3.iter()).all(|id| h.state().review_kept(id) == Some(true)),
        "possible groups are left alone"
    );
}

#[test]
fn the_questions_can_be_asked_again_and_an_answer_can_be_changed() {
    if skip() {
        return;
    }
    let (mut h, _) = ready(FakeService::default());
    to_review(&mut h);
    h.state_mut().perform(Action::RedoQuestions);
    h.step();
    assert_eq!(h.state().review_stage(), "questions");
    assert_eq!(h.state().question_summary().map(|q| (q.0, q.1)), Some((4, 2)), "the answers are still there");
    h.state_mut().perform(Action::AnswerPair { pair: "17:0|18:0".into(), verdict: Some(Verdict::Retake) });
    h.state_mut().perform(Action::QuestionsNext);
    h.state_mut().perform(Action::QuestionsSendAnyway);
    stage(&mut h, "review");
    assert!(h.state().review_groups().iter().any(|g| g.3 == ["17:0", "18:0"] && g.2), "the changed answer made it likely");
}

#[test]
fn the_ai_button_exists_only_when_the_service_offers_it_and_only_fills_in_ticks() {
    if skip() {
        return;
    }
    let (mut none, _) = ready(FakeService { adjudicate: false, ..Default::default() });
    to_review(&mut none);
    assert!(none.query_by_label("Ask AI to choose").is_none());
    assert!(!none.state().ai_available());

    let (mut h, _) = ready(FakeService { adjudicate: true, ..Default::default() });
    to_review(&mut h);
    let before = video_total(&h);
    let steps = h.state().editor().session().history.undo.len();
    click(&mut h, "Ask AI to choose");
    h.get_by_label("Continue");
    assert!(!h.state().is_analyzing(), "nothing is sent before the user agrees");
    click(&mut h, "Continue");
    step_until(&mut h, "the AI", |a| !a.is_analyzing() && a.banner().is_some_and(|b| b.text.contains("AI chose")));
    assert!(h.state().banner().is_some_and(|b| !b.error && b.text.contains("Nothing is cut")), "{:?}", h.state().banner());
    let tagged: Vec<String> = h.state().review_groups().iter().flat_map(|g| g.3.clone()).filter(|id| h.state().review_origin(id) == Some("ai")).collect();
    assert!(!tagged.is_empty(), "the AI's ticks are tagged");
    assert!((video_total(&h) - before).abs() < 1e-9, "nothing was cut");
    assert_eq!(h.state().editor().session().history.undo.len(), steps);

    // the user can override
    let group = h.state().review_groups().iter().find(|g| g.3.contains(&tagged[0])).map(|g| g.3.clone()).unwrap();
    h.state_mut().perform(Action::SetKeep { sentence: tagged[0].clone(), keep: true });
    h.step();
    assert_eq!(h.state().review_origin(&tagged[0]), Some("user"));
    assert!(group.len() >= 2);
}

#[test]
fn a_service_with_nothing_to_ask_goes_straight_to_the_groups() {
    if skip() {
        return;
    }
    let (mut h, _) = ready(FakeService { skip_questions: true, ..Default::default() });
    click(&mut h, "Remove Retakes");
    stage(&mut h, "review");
    assert!(!h.state().review_groups().is_empty());
}

#[test]
fn cancelling_failing_and_wrong_files_leave_nothing_behind() {
    if skip() {
        return;
    }
    let (mut h, _) = ready(FakeService { steps: 50, ..Default::default() });
    click(&mut h, "Remove Retakes");
    click(&mut h, "Cancel analysis");
    assert_eq!(h.state().review_stage(), "none");
    assert!(!h.state().is_analyzing());

    let (mut h, _) = ready(FakeService { fail_analyze: Some("model not found".into()), ..Default::default() });
    click(&mut h, "Remove Retakes");
    step_until(&mut h, "the failure", |a| a.banner().is_some_and(|b| b.error));
    assert!(h.state().banner().unwrap().text.contains("model not found"));
    assert_eq!(h.state().review_stage(), "none");

    let (mut h, _) = ready(FakeService { wrong_length: true, ..Default::default() });
    click(&mut h, "Remove Retakes");
    step_until(&mut h, "the refusal", |a| a.banner().is_some_and(|b| b.error));
    assert_eq!(h.state().review_stage(), "none");

    // a failing model keeps the answers
    let (mut h, _) = ready(FakeService { fail_resolve: Some("out of memory".into()), ..Default::default() });
    click(&mut h, "Remove Retakes");
    stage(&mut h, "questions");
    h.state_mut().perform(Action::AnswerPair { pair: "9:0|11:0".into(), verdict: Some(Verdict::Retake) });
    h.state_mut().perform(Action::QuestionsNext);
    h.state_mut().perform(Action::QuestionsSendAnyway);
    step_until(&mut h, "the failure", |a| a.banner().is_some_and(|b| b.error && b.text.contains("out of memory")));
    stage(&mut h, "questions");
    assert_eq!(h.state().question_summary().map(|q| q.1), Some(1), "the answer is kept");
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

    let good = dir.join("talk.groups.json");
    let json = serde_json::json!({
        "schema": "simplecut.retakes/2",
        "stage": "groups",
        "job": "j_1",
        "media": { "path": "D:/elsewhere/TALK.mp4", "duration_seconds": 4.0 },
        "groups": [
            { "id": "g0", "start": 0.5, "end": 2.0, "likely": true, "sentences": [
                { "id": "a:0", "start": 0.5, "end": 1.0, "text": "Hello there" },
                { "id": "b:0", "start": 1.5, "end": 2.0, "text": "Hello there everyone" } ] },
            { "id": "g1", "start": 3.0, "end": 2.0, "likely": true, "sentences": [
                { "id": "c:0", "start": 3.0, "end": 3.2, "text": "x" },
                { "id": "d:0", "start": 3.4, "end": 3.6, "text": "y" } ] }
        ]
    });
    std::fs::write(&good, json.to_string()).unwrap();
    script.analysis.lock().unwrap().push_back(good);
    h.state_mut().perform(Action::OpenAnalysisFile);
    h.step();
    assert_eq!(h.state().review_stage(), "review");
    assert_eq!(h.state().review_groups().len(), 1);
    assert_eq!(h.state().review_ignored(), 1, "the reversed group is ignored and reported");

    // a file for a different video, one that is not JSON, and one that is missing leave the review alone
    let other = dir.join("other.json");
    std::fs::write(&other, json.to_string().replace("4.0", "99.0")).unwrap();
    h.state_mut().perform(Action::LoadAnalysis(other));
    h.step();
    assert!(h.state().banner().unwrap().error);
    let junk = dir.join("junk.json");
    std::fs::write(&junk, "not json").unwrap();
    h.state_mut().perform(Action::LoadAnalysis(junk));
    h.step();
    assert!(h.state().banner().unwrap().error);
    h.state_mut().perform(Action::LoadAnalysis(dir.join("nope.json")));
    h.step();
    assert!(h.state().banner().unwrap().text.contains("could not be found"));
    assert_eq!(h.state().review_groups().len(), 1);
}

#[test]
fn a_group_that_is_cut_moves_to_already_handled_and_undo_brings_it_back() {
    if skip() {
        return;
    }
    let (mut h, _) = ready(FakeService::default());
    to_review(&mut h);
    let n = h.state().review_groups().len();
    h.state_mut().perform(Action::KeepLastOnly("span_0".into()));
    h.state_mut().perform(Action::ApplyRetakes);
    h.step();
    h.step();
    assert_eq!(h.state().review_groups().len(), n - 1, "only one take of that line is on the timeline now");
    assert_eq!(h.state().review_handled().len(), 1);

    click(&mut h, "Undo");
    assert_eq!(h.state().review_groups().len(), n);
    assert!(h.state().review_handled().is_empty());
}

#[test]
fn preview_skips_unticked_sentences_checkbox_toggle_remove_and_undo() {
    if skip() {
        return;
    }
    let (mut h, _) = ready(FakeService::default());
    to_review(&mut h);

    // Initial state: review is open, "Skip unticked sentences when playing" default ON
    assert!(h.state().review_skips_unticked());
    // Untick sentences of span_0 (keep last only)
    h.state_mut().perform(Action::KeepLastOnly("span_0".into()));
    h.step();

    // Skips are set in player from Cut marks
    assert!(!h.state().player().current_skips().is_empty(), "player has skips from unticked sentences");

    // Toggle checkbox OFF
    click(&mut h, "Skip unticked sentences when playing");
    h.step();
    assert!(!h.state().review_skips_unticked());
    assert!(h.state().player().current_skips().is_empty(), "skips cleared when checkbox off");

    // Toggle checkbox back ON
    click(&mut h, "Skip unticked sentences when playing");
    h.step();
    assert!(h.state().review_skips_unticked());
    assert!(!h.state().player().current_skips().is_empty(), "skips restored when checkbox on");

    // After Remove: nothing left to skip
    h.state_mut().perform(Action::ApplyRetakes);
    h.step();
    assert!(h.state().player().current_skips().is_empty(), "nothing left to skip after Remove");

    // Undo brings the skips back
    click(&mut h, "Undo");
    h.step();
    assert!(!h.state().player().current_skips().is_empty(), "Undo brings skips back");

    // Close review clears skips
    h.state_mut().perform(Action::CloseReview);
    h.step();
    assert!(h.state().player().current_skips().is_empty(), "closing review clears skips");
}
