//! Shared test fixtures: small media files made with ffmpeg (a test-only oracle, never linked).
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;

pub fn have(tool: &str) -> bool {
    Command::new(tool).arg("-version").output().map(|o| o.status.success()).unwrap_or(false)
}

/// A scratch directory under cargo's target tmp, unique per test and process.
pub fn scratch(name: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("scratch dir");
    d
}

/// Whole seconds of 320x240 colour bars at 25 fps, with a 440 Hz tone when `audio`. H.264 + AAC.
pub fn make_clip(dir: &Path, file: &str, secs: u32, audio: bool) -> PathBuf {
    let out = dir.join(file);
    let mut c = Command::new("ffmpeg");
    c.args(["-y", "-v", "error", "-f", "lavfi", "-i"]).arg(format!("testsrc=size=320x240:rate=25:duration={secs}"));
    if audio {
        c.args(["-f", "lavfi", "-i"]).arg(format!("sine=frequency=440:sample_rate=48000:duration={secs}"));
    }
    c.args(["-c:v", "libx264", "-pix_fmt", "yuv420p"]);
    if audio {
        c.args(["-c:a", "aac", "-shortest"]);
    }
    let status = c.arg(&out).status().expect("run ffmpeg");
    assert!(status.success(), "ffmpeg could not make {file}");
    out
}

/// Stereo 48 kHz tone as audio-only `file` (the extension picks the format). None when this
/// ffmpeg cannot encode that format.
pub fn make_music(dir: &Path, file: &str, secs: u32) -> Option<PathBuf> {
    let out = dir.join(file);
    let status = Command::new("ffmpeg")
        .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
        .arg(format!("sine=frequency=220:sample_rate=48000:duration={secs}"))
        .args(["-ac", "2"])
        .arg(&out)
        .status()
        .ok()?;
    status.success().then_some(out)
}
