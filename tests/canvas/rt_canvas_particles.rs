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
    run_inner(name, source, env, false)
}

/// [`run`], with the twelve-glyph test font beside the program as `fixture.ttf`.
fn run_with_font(name: &str, source: &str, env: &[(&str, &str)]) -> Run {
    run_inner(name, source, env, true)
}

fn run_inner(name: &str, source: &str, env: &[(&str, &str)], font: bool) -> Run {
    let project = common::temp_project(name, source);
    if font {
        std::fs::write(project.join("fixture.ttf"), common::fixture_truetype())
            .expect("write the font fixture");
    }
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

FUNC system(item AS canvas::AutoShape) AS canvas::DrawItem
  MUT emission AS canvas::Emission
  MUT motion AS canvas::Motion
  MUT appearance AS canvas::Appearance
  LET emitter AS canvas::Shape = canvas::Circle[id := "", position := vector::Float2[x := 0.0, y := 0.0], radius := 0.0, paint := canvas::fill(color::rgb(0, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET ps AS canvas::DrawItem = canvas::ParticleSystem[position := vector::Float2[x := 100.0, y := 100.0], item := item, emitter := emitter, emission := WITH emission { lifetime := 1.0 }, motion := motion, appearance := appearance, seed := 1, time := 0.0]
  RETURN ps
END FUNC

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  LET a AS canvas::DrawItem = system(dot(5.0))
  LET b AS canvas::DrawItem = system(dot(6.0))
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

/// Every record of the type set is built by name, with every field written out, and a
/// system is built on an emitter of each of the ten `canvas::Shape` variants and
/// presented. Each present is a scene that differs from the last only in its emitter,
/// so ten frames also says the item hash folds the emitter.
const TYPE_SET: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color
IMPORT io

FUNC origin() AS vector::Float2
  RETURN vector::Float2[x := 0.0, y := 0.0]
END FUNC

FUNC system(emitter AS canvas::Shape) AS canvas::DrawItem
  LET white AS color::Color = color::rgb(255, 255, 255)
  LET emission AS canvas::Emission = canvas::Emission[mode := canvas::EmissionMode.Burst, maxParticles := 8, rate := 4.0, burst := 8, burstPeriod := 2.0, lifetime := 1.0, lifetimeVar := 0.25]
  LET motion AS canvas::Motion = canvas::Motion[speed := 40.0, speedVar := 0.5, angle := 0.0, spread := 3.14159, gravityX := 0.0, gravityY := 98.0, drag := 0.5, rotation := 0.0, rotationVar := 1.0, spin := 2.0, spinVar := 0.5, turbulence := 3.0, turbulenceHz := 1.5]
  LET appearance AS canvas::Appearance = canvas::Appearance[startColor := white, endColor := color::rgba(255, 255, 255, 0), startScale := 1.0, endScale := 0.5, scaleVar := 0.25]
  MUT dot AS canvas::Circle
  LET item AS canvas::AutoShape = WITH dot { radius := 3.0, paint := canvas::fill(white) }
  LET ps AS canvas::DrawItem = canvas::ParticleSystem[position := vector::Float2[x := 450.0, y := 320.0], item := item, emitter := emitter, emission := emission, motion := motion, appearance := appearance, seed := 7, time := 0.25]
  RETURN ps
END FUNC

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES fnt AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e)
    io::print("no font")
    RETURN
  END TRAP
  LET ring AS List OF vector::Float2 = [vector::Float2[x := 0.0, y := 0.0], vector::Float2[x := 40.0, y := 0.0], vector::Float2[x := 40.0, y := 30.0]]
  LET bend AS canvas::BezierCurve = canvas::BezierCurve[points := [vector::Float2[x := 0.0, y := 0.0], vector::Float2[x := 20.0, y := 30.0], vector::Float2[x := 40.0, y := 0.0]], weights := []]
  LET back AS canvas::BezierCurve = canvas::BezierCurve[points := [vector::Float2[x := 40.0, y := 0.0], vector::Float2[x := 20.0, y := 0.0 - 30.0], vector::Float2[x := 0.0, y := 0.0]], weights := []]
  MUT rectangle AS canvas::Rectangle
  MUT line AS canvas::Line
  MUT polygon AS canvas::Polygon
  MUT circle AS canvas::Circle
  MUT arc AS canvas::Arc
  MUT ellipse AS canvas::Ellipse
  MUT polyLine AS canvas::PolyLine
  MUT polyCurve AS canvas::PolyCurve
  MUT curvePolygon AS canvas::CurvePolygon
  LET e0 AS canvas::Shape = WITH rectangle { size := canvas::Size[width := 60.0, height := 40.0] }
  LET e1 AS canvas::Shape = WITH line { endAt := vector::Float2[x := 80.0, y := 0.0] }
  LET e2 AS canvas::Shape = WITH polygon { contours := [ring] }
  LET e3 AS canvas::Shape = WITH circle { radius := 25.0 }
  LET e4 AS canvas::Shape = WITH arc { radius := 30.0, endAngle := 3.14159 }
  LET e5 AS canvas::Shape = canvas::Text[id := "", position := origin(), text := "AB", font := fnt, fontSize := 24.0, paint := canvas::fill(color::rgb(0, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET e6 AS canvas::Shape = WITH ellipse { radiusX := 30.0, radiusY := 12.0 }
  LET e7 AS canvas::Shape = WITH polyLine { points := ring }
  LET e8 AS canvas::Shape = WITH polyCurve { curves := [bend] }
  LET e9 AS canvas::Shape = WITH curvePolygon { contours := [[bend, back]] }
  canvas::present([system(e0)])
  canvas::present([system(e1)])
  canvas::present([system(e2)])
  canvas::present([system(e3)])
  canvas::present([system(e4)])
  canvas::present([system(e5)])
  canvas::present([system(e6)])
  canvas::present([system(e7)])
  canvas::present([system(e8)])
  canvas::present([system(e9)])
  io::print("built")
END FUNC
"#;

#[test]
fn every_record_is_built_and_every_shape_is_an_emitter() {
    let run = run_with_font("canvas_particles_types", TYPE_SET, &[]);
    assert!(run.stdout.contains("built"), "{}", run.stdout);
    assert_eq!(run.frames.len(), 10, "one frame per emitter shape");
}
