//! The timeline: a ruler, one lane per track, clips, and the playhead. Click or drag to move the
//! playhead. (Editing on the timeline itself comes in the next step.)

use egui::{Align2, CornerRadius, FontId, Rect, Sense, Shape, Stroke, StrokeKind, WidgetInfo, WidgetType, pos2, vec2};
use simplecut_core::{Tick, TrackKind};

use crate::app::{Action, SimpleCutApp};
use crate::format;
use crate::theme;

const HEADER_W: f32 = 104.0;
const RULER_H: f32 = 26.0;
const LANE_H: f32 = 48.0;

pub(crate) fn show(app: &mut SimpleCutApp, ui: &mut egui::Ui) {
    let t = theme::tokens();
    let Some(seq) = app.editor.sequence() else {
        ui.label("No project open.");
        return;
    };
    ui.horizontal(|ui| {
        ui.heading("Timeline");
        ui.label(egui::RichText::new(format::timecode(seq.playhead, seq.fps)).monospace().color(t.text_dim));
    });

    let height = RULER_H + LANE_H * seq.tracks.len() as f32;
    let (resp, painter) = ui.allocate_painter(vec2(ui.available_width(), height), Sense::click_and_drag());
    resp.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, "Timeline lanes"));
    let rect = resp.rect;
    let lanes_left = rect.left() + HEADER_W;
    let lanes_w = (rect.right() - lanes_left - 10.0).max(50.0);
    let span = (seq.duration.0 as f64 / 254_016_000_000.0).max(10.0) * 1.04;
    let px_per_sec = f64::from(lanes_w) / span;
    let x_of = |t: Tick| lanes_left + (t.0.max(0) as f64 / 254_016_000_000.0 * px_per_sec) as f32;

    // ruler
    painter.rect_filled(Rect::from_min_max(pos2(lanes_left, rect.top()), pos2(rect.right(), rect.top() + RULER_H)), CornerRadius::ZERO, t.panel);
    let step = [1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0, 1800.0, 3600.0].into_iter().find(|s| s * px_per_sec >= 70.0).unwrap_or(3600.0);
    let mut s = 0.0;
    while s <= span {
        let x = lanes_left + (s * px_per_sec) as f32;
        painter.line_segment([pos2(x, rect.top() + RULER_H - 7.0), pos2(x, rect.top() + RULER_H)], Stroke::new(1.0, t.text_dim));
        painter.text(pos2(x + 3.0, rect.top() + 5.0), Align2::LEFT_TOP, format::length(Tick::from_seconds_f64(s)), FontId::proportional(11.0), t.text_dim);
        s += step;
    }

    // lanes
    for (i, track) in seq.tracks.iter().enumerate() {
        let top = rect.top() + RULER_H + i as f32 * LANE_H;
        let lane = Rect::from_min_max(pos2(lanes_left, top + 2.0), pos2(rect.right(), top + LANE_H - 2.0));
        painter.rect_filled(
            Rect::from_min_max(pos2(rect.left(), top + 2.0), pos2(lanes_left - 4.0, top + LANE_H - 2.0)),
            CornerRadius::same(4),
            t.panel_raised,
        );
        painter.text(pos2(rect.left() + 10.0, top + LANE_H / 2.0 - 7.0), Align2::LEFT_CENTER, &track.name, FontId::proportional(13.0), t.text);
        if track.muted {
            painter.text(pos2(rect.left() + 10.0, top + LANE_H / 2.0 + 9.0), Align2::LEFT_CENTER, "muted", FontId::proportional(11.0), t.danger);
        }
        painter.rect_filled(lane, CornerRadius::same(4), t.lane);
        let color = match (track.kind, track.name.as_str()) {
            (TrackKind::Video, _) => t.video_clip,
            (_, "Music") => t.music_clip,
            _ => t.dialogue_clip,
        };
        for clip in &track.clips {
            let (x0, x1) = (x_of(clip.start), x_of(clip.end()).max(x_of(clip.start) + 2.0));
            let r = Rect::from_min_max(pos2(x0, lane.top() + 2.0), pos2(x1.min(lane.right()), lane.bottom() - 2.0));
            let fill = if track.muted || !clip.enabled { color.gamma_multiply(0.35) } else { color };
            painter.rect_filled(r, CornerRadius::same(4), fill);
            if clip.selected {
                painter.rect_stroke(r, CornerRadius::same(4), Stroke::new(2.0, egui::Color32::WHITE), StrokeKind::Inside);
            }
            painter.with_clip_rect(r.shrink2(vec2(4.0, 0.0))).text(
                pos2(r.left() + 6.0, r.center().y),
                Align2::LEFT_CENTER,
                &clip.name,
                FontId::proportional(12.0),
                egui::Color32::WHITE,
            );
        }
    }
    if seq.clip_count() == 0 {
        painter.text(
            pos2((lanes_left + rect.right()) / 2.0, rect.top() + RULER_H + LANE_H * 1.5),
            Align2::CENTER_CENTER,
            "Double-click a file in the library to add it here",
            FontId::proportional(14.0),
            t.text_dim,
        );
    }

    // playhead
    let x = x_of(seq.playhead);
    painter.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(1.5, t.playhead));
    painter.add(Shape::convex_polygon(vec![pos2(x - 6.0, rect.top()), pos2(x + 6.0, rect.top()), pos2(x, rect.top() + 9.0)], t.playhead, Stroke::NONE));

    // click or drag anywhere on the lanes to move the playhead
    if (resp.clicked() || resp.dragged() || resp.drag_started())
        && let Some(p) = resp.interact_pointer_pos()
        && p.x >= lanes_left
    {
        let secs = f64::from(p.x - lanes_left) / px_per_sec;
        let to = Tick::from_seconds_f64(secs.clamp(0.0, span));
        if to != seq.playhead {
            app.queue(Action::SeekTimeline(to));
        }
    }
}
