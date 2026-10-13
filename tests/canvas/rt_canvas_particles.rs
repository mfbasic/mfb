//! `canvas::ParticleSystem` (plan-150-A): one scene item drawn as many.
//!
//! The first cases here are the ones the type itself has to pass before anything is
//! drawn from it: a scene holding a system is presented, copied, compared and dropped
//! like any other, and two systems that differ only inside their template are two
//! different scenes.
//!
//! The numbers are read off `MFB_CANVAS_STATS` (one line per frame the graphics thread
//! finished) and the `--debug` report a program prints as it exits, which carries the
//! worker arena's live byte count.

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

/// What one headless, synchronous run of a `--debug --app` program left behind.
struct Run {
    /// One `MFB_CANVAS_STATS` line per frame.
    frames: Vec<String>,
    /// The program's standard output.
    stdout: String,
    /// Its standard error, where a `--debug` build writes its report as it exits.
    report: String,
}

fn run(name: &str, source: &str, env: &[(&str, &str)]) -> Run {
    let project = common::temp_project(name, source);
    let binary = common::build_app_debug(&project, name);
    let stats = project.join("stats.txt");
    let mut command = Command::new(&binary);
    command
        .current_dir(&project)
        .env("MFB_HEADLESS", "1")
        .env("MFB_CANVAS_STATS", &stats)
        // Synchronous, or presents coalesce and the frame count is the scheduler's
        // answer rather than the program's.
        .env("MFB_CANVAS_SYNC", "1");
    for (key, value) in env {
        command.env(key, value);
    }
    let out = command
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", binary.display()));
    assert!(
        out.status.success(),
        "program {}:\n{}\n{}",
        common::exit_description(&out.status),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let frames = std::fs::read_to_string(&stats)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect();
    let _ = std::fs::remove_dir_all(&project);
    Run {
        frames,
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        report: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// The integer after `key ` on its own line of the `--debug` report.
fn report(text: &str, key: &str) -> i64 {
    text
        .lines()
        .find_map(|line| line.strip_prefix(key)?.trim().parse().ok())
        .unwrap_or_else(|| panic!("no `{key}` in the debug report:\n{text}"))
}

/// Two systems that differ only in their template's radius, presented `LOOPS` times
/// as: the first, the first again, the second.
const TEMPLATES: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color
IMPORT io
IMPORT os

FUNC dot(radius AS Float) AS canvas::AutoShape
  LET c AS canvas::AutoShape = canvas::Circle[id := "", position := vector::Float2[x := 0.0, y := 0.0], radius := radius, paint := canvas::fill(color::rgb(255, 255, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  RETURN c
END FUNC

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  LET a AS canvas::DrawItem = canvas::ParticleSystem[item := dot(5.0)]
  LET b AS canvas::DrawItem = canvas::ParticleSystem[item := dot(6.0)]
  LET loops AS Integer = toInt(os::getEnvOr("LOOPS", "1"))
  MUT i AS Integer = 0
  WHILE i < loops
    canvas::present([a])
    canvas::present([a])
    canvas::present([b])
    i = i + 1
  END WHILE
  io::print("rendered")
END FUNC
"#;

/// A scene holding a system survives `present`: it is copied into the scene ring,
/// compared against the installed scene, retired and freed, a few hundred times, and the
/// program exits cleanly.
///
/// Of each three presents one is identical to the scene installed and is skipped, and
/// two are not — the second differs from the first only INSIDE the template, so this
/// is also the proof that the frame skip and the item hash both see through `item`.
#[test]
fn a_scene_holding_a_system_is_presented_copied_and_dropped() {
    let run = run("canvas_particles_present", TEMPLATES, &[("LOOPS", "100")]);
    assert!(run.stdout.contains("rendered"));
    assert_eq!(
        run.frames.len(),
        200,
        "100 rounds of (a, a, b) are 200 scenes that differ from the one installed"
    );
}

/// Presenting systems does not leak the scene copies: the worker's arena holds the same
/// number of live bytes after 120 rounds as after 20.
#[test]
fn presenting_a_system_leaks_nothing() {
    let few = run("canvas_particles_leak_few", TEMPLATES, &[("LOOPS", "20")]);
    let many = run("canvas_particles_leak_many", TEMPLATES, &[("LOOPS", "120")]);
    assert_eq!(
        report(&few.report, "arena.0.live_bytes"),
        report(&many.report, "arena.0.live_bytes"),
        "the worker arena grew with the number of presents"
    );
    assert_eq!(report(&many.report, "arena.0.double_free_skips"), 0);
}
