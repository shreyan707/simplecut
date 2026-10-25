//! Cutting retakes out of a real timeline: the contract's times (seconds on the media file's own
//! clock) must land on the right frames however the clip was trimmed, moved or repeated; picture and
//! sound stay together, music is untouched, and the whole thing is one undo step.
//!
//! Skipped (with a message) when ffmpeg / ffprobe are not installed.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::{have, make_clip, make_music, scratch};
use simplecut_core::retake::{CutProposal, CutScope, Decision};
use simplecut_core::{Edge, Editor, EditorError, ItemId, Lane, PlaceAt, SequenceView, Tick, TrackView, TrimMode};

const TPS: f64 = 254_016_000_000.0;

fn secs(s: f64) -> Tick {
    Tick::from_seconds_f64(s)
}

fn s_of(t: Tick) -> f64 {
    t.0 as f64 / TPS
}

fn track<'a>(v: &'a SequenceView, name: &str) -> &'a TrackView {
    v.tracks.iter().find(|t| t.name == name).unwrap()
}

/// Each clip as (timeline start, duration, source start) in seconds.
fn pieces(t: &TrackView) -> Vec<(f64, f64, f64)> {
    t.clips.iter().map(|c| (s_of(c.start), s_of(c.duration), s_of(c.source_in))).collect()
}

fn same(a: &[(f64, f64, f64)], b: &[(f64, f64, f64)]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x.0 - y.0).abs() < 0.04 && (x.1 - y.1).abs() < 0.04 && (x.2 - y.2).abs() < 0.04)
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

fn assert_valid(e: &Editor) {
    e.session().active_sequence().unwrap().check().unwrap();
    let v = e.sequence().unwrap();
    let (video, dialogue) = (track(&v, "Video"), track(&v, "Dialogue"));
    for c in video.clips.iter().filter(|c| c.link.is_some()) {
        let d = dialogue.clips.iter().find(|d| d.link == c.link).expect("picture kept its sound");
        assert_eq!((c.start, c.duration, c.source_in), (d.start, d.duration, d.source_in), "picture and sound stay together");
    }
    // no gaps on the main tracks
    for t in [video, dialogue] {
        for w in t.clips.windows(2) {
            assert_eq!(w[0].end(), w[1].start, "no gap left behind");
        }
    }
}

/// A project with a 10 s clip placed, and 10 s of music under it.
fn project() -> Option<(Editor, ItemId)> {
    if !have("ffmpeg") || !have("ffprobe") {
        eprintln!("SKIPPED: ffmpeg / ffprobe are not installed");
        return None;
    }
    let dir = scratch("retake");
    let mut e = Editor::new();
    e.new_project("T").unwrap();
    let item = e.import(&[make_clip(&dir, "a.mp4", 10, true)]).unwrap().imported[0];
    let music = e.import(&[make_music(&dir, "bed.wav", 10).unwrap()]).unwrap().imported[0];
    e.place(item, Lane::Main, PlaceAt::End).unwrap();
    e.place(music, Lane::Music, PlaceAt::At(Tick::ZERO)).unwrap();
    e.select(&[]).unwrap();
    Some((e, item))
}

#[test]
fn cuts_in_file_time_land_on_the_right_places_in_one_undo_step() {
    let Some((mut e, item)) = project() else { return };
    let before = e.sequence().unwrap();
    let steps = e.session().history.undo.len();

    let r = e.apply_cuts(item, &[proposal("a", 2.0, 3.0), proposal("b", 6.0, 7.0)], CutScope::MainTracks).unwrap();
    assert_eq!(r.cuts, 2);
    assert!((s_of(r.removed) - 2.0).abs() < 0.04, "{:?}", r.removed);
    assert!(r.notes.is_empty());

    let v = e.sequence().unwrap();
    // the file's 0-2, 3-6 and 7-10 remain, back to back
    let want = [(0.0, 2.0, 0.0), (2.0, 3.0, 3.0), (5.0, 3.0, 7.0)];
    assert!(same(&pieces(track(&v, "Video")), &want), "{:?}", pieces(track(&v, "Video")));
    assert!(same(&pieces(track(&v, "Dialogue")), &want), "its sound is cut identically");
    assert_eq!(track(&v, "Music"), track(&before, "Music"), "music is untouched");
    assert_valid(&e);
    assert_eq!(e.session().history.undo.len(), steps + 1, "one undo step");

    e.undo().unwrap();
    assert_eq!(e.sequence().unwrap().tracks, before.tracks, "undo restores exactly");
    e.redo().unwrap();
    assert!(same(&pieces(track(&e.sequence().unwrap(), "Video")), &want));
}

#[test]
fn a_trimmed_clip_maps_file_time_through_where_it_starts() {
    let Some((mut e, item)) = project() else { return };
    let clip = track(&e.sequence().unwrap(), "Video").clips[0].id;
    // show only the file's 1 s to 10 s
    e.trim(clip, Edge::Start, secs(1.0), TrimMode::Ripple).unwrap();
    let start = s_of(track(&e.sequence().unwrap(), "Video").clips[0].start);

    // 2 s to 3 s of the FILE is the second second of what is on the timeline
    e.apply_cuts(item, &[proposal("a", 2.0, 3.0)], CutScope::MainTracks).unwrap();
    let v = e.sequence().unwrap();
    let want = [(start, 1.0, 1.0), (start + 1.0, 7.0, 3.0)];
    assert!(same(&pieces(track(&v, "Video")), &want), "{:?}", pieces(track(&v, "Video")));
    assert_valid(&e);
}

#[test]
fn a_cut_partly_outside_the_used_part_is_clipped_and_one_wholly_outside_is_ignored() {
    let Some((mut e, item)) = project() else { return };
    let clip = track(&e.sequence().unwrap(), "Video").clips[0].id;
    e.trim(clip, Edge::Start, secs(2.0), TrimMode::Ripple).unwrap(); // file 2 s .. 10 s is on the timeline
    // 1 s to 3 s: only file 2-3 is on the timeline
    e.apply_cuts(item, &[proposal("edge", 1.0, 3.0)], CutScope::MainTracks).unwrap();
    let v = e.sequence().unwrap();
    let c = &track(&v, "Video").clips;
    assert_eq!(c.len(), 1);
    assert!((s_of(c[0].source_in) - 3.0).abs() < 0.04 && (s_of(c[0].duration) - 7.0).abs() < 0.04, "{:?}", pieces(track(&v, "Video")));

    // entirely before what is on the timeline: nothing to do, and it says so
    let steps = e.session().history.undo.len();
    let err = e.apply_cuts(item, &[proposal("gone", 0.2, 0.8)], CutScope::MainTracks).unwrap_err();
    assert!(matches!(err, EditorError::NotAvailable(m) if m.contains("None of those cuts")));
    assert_eq!(e.session().history.undo.len(), steps, "nothing changed, no undo step");
}

#[test]
fn a_file_used_twice_is_cut_in_both_places() {
    let Some((mut e, item)) = project() else { return };
    e.place(item, Lane::Main, PlaceAt::End).unwrap(); // the file again, after the first: 10-20 s
    e.apply_cuts(item, &[proposal("a", 4.0, 5.0)], CutScope::MainTracks).unwrap();
    let v = e.sequence().unwrap();
    let want = [(0.0, 4.0, 0.0), (4.0, 5.0, 5.0), (9.0, 4.0, 0.0), (13.0, 5.0, 5.0)];
    assert!(same(&pieces(track(&v, "Video")), &want), "{:?}", pieces(track(&v, "Video")));
    assert_valid(&e);
}

#[test]
fn overlapping_and_touching_cuts_become_one_and_a_cut_does_not_leave_a_sliver() {
    let Some((mut e, item)) = project() else { return };
    let r = e.apply_cuts(item, &[proposal("a", 2.0, 3.0), proposal("b", 2.5, 4.0), proposal("c", 4.0, 4.5)], CutScope::MainTracks).unwrap();
    assert_eq!(r.cuts, 1, "2-3, 2.5-4 and 4-4.5 are one piece, 2 to 4.5");
    assert!((s_of(r.removed) - 2.5).abs() < 0.04);
    let v = e.sequence().unwrap();
    assert!(same(&pieces(track(&v, "Video")), &[(0.0, 2.0, 0.0), (2.0, 5.5, 4.5)]), "{:?}", pieces(track(&v, "Video")));
    assert_valid(&e);
}

#[test]
fn the_scope_chooses_whether_music_is_cut_at_the_same_moments() {
    let Some((mut e, item)) = project() else { return };
    // main tracks only (the default): the picture and its sound are cut, the music plays on whole
    e.apply_cuts(item, &[proposal("a", 2.0, 3.0)], CutScope::MainTracks).unwrap();
    assert!(same(&pieces(track(&e.sequence().unwrap(), "Music")), &[(0.0, 10.0, 0.0)]));
    e.undo().unwrap();

    // all tracks: the music loses the same moments, so it stays lined up with the picture
    e.apply_cuts(item, &[proposal("a", 2.0, 3.0)], CutScope::AllTracks).unwrap();
    let v = e.sequence().unwrap();
    assert!(same(&pieces(track(&v, "Music")), &[(0.0, 2.0, 0.0), (2.0, 7.0, 3.0)]), "{:?}", pieces(track(&v, "Music")));
    assert!(same(&pieces(track(&v, "Video")), &[(0.0, 2.0, 0.0), (2.0, 7.0, 3.0)]));
}

#[test]
fn nothing_to_cut_and_hostile_values_are_refused_without_a_panic() {
    let Some((mut e, item)) = project() else { return };
    assert_eq!(e.apply_cuts(item, &[], CutScope::MainTracks).unwrap_err(), EditorError::NothingSelected);
    for (a, b) in [(Tick(i64::MAX), Tick(i64::MAX)), (Tick(-5), Tick(-1)), (secs(5.0), secs(5.0)), (secs(5.0), secs(4.0)), (Tick(i64::MIN), Tick(i64::MAX))] {
        let p = CutProposal { start: a, end: b, ..proposal("x", 1.0, 2.0) };
        let _ = e.apply_cuts(item, &[p], CutScope::MainTracks); // an error or a sensible cut, never a panic
    }
    e.session().active_sequence().unwrap().check().unwrap();
    let unknown = e.apply_cuts(ItemId(987_654), &[proposal("a", 1.0, 2.0)], CutScope::MainTracks).unwrap_err();
    assert!(matches!(unknown, EditorError::NotAvailable(m) if m.contains("None of those cuts")), "a file that is not on the timeline has nothing to cut");
}

#[test]
fn a_document_is_read_checked_against_the_file_and_applied() {
    let Some((mut e, item)) = project() else { return };
    let json = r#"{"schema":"simplecut.retakes/1","media":{"path":"x.mp4","duration_seconds":10.0},
        "proposals":[{"id":"s:0","start":2.0,"end":3.0,"confidence":0.9,"reason":"restart"},
                     {"id":"bad","start":3,"end":2,"confidence":0.5}]}"#;
    let a = e.read_analysis(json, item).unwrap();
    assert_eq!(a.proposals.len(), 1);
    assert_eq!(a.rejected.len(), 1);
    assert_eq!(a.proposals[0].reason, "restart");
    e.apply_cuts(item, &a.proposals, CutScope::MainTracks).unwrap();
    let v = e.sequence().unwrap();
    assert!(same(&pieces(track(&v, "Video")), &[(0.0, 2.0, 0.0), (2.0, 7.0, 3.0)]), "the checked proposal was applied: {:?}", pieces(track(&v, "Video")));

    // the same results for a 60 s video are not for this 10 s file
    let wrong = json.replace("10.0", "60.0");
    assert!(matches!(e.read_analysis(&wrong, item), Err(EditorError::Invalid(m)) if m.contains("different file")));
    assert_eq!(e.read_analysis(json, ItemId(987_654)).unwrap_err(), EditorError::NoSuchMedia);
}
