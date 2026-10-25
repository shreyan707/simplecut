# SimpleCut: handoff for the next agent

Read this first. It is written so you can continue without the original conversation. Everything here was verified against the code or by running it unless marked **(unverified)**.

## 1. What this is

A **simple, dark, minimal video editor** for talking-head / YouTube videos, built **bottom-up on FilmCraft's engine** (an open-source pure-Rust Premiere-style editor). The owner's own **retake-detection pipeline** (separate Python repo, "ReTake") will later plug in as a one-click button that finds failed takes, shows confidence scores, and applies approved cuts. The architecture must keep room for that; do **not** build the pipeline itself.

Layout the owner asked for: media library left, preview centre, timeline bottom, essential controls in the toolbar.

Required features (the owner's list):
- **Projects:** new, save, reopen; persist edits and media references; clear errors for missing or inaccessible media; stay compatible with FilmCraft's `.fcproj` where possible.
- **Media:** import common video and audio; show duration and metadata; add to timeline; preview before placing.
- **Timeline:** place, move, split at playhead, trim, delete, undo/redo; preview and playhead stay in sync.
- **Audio:** background music, clip volume, mute tracks, keep A/V sync, no gaps or unintended overlaps.
- **Export:** MP4, H.264 + AAC, sensible defaults, user-chosen path, progress, cancel, clear failures, **verify the file plays**.

Captions are **not** in the MVP (the engine supports them; see section 9).

## 2. Where things are

| What | Path |
|---|---|
| This project (own git repo, own Cargo workspace) | `Z:\simplecut` |
| FilmCraft (dependency, **do not modify**, clean on `main`) | `Z:\filmcraft`, built against commit `a504f3c5` |
| Owner's retake pipeline walkthrough (owner's file, untracked in FilmCraft, leave it) | `Z:\filmcraft\repository_walkthrough.md` |
| Original plan (outside the repo, may be gone; its content is summarised here) | `C:\Users\Arnav\.claude\plans\i-want-to-build-swift-moth.md` |

FilmCraft's crates are used by **path dependency** (`../filmcraft/crates/...` in `Z:\simplecut\Cargo.toml`). Reused layers: L0-L4 (codecs, project format, edit algebra, renderer, exporter, engine `Session`). New: the front end only.

```
Z:\simplecut
  Cargo.toml            workspace: lints, FilmCraft path deps, repeated dev-profile opt levels (REQUIRED, see gotchas)
  clippy.toml  rustfmt.toml (max_width 160, copied from FilmCraft)
  crates/core/          simplecut-core: headless, NO egui. Editor, Snapshot, Player, PreviewRenderer, Thumbnails
  crates/ui/            simplecut-ui: egui window (app.rs, toolbar, library, preview, timeline, tl_math, ...)
  apps/simplecut/       the exe: eframe (glow), cpal sound card, rfd file dialogs
  HANDOFF.md            this file
```

## 3. How to build, test, run (Windows, important)

- **Use PowerShell**, not Git Bash, for cargo. Git Bash's `/usr/bin/link` shadows Microsoft's `link.exe` and every build fails with "link: extra operand". Put cargo on PATH: `$env:Path += ";$env:USERPROFILE\.cargo\bin"`.
- Visual Studio 2019 Build Tools with the "Desktop development with C++" workload is installed (the owner did this; it was missing at first).
- Toolchain: Rust 1.99, edition 2024.
- **ffmpeg and ffprobe** are at `C:\ProgramData\chocolatey\bin` (on PATH). Tests that need them skip with a message when absent.
- Gates (all must pass before every commit):
  ```
  cargo fmt --check
  cargo clippy --workspace --all-targets -- -D warnings
  cargo test --workspace            # 72 tests at the end of step 4
  ```
- Run the app: `cargo run -p simplecut` (a window opens; takes an optional project path argument).
- When running cargo from a tool, write output to a file and read it (`cargo test ... *> t.log`), and read the **real** exit code and `test result` lines. An earlier mistake: piping through `tail` hid a failed build as "exit 0".
- Very large heredocs in the Bash tool failed once; writing big files with the Write tool worked.

## 4. Working agreements with the owner

- **Commit after the gates pass**, one commit per step, with a descriptive message ending with the attribution line the environment requires (so far: `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`). The owner made the first commit themselves.
- Be honest about what is unverified (e.g. real sound through speakers was never tested by hand; only a fake card).
- Keep replies short and plain: outcome first, no jargon. The owner has limited token budget; prefer fewer, larger steps.
- Code rules ("never crash", copied from FilmCraft): no `unwrap`/`expect`/`panic` in product code (clippy denies them; tests are exempt via `clippy.toml`, but **helper functions in test files need `#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]` at the top of the file**). No `unsafe`. All time is `filmcraft_time::Tick` (254,016,000,000 per second); `f64` seconds are for display only. Hostile numbers (NaN, huge, negative) are handled (see `Editor` volume/seek checks).
- Git prints "LF will be replaced by CRLF" warnings on every commit. Harmless; a `.gitattributes` would silence them.

## 5. Architecture and the main types

**Principle:** every project change is a FilmCraft engine command (`Session::execute(id, params)`), so undo/redo and the `.fcproj` format are FilmCraft's own, and the engine checks invariants (no overlapping clips, linked clips together) after every edit. The UI never calls engine command ids; it uses `Editor`.

`simplecut-core` (all headless and tested):
- `Editor` (editor.rs): typed wrapper over `Session`. new/open/save/save_as, import, media list, `sequence()` snapshot, place/move_clip/trim/split_at_playhead/delete, select (also selects linked partners), set_clip_gain_db / set_clips_gain_db / set_track_muted / set_track_volume_db, undo/redo, `execute_raw`. Errors are `EditorError` with plain-sentence messages.
  - A new project has **one sequence: V1 "Video", A1 "Dialogue" (linked to V1), A2 "Music"**, 1920x1080, 30 fps, 48 kHz. Music has sync-lock **off** so ripple edits of the main tracks do not move it. The template is cleared from undo and marked clean.
- `Snapshot` (snapshot.rs): frozen `Arc<Project>` + media pool; targets the sequence or one media file. `render_frame`, `render_media_frame`, `mix_audio`, `duration`, `frame_rate`, `sample_rate`.
- `PreviewRenderer` (renderer.rs): worker thread, latest request wins, panic-safe.
- `Thumbnails` (thumbnails.rs): worker thread, one per file, once; audio gives `None`.
- `Player` (player.rs): mixer thread fills a ring buffer ahead of the sound card; **the count of sample frames the card actually took is the clock**; falls back to wall clock with no/broken/silent card (2 s patience); resamples to the card's rate; restarts on edits during playback (detects `session.revision` change); stops at the end. `play`, `play_media(item, from)`, `pause`, `toggle`, `seek`, `seek_media`, `tick(&mut Editor) -> PlayState`, `position()`. The card is behind `trait AudioOutput { fn start(&mut self, fill: FillFn) -> Result<u32,String>; fn stop(&mut self); }` (not `Send`; cpal streams are not).

`simplecut-ui`:
- `SimpleCutApp` (app.rs) implements `eframe::App` (egui 0.36 uses `fn ui(&mut self, ui, frame)`). **Everything the user does is an `Action`**, carried out by `perform()`. Panels only draw and `queue()` actions; the queue is drained at the end of each frame. Errors become a banner in the status bar. Tests drive the same path.
- Panels: toolbar.rs, library.rs, preview.rs (picture, seek bar, transport, "Add to timeline"/"Add as music"), timeline.rs (interactions), status.rs, theme.rs, format.rs, dialogs.rs (the `Dialogs` trait so tests script file dialogs).
- `tl_math.rs`: pure timeline arithmetic (axis, zoom, scroll, snapping, `place_without_overlap`, hit-testing), with unit tests.
- Unsaved-changes protection (New/Open/close), drag-and-drop import, panic guard around the whole frame, window title with `*` when dirty.

`apps/simplecut`: `main.rs` (eframe `glow`, logging, panic logging, `filmcraft_platform::register()` for hardware decoding: "Media Foundation" available on the owner's machine), `audio.rs` (cpal; all common sample formats; panic-safe callback), `dialogs.rs` (rfd).

## 6. Engine facts you will need (verified; several differ from the engine's own docs)

Commands used and their real behaviour:
- `file.newProject {name}`, `file.newSequence {name,width,height,fps,sampleRate,video,audio,fromItem?}` (with `fromItem` it also places the clip), `timeline.setTrack {track:"V1"|id, name?, syncLock?, muted?, volumeDb?}`.
- `file.import {paths}` returns `{items:[ids], errors:[strings], ...}`; fails only if nothing imported.
- `timeline.place {item, track:"V1", audioTrack:"A1", time:ticks, audio:bool}`; audio-only/music: `{track:"A2", video:false}`. Returns `{clips:[ids]}`.
- `timeline.move {moves:[{clip, track:id, time}], linked:true}`: **`track` is required** although the docs say optional.
- `timeline.trim {clip, edge:"in"|"out", mode:"regular"|"ripple", delta:ticks}`: negative delta shortens the end; linked sound is trimmed too.
- `timeline.razor {time, clip}`: also cuts the clip's linked partners. `edit.rippleDelete {clips}`: also removes partners and closes the gap.
- `timeline.select {clips:[ids]}`; `clip.audioGain {clips, mode:"set", db}`.
- `file.save`, `file.saveAs {path}`, `file.open {path}`. `Session.saved_revision`, `Session.history`, `Session.revision`, `Session.path` are public fields.
- `set_playhead` snaps to frames and does **not** bump `revision`.
- Media items: `ItemKind::Media(MediaClip{ media: MediaRef::File{path}, info: MediaInfo{duration, video, audio_streams, ...}, offline, ... })`. Offline detection in `Editor::media_entry_of` combines `mc.offline`, `media.offline_status(item)`, and "file does not exist".
- Rendering: `filmcraft_render::render_sequence(project, seq_id, t, RenderOptions{scale,captions:true}, &provider).over_black_rgba8()`; audio `mix_sequence(project, seq, start_sample, frames, &provider)`; provider from `session.media.provider(project, services)`. A source may return a larger frame than the requested scale.

**Export (step 5), from FilmCraft's code, not yet used by SimpleCut:**
- `file.exportMedia {path, preset:"YouTube 1080p Full HD", wait:false|true, range?, burnCaptions?, hardwareEncoding:"off|auto", ...}`. Preset is defined in FilmCraft `crates/export/src/presets.rs` (H.264 1920x1080, VBR 1-pass 16 Mbps, AAC). `wait:true` returns `{result:{...}}`; `wait:false` returns `{job:id, poll:"jobs.list", cancel:"jobs.cancel"}`.
- Jobs: `session.jobs: Vec<Job{id,label,progress:Arc<Progress>,result:Arc<Mutex<Option<Result<Report,String>>>>}>`. `Progress` has `fraction()`, `eta()`, `status: Mutex<String>`, `error: Mutex<Option<String>>`, `finished: AtomicBool`, `cancel: AtomicBool` (set true to cancel). Software H.264 encode is about 10-12 fps at 1080p (FilmCraft `docs/performance.md`); NVENC hardware encoding exists on Windows and is opt-in.
- FilmCraft's own UI calls `Session::poll_persistence()` every frame (autosave and finished-job handling). **SimpleCut does not call it yet. Verify what needs it, and call it each frame in step 5/6.**

## 7. Gotchas discovered (save yourself the time)

- **Profiles:** Cargo ignores a dependency workspace's `[profile]`. `Z:\simplecut\Cargo.toml` repeats FilmCraft's dev opt-levels (dependencies at 2, codecs at 3). Without them debug builds of the decoders are unusably slow. Keep them.
- **egui 0.36 API:** `egui::Panel::top/bottom/left(id).show(ui, ..)` (not `show_inside`); `CentralPanel::default().show(ui, ..)`; `eframe::App::ui`; `DroppedFile::path()` is a method; `Event::MouseWheel` has a `phase` field; Ctrl+wheel arrives as `input.zoom_delta()`, not as scroll; `ctx.global_style_mut`; `ctx.egui_wants_keyboard_input()`; `Response::drag_stopped()`.
- **egui_kittest 0.36 quirks:** needs `eframe` with a renderer feature (`glow`) as a dev-dependency; `Node::accesskit_node()` needs the `NodeT` trait imported; hold modifiers with `h.event(Event::ModifiersChanged(m))` across frames (the `event_modifiers` helper resets them within the same frame); use `Modifiers::COMMAND` (not `CTRL`) for Ctrl on Windows; a popup `Window` lays itself out one frame late, so `step()` before clicking it (the shared `click` helper does); labels must be unique (`get_by_label` panics on duplicates, so buttons were renamed: "Add media", "Save changes", "Discard changes", "Zoom in/out", "Fit", "Mute <track>", "Clip volume", "Timeline lanes").
- **Tests run in parallel in one process:** `scratch()` directories must be unique per call (they are, via a counter).
- Thumbnail size from `render_item` was 80x60 for a 320x240 source at scale 0.2: assert shape, not exact size.
- Sequence is fixed 1080p30. A 25 or 29.97 fps clip is converted by the engine. Matching the sequence to the first clip is a possible later improvement.

## 8. Status

Commits in `Z:\simplecut` (oldest first): `intial commit` (owner), step 2, 3a, 3b, 4. Steps 0 and 1 were committed by the owner in the first commit.

| Step | Status |
|---|---|
| 0 Spike: FilmCraft engine via path deps, import, split, export H.264+AAC verified with ffprobe | done |
| 1 `Editor` facade and tests | done |
| 2 Preview renderer, sound-driven player | done |
| 3 App shell: window, library, preview, read-only timeline, toolbar, real sound card, dialogs | done |
| 4 Timeline editing with the mouse, mute/volume, zoom | done |
| 5 Export | done |
| 6 Robustness | done |
| **7 Retake seam** | **next** |
| 8 Real sidecar and packaging | todo (depends on the owner's Python wrapper) |

**Verified by tests (90):** editing invariants (no overlaps, picture and sound stay together), save/reopen, offline media, MP3 music, playback clock, 44.1 kHz resampling, edits during playback, media preview, thumbnails, timeline interactions, full export flow (headless & UI dialog, monotonic progress, cancel, demux verification with FilmCraft, ffprobe stream validation, ffmpeg frame decoding, overwrite confirmation, open folder action, editing input lock during export), autosave persistence, crash-recovery journal restore and discard, offline media relinking via preview and library, keyboard shortcuts modal and F1 toggle, panic handling, and crash log writing.
**Verified by hand once:** the exe launches, stays responsive, finds hardware decoding.
**Never verified:** audible sound through real speakers; how the UI looks (no screenshot was taken); preview frame rate on real 1080p footage; long projects.

## 9. What to do next

### Step 5: Export (medium)
Toolbar "Export" button (currently disabled, tooltip says "next update") opens a dialog:
- Output path via a new `Dialogs::pick_export_path(suggested)` method (add it to the trait, `NoDialogs`, the test `Script`, and rfd). Default file name from the project name; `.mp4`.
- Defaults: preset "YouTube 1080p Full HD", audio on. Optional "Use graphics card if available" mapping to `hardwareEncoding:"auto"`.
- Run `file.exportMedia {..., wait:false}`; poll the `Job` each frame for progress/ETA/status; Cancel sets `progress.cancel`; failures show `progress.error` clearly. Keep the window usable but block editing during export or export a snapshot (the engine exports from the project snapshot at start; check).
- **Verify the result:** after success, open the output with FilmCraft's demuxer (import it into a scratch `Editor`/`Session` and read `MediaEntry`) and check: has a video and an audio stream, duration within one frame of the sequence, 1920x1080. In tests also use ffprobe (`codec_name` h264/aac) and decode first/last frame. Show "Export finished" with the path and an "Open folder" action.
- Empty timeline: refuse with a clear message. Overwriting an existing file: confirm.
- Put the export logic in `simplecut-core` (`ExportJob` type, headless-testable) and only the dialog in the UI. Tests: export a short timeline, cancel mid-way (use a longer clip; expect no output file or a clean error), failure on a bad path (read-only folder), progress monotonic.

### Step 6: Robustness (small to medium)
- Call `session.poll_persistence()` each frame; enable and test autosave/recovery (FilmCraft has `autosave` with a recovery journal; see `crates/engine/src/autosave.rs`, `Session::shutdown()` on exit, `prefs_path`).
- Relink flow for offline media (`media.relink` / `media.autoRelink` engine commands) with a "Relink" button in the library and preview.
- `catch_unwind` around background work is in place for render/mix/thumbnails; add it around export, and a crash log file next to the project data.
- Polish: first-run hint, keyboard shortcut list, better empty states, `.gitattributes`, window icon.

### Step 7: Retake seam (medium; build with a FAKE pipeline first)
New crate `crates/retake` (or module in core) with:
```
trait Analyzer { fn analyze(&self, req: AnalysisRequest) -> Box<dyn AnalysisJob>; }
AnalysisRequest { media: Vec<{item, path, used_range}>, options }
AnalysisJob::poll() -> Running{progress,status} | Done(Vec<CutProposal>) | Failed(String); cancel()
CutProposal { id, media_item, start, end /*Ticks in MEDIA time*/, confidence: f32 /*0..1*/, label: String /*high|medium|low or needs-review*/,
              reasoning: String, kept_text: String, removed_text: String }
```
- `FakeAnalyzer` returns canned proposals so the review UI, markers, apply and undo are built and tested **before** Python exists.
- UI: the toolbar button "Remove Retakes" is hidden until an analyzer is registered; the empty right-hand space (add a right column) becomes the review list (approve/reject checkbox, confidence badge, reasoning, "go to"), and proposals are drawn on the timeline as ranges. Nothing is cut without approval.
- **Apply = one undo step, using public engine APIs (no FilmCraft patch):** `session.edit_sequence("Remove Retakes", |seq, ctx, _| { for each range, latest first: filmcraft_edit::extract(seq, &tracks, range, ctx) })`. `extract(seq, tracks, range, ctx)` is in `filmcraft-edit` (`crates/edit/src/lib.rs`); `Session::edit_sequence` is `pub`. Choose `tracks` (Video + Dialogue by default; Music left alone; `TrackId`s from `Editor::sequence()`). FilmCraft's `edit::transcript::ripple_delete_ranges` does the same on **all unlocked tracks**, which would cut the music, so do not use it unmodified. Pattern to copy: `apply_results` in FilmCraft `crates/engine/src/scene_detect.rs`.
- Proposals are in **media time**; convert to timeline time per clip with `TrackItem::source_in/start/speed` (FilmCraft `crates/engine/src/sequence_tools.rs::timeline_time_of`); drop or clip proposals outside the used range of a clip.
- Tests: apply on trimmed and sped-up clips; linked audio stays in sync; music untouched; undo restores exactly; redo; hostile proposals (reversed, NaN, huge, overlapping) are rejected or merged, never panic.

### Step 8: Real pipeline (depends on the owner)
The owner's pipeline is Python (`repository_walkthrough.md`): WhisperX transcribe, unit splitting, lexical and semantic scoring, span clustering, Gemini adjudication, cut list. Decision already made: **not a web app**; run it as a **local sidecar service** on `127.0.0.1` so video never leaves the machine (a remote GPU worker could later receive only extracted audio, ~2 MB/min).
- Versioned contract: `POST /v1/analyze {media_path, language?, options}` -> `{job_id}`; `GET /v1/jobs/{id}` -> state/progress/status; `DELETE /v1/jobs/{id}` cancels; `GET /v1/jobs/{id}/result` -> `{transcript, proposals[]}` (seconds in JSON, converted to Ticks at the boundary). Rust client behind an HTTP library in a non-UI crate (FilmCraft has no HTTP client in the engine except an optional downloader).
- Skip the pipeline's own FFmpeg editing step: SimpleCut applies the cuts.
- Preconditions in the owner's repo: fix `pipeline/run_pipeline.py` (wrong keyword arguments to `build_retake_spans` / `run_stage6`, so Stage 5 raises `TypeError`; also `adjudicated_count` is always 0); expose a function returning transcript + cut list without FFmpeg, with progress callbacks and a cancel flag; return per-span confidence and reasoning.
- Optional reuse: load the pipeline's word-level transcript with FilmCraft's `transcript.set` (JSON `{language, speakers, words:[{text,start,end,speaker}]}`) and build captions with `transcript.createCaptions`; captions are then cut and shifted with the video by the same ripple edit. The pipeline was evaluated on WhisperX transcripts; changing the speech recogniser changes its behaviour, so re-evaluate before switching.
- WhisperX needs an NVIDIA GPU (CUDA); the owner has not said what hardware they have **(ask)**. Packaging a Python environment next to the Rust app is the heaviest part; start by pointing the app at a configured venv path.

## 10. Known limitations and risks

- Export speed: software H.264 is ~10-12 fps at 1080p (a 10-minute video takes roughly 25 minutes); offer the NVENC option.
- Preview uses the CPU compositor at 1/4 to full scale chosen from the panel width; speed on real 1080p footage is unmeasured. If it stutters, adopt FilmCraft's GPU path (`filmcraft-gpu`, `render::plan`).
- 10-bit / 4:2:2 camera H.264 may not decode on Windows without hardware support (FilmCraft gap G3); phone and webcam footage is fine.
- FilmCraft moves fast. Bump the pinned commit deliberately and re-run all gates. `Z:\filmcraft` must stay checked out at (or compatible with) `a504f3c5` for these path dependencies to compile.
- The `.fcproj` format is FilmCraft's. SimpleCut writes no private fields, so FilmCraft opens its projects and vice versa.
- The timeline has three fixed tracks; no track add/remove, no transitions, no effects, no captions UI.

## 11. Quick orientation for a first session

1. `cd Z:\simplecut`, run the three gates in PowerShell and confirm 72 tests pass.
2. `cargo run -p simplecut`, import a clip and a music file, add both, split, drag, trim, play.
3. Read `crates/ui/src/app.rs` (Actions and `perform`) and `crates/core/src/editor.rs`; they are the spine.
4. Start step 5 as described above. Add tests in the same style as `crates/ui/tests/timeline_edit.rs` (real events, real ffmpeg media) and `crates/core/tests/*.rs` (headless).
