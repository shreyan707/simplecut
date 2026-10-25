//! Editing on the timeline with the mouse: select, move, trim, scrub, zoom, mute and volume. Real
//! press / move / release events go to the real window; the editor underneath is checked.
//!
//! Skipped (with a message) when ffmpeg / ffprobe are not installed.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::harness::{Window, click, secs, skip, with_media};
use egui::{Event, Modifiers, MouseWheelUnit, Pos2, Rect, vec2};
use egui_kittest::kittest::Queryable;
use simplecut_core::{ClipId, ItemId, Lane, SequenceView, Tick, TrackView};
use simplecut_ui::Action;

const TPS: f64 = 254_016_000_000.0;

fn seq(h: &Window) -> SequenceView {
    h.state().editor().sequence().unwrap()
}

fn track<'a>(v: &'a SequenceView, name: &str) -> &'a TrackView {
    v.tracks.iter().find(|t| t.name == name).unwrap_or_else(|| panic!("no track {name}"))
}

/// A clip's (start, end) in milliseconds.
fn spans(t: &TrackView) -> Vec<(i64, i64)> {
    let ms = |x: Tick| (x.0 as f64 / TPS * 1000.0).round() as i64;
    t.clips.iter().map(|c| (ms(c.start), ms(c.end()))).collect()
}

fn near(a: &[(i64, i64)], b: &[(i64, i64)]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x.0 - y.0).abs() <= 40 && (x.1 - y.1).abs() <= 40)
}

fn assert_valid(h: &Window) {
    h.state().editor().session().active_sequence().unwrap().check().expect("no overlaps");
    let v = seq(h);
    let (video, dialogue) = (track(&v, "Video"), track(&v, "Dialogue"));
    for c in video.clips.iter().filter(|c| c.link.is_some()) {
        let d = dialogue.clips.iter().find(|d| d.link == c.link).expect("picture kept its sound");
        assert_eq!((c.start, c.duration), (d.start, d.duration), "picture and sound stay together");
    }
}

fn undo_steps(h: &Window) -> usize {
    h.state().editor().session().history.undo.len()
}

/// Two 4 s clips back to back on the main tracks, and 6 s of music under them.
fn two_clips() -> (Window, ItemId) {
    let (mut h, _, _) = with_media();
    let video = h.state().editor().media().iter().find(|m| m.name == "talk.mp4").unwrap().id;
    let music = h.state().editor().media().iter().find(|m| m.name == "bed.wav").unwrap().id;
    h.state_mut().perform(Action::AddToTimeline(video, Lane::Main));
    h.state_mut().perform(Action::AddToTimeline(video, Lane::Main));
    h.state_mut().perform(Action::AddToTimeline(music, Lane::Music));
    h.state_mut().perform(Action::SelectClips { clips: vec![], add: false });
    h.step();
    h.step();
    (h, video)
}

fn video_clips(h: &Window) -> (ClipId, ClipId) {
    let v = seq(h);
    let t = track(&v, "Video");
    (t.clips[0].id, t.clips[1].id)
}

fn rect(h: &Window, c: ClipId) -> Rect {
    h.state().timeline_clip_rect(c).expect("the clip was drawn")
}

fn pps(h: &Window) -> f32 {
    h.state().timeline_scale().unwrap() as f32
}

fn press_release(h: &mut Window, p: Pos2) {
    h.hover_at(p);
    h.step();
    h.drag_at(p);
    h.step();
    h.drop_at(p);
    h.step();
    h.step();
}

/// Press at `from`, move to `to` in a few frames, release.
fn drag(h: &mut Window, from: Pos2, to: Pos2) {
    h.hover_at(from);
    h.step();
    h.drag_at(from);
    h.step();
    for i in 1..=5 {
        h.hover_at(from + (to - from) * (i as f32 / 5.0));
        h.step();
    }
    h.drop_at(to);
    h.step();
    h.step();
}

fn click_clip(h: &mut Window, c: ClipId) {
    let p = rect(h, c).center();
    press_release(h, p);
}

fn hold(h: &mut Window, m: Modifiers) {
    h.event(Event::ModifiersChanged(m));
    h.step();
}

#[test]
fn clicking_selects_a_clip_with_its_sound_and_ctrl_click_adds_and_removes() {
    if skip() {
        return;
    }
    let (mut h, _) = two_clips();
    let (a, b) = video_clips(&h);
    click_clip(&mut h, a);
    let v = seq(&h);
    assert_eq!(h.state().editor().selection().len(), 2, "the picture and its sound");
    assert!(v.clip(a).unwrap().selected && !v.clip(b).unwrap().selected);

    hold(&mut h, Modifiers::COMMAND);
    click_clip(&mut h, b);
    assert_eq!(h.state().editor().selection().len(), 4, "ctrl-click adds");
    click_clip(&mut h, b);
    assert_eq!(h.state().editor().selection().len(), 2, "ctrl-click again takes it out");
    hold(&mut h, Modifiers::NONE);
    assert!(seq(&h).clip(a).unwrap().selected);

    // empty space: deselect and put the playhead there
    let video_lane = h.state().timeline_lane(0).unwrap();
    let x = h.state().timeline_x(secs(9.5)).unwrap();
    press_release(&mut h, Pos2::new(x, video_lane.center().y));
    assert!(h.state().editor().selection().is_empty());
    assert!((h.state().editor().playhead().0 as f64 / TPS - 9.5).abs() < 0.1, "{:?}", h.state().editor().playhead());
}

#[test]
fn dragging_a_clip_moves_it_and_its_sound_in_one_undo_step() {
    if skip() {
        return;
    }
    let (mut h, _) = two_clips();
    let (_, b) = video_clips(&h);
    let before = spans(track(&seq(&h), "Video"));
    let steps = undo_steps(&h);
    let from = rect(&h, b).center();
    let k = pps(&h);
    drag(&mut h, from, from + vec2(2.0 * k, 0.0));

    let v = seq(&h);
    assert!(near(&spans(track(&v, "Video")), &[(0, 4000), (6000, 10_000)]), "{:?}", spans(track(&v, "Video")));
    assert!(near(&spans(track(&v, "Dialogue")), &[(0, 4000), (6000, 10_000)]), "its sound went with it");
    assert_eq!(spans(track(&v, "Music")), [(0, 6000)], "music was not touched");
    assert_valid(&h);
    assert_eq!(undo_steps(&h), steps + 1, "one drag, one undo step");

    h.state_mut().perform(Action::Undo);
    assert_eq!(spans(track(&seq(&h), "Video")), before);
}

#[test]
fn a_dragged_clip_never_lands_on_top_of_another() {
    if skip() {
        return;
    }
    let (mut h, _) = two_clips();
    let (a, _) = video_clips(&h);
    let before = (spans(track(&seq(&h), "Video")), undo_steps(&h));

    // onto the second clip: the nearest free place is where it already is, so nothing happens
    let from = rect(&h, a).center();
    let k = pps(&h);
    drag(&mut h, from, from + vec2(2.0 * k, 0.0));
    assert_eq!((spans(track(&seq(&h), "Video")), undo_steps(&h)), before, "no change, and no undo step for nothing");
    assert!(h.state().banner().is_none_or(|b| !b.error));

    // well past the second clip: it lands after it, not over it
    let from = rect(&h, a).center();
    let k = pps(&h);
    drag(&mut h, from, from + vec2(7.0 * k, 0.0));
    let v = seq(&h);
    let video = spans(track(&v, "Video"));
    assert_eq!(video.len(), 2);
    assert!(video.iter().any(|s| (s.0 - 4000).abs() <= 40 && (s.1 - 8000).abs() <= 40), "the other clip is whole: {video:?}");
    assert!(video.iter().any(|s| s.0 >= 7960), "the dragged clip sits after it: {video:?}");
    assert_valid(&h);
}

#[test]
fn dragging_a_clip_edge_trims_it_and_closes_the_gap_unless_alt_is_held() {
    if skip() {
        return;
    }
    let (mut h, _) = two_clips();
    let (a, _) = video_clips(&h);
    let edge = |h: &Window| {
        let r = rect(h, a);
        Pos2::new(r.right() - 2.0, r.center().y)
    };

    // default: ripple, so the next clip follows
    let from = edge(&h);
    let k = pps(&h);
    drag(&mut h, from, from - vec2(1.0 * k, 0.0));
    let v = seq(&h);
    assert!(near(&spans(track(&v, "Video")), &[(0, 3000), (3000, 7000)]), "{:?}", spans(track(&v, "Video")));
    assert!(near(&spans(track(&v, "Dialogue")), &[(0, 3000), (3000, 7000)]), "the sound is trimmed with the picture");
    assert_valid(&h);
    h.state_mut().perform(Action::Undo);

    // with Alt: a plain trim, the gap stays
    hold(&mut h, Modifiers::ALT);
    let from = edge(&h);
    let k = pps(&h);
    drag(&mut h, from, from - vec2(1.0 * k, 0.0));
    hold(&mut h, Modifiers::NONE);
    let v = seq(&h);
    assert!(near(&spans(track(&v, "Video")), &[(0, 3000), (4000, 8000)]), "{:?}", spans(track(&v, "Video")));
    assert_valid(&h);
}

#[test]
fn trimming_the_start_skips_into_the_source_and_cannot_stretch_past_the_media() {
    if skip() {
        return;
    }
    let (mut h, _) = two_clips();
    let (_, b) = video_clips(&h);
    let r = rect(&h, b);
    let from = Pos2::new(r.left() + 2.0, r.center().y);
    let k = pps(&h);
    drag(&mut h, from, from + vec2(1.0 * k, 0.0));
    let v = seq(&h);
    let clip = v.clip(b).unwrap();
    assert!((clip.duration.0 as f64 / TPS - 3.0).abs() < 0.05, "{:?}", clip.duration);
    assert!((clip.source_in.0 as f64 / TPS - 1.0).abs() < 0.05, "starts one second into the file: {:?}", clip.source_in);
    assert_valid(&h);
    h.state_mut().perform(Action::Undo);

    // the first clip already shows its whole file: pulling its end out does nothing
    let (a, _) = video_clips(&h);
    let r = rect(&h, a);
    let from = Pos2::new(r.right() - 2.0, r.center().y);
    let steps = undo_steps(&h);
    let k = pps(&h);
    drag(&mut h, from, from + vec2(1.5 * k, 0.0));
    assert_eq!(undo_steps(&h), steps, "nothing to trim, nothing changed");
    assert!(h.state().banner().is_none_or(|b| !b.error));
    assert!(near(&spans(track(&seq(&h), "Video")), &[(0, 4000), (4000, 8000)]));
}

#[test]
fn dragging_the_ruler_moves_the_playhead_and_shows_the_timeline() {
    if skip() {
        return;
    }
    let (mut h, _) = two_clips();
    let ruler = h.state().timeline_ruler().unwrap();
    let y = ruler.center().y;
    let (x1, x5) = (h.state().timeline_x(secs(1.0)).unwrap(), h.state().timeline_x(secs(5.0)).unwrap());
    drag(&mut h, Pos2::new(x1, y), Pos2::new(x5, y));
    assert!((h.state().editor().playhead().0 as f64 / TPS - 5.0).abs() < 0.1, "{:?}", h.state().editor().playhead());
    assert_eq!(h.state().source(), simplecut_ui::Source::Timeline);
}

#[test]
fn track_mute_and_clip_volume_controls_work() {
    if skip() {
        return;
    }
    let (mut h, _) = two_clips();
    assert!(h.query_by_label("Clip volume").is_none(), "no volume control until something with sound is selected");

    click(&mut h, "Mute Dialogue");
    assert!(track(&seq(&h), "Dialogue").muted);
    click(&mut h, "Mute Dialogue");
    assert!(!track(&seq(&h), "Dialogue").muted);

    let (a, _) = video_clips(&h);
    click_clip(&mut h, a);
    h.step();
    let sound = track(&seq(&h), "Dialogue").clips[0].id;
    let control = h.get_by_label("Clip volume").rect();
    let steps = undo_steps(&h);
    drag(&mut h, control.center(), control.center() + vec2(40.0, 0.0));
    let gain = seq(&h).clip(sound).unwrap().gain_db;
    assert!(gain > 4.0 && gain <= 24.0, "dragging the number up raised the volume: {gain} dB");
    assert_eq!(undo_steps(&h), steps + 1, "a whole drag is one undo step");
    assert_eq!(seq(&h).clip(a).unwrap().gain_db, 0.0, "the picture has no volume of its own");
}

#[test]
fn the_zoom_buttons_and_ctrl_wheel_zoom_the_timeline() {
    if skip() {
        return;
    }
    let (mut h, _) = two_clips();
    let fit = pps(&h);
    assert!(!h.state().timeline_zoomed());
    click(&mut h, "Zoom in");
    assert!(h.state().timeline_zoomed());
    let zoomed = pps(&h);
    assert!((zoomed / fit - 1.5).abs() < 0.01, "{fit} -> {zoomed}");
    click(&mut h, "Zoom out");
    click(&mut h, "Fit");
    assert!(!h.state().timeline_zoomed());
    assert!((pps(&h) - fit).abs() < 0.01);

    // Ctrl + wheel over the lanes
    let lane = h.state().timeline_lane(0).unwrap();
    h.hover_at(lane.center());
    h.step();
    hold(&mut h, Modifiers::COMMAND);
    h.event(Event::MouseWheel { unit: MouseWheelUnit::Point, delta: vec2(0.0, 60.0), modifiers: Modifiers::COMMAND, phase: egui::TouchPhase::Move });
    for _ in 0..6 {
        h.step();
    }
    hold(&mut h, Modifiers::NONE);
    assert!(h.state().timeline_zoomed() && pps(&h) > fit * 1.2, "{} vs {fit}", pps(&h));
}

#[test]
fn delete_removes_the_selected_clip_with_its_sound_and_closes_the_gap() {
    if skip() {
        return;
    }
    let (mut h, _) = two_clips();
    let (a, _) = video_clips(&h);
    click_clip(&mut h, a);
    h.key_press(egui::Key::Delete);
    h.step();
    h.step();
    let v = seq(&h);
    assert!(near(&spans(track(&v, "Video")), &[(0, 4000)]), "the second clip moved up: {:?}", spans(track(&v, "Video")));
    assert!(near(&spans(track(&v, "Dialogue")), &[(0, 4000)]));
    assert_eq!(spans(track(&v, "Music")), [(0, 6000)], "music stays where it was");
    assert_valid(&h);
}
