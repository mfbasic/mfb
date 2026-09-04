//! `audio::play`'s MML sequencer is bounded (bug-509, DEC-55).
//!
//! A `{ … }<count>` repeat had a floor (`count >= 1`) and no ceiling, and repeats nest
//! multiplicatively, so fifteen characters of tune expanded to 200,000 notes and thirty
//! to 64^4 — 38 GB and a process killed before any raise could fire. The expander now
//! refuses a tune past 65,536 tokens *before* building it, and the synthesiser refuses
//! a track past ten minutes of audio before rendering a frame of it. Both are
//! `ErrInvalidArgument`, the code `play` already raises for malformed MML, and both
//! are checked from the counts alone, so the cost of refusing is the cost of parsing.
//!
//! These run a real program against the default output device, because `play` takes
//! an open stream and synthesises before it writes. A host with no device is a real
//! configuration, not a failure: the program prints that and the test skips. The
//! bombs' failure mode is "still running", so every run carries a deadline and the
//! deadline is the failure.

mod common;

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Plays `$TUNE` and prints one line: `played`, `raised <code>: <message>`, or
/// `no output device: …` when there is nothing to open.
const SOURCE: &str = r#"IMPORT audio
IMPORT io
IMPORT os

FUNC main() AS Integer
  LET tune AS String = os::getEnvOr("TUNE", "")
  RES out AS audio::AudioOutput = audio::openOutput(48000, 1, 512) TRAP(e)
    io::print("no output device: " & e.message)
    RETURN 0
  END TRAP
  audio::play(out, tune) TRAP(e)
    io::print("raised " & toString(e.code) & ": " & e.message)
    audio::close(out)
    RETURN 0
  END TRAP
  io::print("played")
  audio::close(out)
  RETURN 0
END FUNC
"#;

/// `ErrInvalidArgument`, the code `audio::play` raises for MML it refuses.
const ERR_INVALID_ARGUMENT: &str = "77050002";

struct Player {
    project: PathBuf,
    binary: PathBuf,
}

impl Drop for Player {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.project);
    }
}

fn player(name: &str) -> Player {
    let project = common::temp_project(name, SOURCE);
    let binary = common::build_project(&project);
    Player { project, binary }
}

/// One run: the program's line and how long it took. `None` when the host has no
/// output device, which the caller treats as a skip.
fn play(binary: &Path, tune: &str, timeout: Duration) -> Option<(String, Duration)> {
    let mut child = Command::new(binary)
        .current_dir(binary.parent().expect("binary has a directory"))
        .env("TUNE", tune)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| panic!("run {}: {e}", binary.display()));
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            break status;
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "`{tune}`: still running after {timeout:?} — an MML expansion or render \
                 the sequencer did not bound"
            );
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    let elapsed = start.elapsed();
    let mut stdout = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        pipe.read_to_string(&mut stdout).ok();
    }
    assert!(
        status.success(),
        "`{tune}`: program {}:\n{stdout}",
        common::exit_description(&status),
    );
    let line = stdout.lines().last().unwrap_or_default().to_string();
    if line.starts_with("no output device") {
        eprintln!("skipping: {line}");
        return None;
    }
    Some((line, elapsed))
}

fn assert_refused(line: &str, tune: &str) {
    assert!(
        line.starts_with(&format!("raised {ERR_INVALID_ARGUMENT}:")),
        "`{tune}` must be refused as ErrInvalidArgument, got: {line}",
    );
}

#[test]
fn nested_repeats_are_refused_by_their_product() {
    // Thirty characters, a billion notes. Before the cap the expander tried to build
    // the token list — 20 GB in twelve seconds, and climbing.
    let p = player("audio_mml_nested_bomb");
    let tune = "{ { { C }1000 }1000 }1000";
    if let Some((line, _)) = play(&p.binary, tune, Duration::from_secs(20)) {
        assert_refused(&line, tune);
    }
}

#[test]
fn a_flat_repeat_past_the_token_cap_is_refused() {
    // The DEC-55 spike: fifteen characters, 200,000 quarter notes, 38 GB.
    let p = player("audio_mml_flat_bomb");
    let tune = "{ C }200000";
    if let Some((line, _)) = play(&p.binary, tune, Duration::from_secs(20)) {
        assert_refused(&line, tune);
    }
}

#[test]
fn a_track_longer_than_the_render_limit_is_refused_before_rendering() {
    // Under the token cap and still a bomb: 5,000 whole notes at the slowest tempo is
    // ten and a half hours of audio, 1.8 billion samples to synthesise and hold.
    let p = player("audio_mml_length_bomb");
    let tune = "T32 L1 { C }5000";
    if let Some((line, _)) = play(&p.binary, tune, Duration::from_secs(20)) {
        assert_refused(&line, tune);
    }
}

#[test]
fn ordinary_repeats_still_expand_to_the_same_notes() {
    // The caps must not change what an ordinary tune plays. `play` writes in real time,
    // so the tune's length is observable as wall time: 64 sixty-fourth notes at T255
    // are 64 x 705 frames = 0.94 s, and a nested `{ { C }8 }8` must take at least that
    // long — an expander that dropped or mis-nested a repeat would finish early.
    let p = player("audio_mml_ordinary");
    let tune = "T255 L64 { { C }8 }8";
    if let Some((line, elapsed)) = play(&p.binary, tune, Duration::from_secs(30)) {
        assert_eq!(line, "played", "`{tune}` must still play");
        assert!(
            elapsed >= Duration::from_millis(800),
            "64 nested notes played in {elapsed:?} — fewer notes than the tune has",
        );
    }
    for tune in [
        "T255 L64 { C D }4 { E }1 F",
        "T255 L64 { C }1",
        "T255 L64 C D E",
    ] {
        if let Some((line, _)) = play(&p.binary, tune, Duration::from_secs(30)) {
            assert_eq!(line, "played", "`{tune}` must still play");
        }
    }
    // The floor is unchanged: a count below one was refused before and still is.
    let tune = "{ C }0";
    if let Some((line, _)) = play(&p.binary, tune, Duration::from_secs(30)) {
        assert_refused(&line, tune);
    }
}
