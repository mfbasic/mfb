//! bug-810: `audio::render` refuses a note longer than ten minutes up front.
//!
//! `render` looped `FOR i = 0 TO noteFrames - 1` appending two bytes a frame with
//! no bound on `noteFrames`, so `AudioNote[440.0, 9223372036854775807, …]` grew
//! past 2 GB and kept going — nothing raised and the `TRAP` never ran. It now
//! applies `audio::play`'s ten-minute limit (28,800,000 frames at 48 kHz) before
//! it renders a frame, and raises `ErrInvalidArgument`, the code `play` raises for
//! the same limit. The failure mode being fixed is "still running", so every run
//! carries a deadline and the deadline is the failure.

#[path = "../common/mod.rs"]
mod common;

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Renders one note of `$FRAMES` frames and prints `len=<n>` or
/// `raised <code>: <message>`.
const SOURCE: &str = r#"IMPORT audio
IMPORT io
IMPORT os

FUNC main() AS Nothing
  LET frames AS Integer = toInt(os::getEnvOr("FRAMES", "0"))
  LET env = audio::AudioEnvelope[0, 0, 0, 0, 32767]
  LET note = audio::AudioNote[440.0, frames, env, 0.8]
  LET tone = audio::render(note) TRAP(e)
    io::print("raised " & toString(e.code) & ": " & e.message)
    RETURN
  END TRAP
  io::print("len=" & toString(len(tone)))
END FUNC
"#;

/// `ErrInvalidArgument`.
const ERR_INVALID_ARGUMENT: &str = "77050002";
/// Ten minutes at 48 kHz, the limit `audio::play` already applies.
const LIMIT_FRAMES: i64 = 28_800_000;

struct Renderer {
    project: PathBuf,
    binary: PathBuf,
}

impl Drop for Renderer {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.project);
    }
}

fn renderer() -> Renderer {
    let project = common::temp_project("audio_render_frames", SOURCE);
    let binary = common::build_project(&project);
    Renderer { project, binary }
}

/// One run: the program's last line, failing the test if it outlives `timeout`.
fn render(r: &Renderer, frames: i64, timeout: Duration) -> String {
    let mut child = Command::new(&r.binary)
        .env("FRAMES", frames.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn renderer");
    let start = Instant::now();
    while child.try_wait().expect("try_wait").is_none() {
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "noteFrames = {frames}: still running after {timeout:?} — render did not \
                 bound the note before rendering it"
            );
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let output = child.wait_with_output().expect("collect output");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    assert!(
        output.status.success(),
        "noteFrames = {frames}: program {}:\n{stdout}\n{}",
        common::exit_description(&output.status),
        String::from_utf8_lossy(&output.stderr),
    );
    stdout.lines().last().unwrap_or_default().to_string()
}

#[test]
fn a_note_past_ten_minutes_is_refused_before_rendering() {
    let r = renderer();
    for frames in [
        i64::MAX,
        i64::MAX / 2 + 1,
        1_000_000_000_000,
        LIMIT_FRAMES + 1,
    ] {
        let line = render(&r, frames, Duration::from_secs(10));
        assert!(
            line.starts_with(&format!("raised {ERR_INVALID_ARGUMENT}:")),
            "noteFrames = {frames} must raise ErrInvalidArgument, got: {line}",
        );
    }
}

#[test]
fn a_note_within_the_limit_still_renders() {
    let r = renderer();
    // `noteFrames * 2` bytes, and empty for a non-positive count.
    for (frames, expected) in [(48_000, 96_000), (1, 2), (0, 0), (-5, 0), (i64::MIN, 0)] {
        let line = render(&r, frames, Duration::from_secs(60));
        assert_eq!(line, format!("len={expected}"), "noteFrames = {frames}");
    }
}
