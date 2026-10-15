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

use common::canvas_image::{compare_exact, compare_within_tolerance, Frame, Tolerance};
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
    text.lines()
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
  LET e5 AS canvas::Shape = canvas::Text[id := "", position := origin(), text := "AB", fonts := [fnt], fontSize := 24.0, paint := canvas::fill(color::rgb(0, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
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

// ---- The analytic model, as arithmetic (plan-150-A Phase 3) ---------------------------
//
// The model is MFBASIC text the compiler injects (`helper_particles.rs:MODEL`), and the
// compiler is a binary crate, so a test cannot link it. As `rt_canvas_auto_step.rs` does
// for the `AutoPaint` rules, this reads the text out of the file it is declared in,
// compiles it behind a driver as a plain console program, and runs it: what is run is
// the model character for character, with only the `__canvas_` prefix -- which user
// source may not spell -- renamed.

/// The raw string a `const NAME: &str =` in a canvas helper file holds.
fn helper_source(file: &str, opens: &str) -> String {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/codegen/builtins/canvas")
        .join(file);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {file}: {e}"));
    let start = text
        .find(opens)
        .unwrap_or_else(|| panic!("{file} does not declare `{opens}`"))
        + opens.len();
    let end = start + text[start..].find("\"#;").expect("the raw string closes");
    text[start..end]
        .replace("__canvas_", "k_")
        .replace("__CANVAS_", "K_")
}

const MODEL_DRIVER: &str = r#"
FUNC k_absF(v AS Float) AS Float
  IF v < 0.0 THEN
    RETURN 0.0 - v
  END IF
  RETURN v
END FUNC

' A Float in full: `toString` keeps two decimals, so a value is printed as the whole
' number of billionths (or, `pico`, trillionths) it holds.
FUNC nano(v AS Float) AS String
  RETURN toString(toInt(v * 1000000000.0))
END FUNC

FUNC pico(v AS Float) AS String
  RETURN toString(toInt(v * 1000000000000.0))
END FUNC

FUNC at(x AS Float, y AS Float) AS vector::Float2
  RETURN vector::Float2[x := x, y := y]
END FUNC

' The distance from the sampled point to the segment a-b.
FUNC offSegment(ax AS Float, ay AS Float, bx AS Float, by AS Float) AS Float
  LET dx AS Float = bx - ax
  LET dy AS Float = by - ay
  MUT f AS Float = ((K_PARTICLE_X - ax) * dx + (K_PARTICLE_Y - ay) * dy) / (dx * dx + dy * dy)
  IF f < 0.0 THEN
    f = 0.0
  END IF
  IF f > 1.0 THEN
    f = 1.0
  END IF
  LET ox AS Float = K_PARTICLE_X - (ax + dx * f)
  LET oy AS Float = K_PARTICLE_Y - (ay + dy * f)
  RETURN math::sqrt(ox * ox + oy * oy)
END FUNC

FUNC determinism() AS Nothing
  MUT first AS String = ""
  MUT second AS String = ""
  MUT slot AS Integer = 0
  WHILE slot < 6
    MUT stream AS Integer = 0
    WHILE stream < 4
      first = first & nano(k_particleHash(7, slot, 2, stream)) & " "
      stream = stream + 1
    END WHILE
    slot = slot + 1
  END WHILE
  slot = 0
  WHILE slot < 6
    MUT stream AS Integer = 0
    WHILE stream < 4
      second = second & nano(k_particleHash(7, slot, 2, stream)) & " "
      stream = stream + 1
    END WHILE
    slot = slot + 1
  END WHILE
  io::print("same " & toString(first = second))
  io::print("draws " & first)
  ' A different seed, slot, cycle or stream is a different draw.
  LET base AS Float = k_particleHash(7, 3, 2, 1)
  io::print("distinct " & toString(base <> k_particleHash(8, 3, 2, 1) AND base <> k_particleHash(7, 4, 2, 1) AND base <> k_particleHash(7, 3, 3, 1) AND base <> k_particleHash(7, 3, 2, 2)))
END FUNC

' Whether the particle in `slot` is alive at `time`: an age, and one under its life.
FUNC alive(burstMode AS Boolean, maxParticles AS Integer, rate AS Float, burst AS Integer, burstPeriod AS Float, slot AS Integer, time AS Float, lifetime AS Float) AS Boolean
  LET age AS Float = k_particleAge(burstMode, maxParticles, rate, burst, burstPeriod, slot, time)
  IF age < 0.0 THEN
    RETURN FALSE
  END IF
  RETURN age < k_particleLife(lifetime, 0.0, k_particleHash(1, slot, K_PARTICLE_CYCLE, 0))
END FUNC

FUNC liveness() AS Nothing
  ' Ten slots at five a second: slot 3 is born at 0.6 s and again every 2 s.
  io::print("before " & toString(alive(FALSE, 10, 5.0, 0, 0.0, 3, 0.5, 1.0)))
  io::print("young " & toString(alive(FALSE, 10, 5.0, 0, 0.0, 3, 0.7, 1.0)))
  io::print("old " & toString(alive(FALSE, 10, 5.0, 0, 0.0, 3, 2.1, 1.0)))
  io::print("reborn " & toString(alive(FALSE, 10, 5.0, 0, 0.0, 3, 2.7, 1.0)))
  io::print("cycle " & toString(K_PARTICLE_CYCLE))
  io::print("outside " & toString(alive(FALSE, 10, 5.0, 0, 0.0, 10, 5.0, 1.0)))
  io::print("norate " & toString(alive(FALSE, 10, 0.0, 0, 0.0, 0, 5.0, 1.0)))
  io::print("negative " & toString(alive(FALSE, 10, 5.0, 0, 0.0, 0, 0.0 - 1.0, 1.0)))
END FUNC

FUNC burst() AS Nothing
  io::print("t0.1 " & toString(alive(TRUE, 10, 0.0, 5, 2.0, 0, 0.1, 1.0)))
  io::print("t1.9 " & toString(alive(TRUE, 10, 0.0, 5, 2.0, 0, 1.9, 1.0)))
  io::print("t2.1 " & toString(alive(TRUE, 10, 0.0, 5, 2.0, 0, 2.1, 1.0)))
  io::print("past " & toString(alive(TRUE, 10, 0.0, 5, 2.0, 5, 0.1, 1.0)))
  ' One burst: alive once, and never again.
  io::print("once " & toString(alive(TRUE, 10, 0.0, 5, 0.0, 0, 0.5, 1.0)) & " " & toString(alive(TRUE, 10, 0.0, 5, 0.0, 0, 2.5, 1.0)))
  ' A burst larger than the pool is the pool.
  io::print("clamped " & toString(alive(TRUE, 10, 0.0, 50, 0.0, 9, 0.5, 1.0)) & " " & toString(alive(TRUE, 10, 0.0, 50, 0.0, 10, 0.5, 1.0)))
END FUNC

FUNC drag() AS Nothing
  LET free AS Float = k_particlePosition(5.0, 120.0, 98.0, 0.0, 1.0)
  LET nearly AS Float = k_particlePosition(5.0, 120.0, 98.0, 0.000001, 1.0)
  io::print("free " & nano(free))
  io::print("gap " & pico(k_absF(free - nearly)))
  ' Either side of the series' end the two forms agree.
  io::print("seam " & pico(k_absF(k_particlePosition(0.0, 120.0, 98.0, 0.2499999, 1.0) - k_particlePosition(0.0, 120.0, 98.0, 0.2500001, 1.0))))
  ' Under heavy drag a falling particle settles at g / k: 98 / 2 = 49 px/s.
  io::print("terminal " & nano(k_particlePosition(0.0, 0.0, 98.0, 2.0, 21.0) - k_particlePosition(0.0, 0.0, 98.0, 2.0, 20.0)))
  io::print("exp " & pico(k_absF(k_particleExpNeg(1.0) - 0.36787944117144233)))
END FUNC

FUNC disc() AS Nothing
  MUT inner AS Integer = 0
  MUT outside AS Integer = 0
  MUT slot AS Integer = 0
  WHILE slot < 10000
    k_particleDisc(40.0, 40.0, k_particleHash(3, slot, 0, 1), k_particleHash(3, slot, 0, 2))
    LET d2 AS Float = K_PARTICLE_X * K_PARTICLE_X + K_PARTICLE_Y * K_PARTICLE_Y
    IF d2 < 400.0 THEN
      inner = inner + 1
    END IF
    IF d2 > 1600.0000001 THEN
      outside = outside + 1
    END IF
    slot = slot + 1
  END WHILE
  io::print("inner " & toString(inner))
  io::print("outside " & toString(outside))
END FUNC

FUNC placement() AS Nothing
  ' A 40 x 20 box about its centre, a quarter turn (x, y) -> (-y, x), the emitter at
  ' (100, 50) in a system at (10, 10): the turned box is 20 wide and 40 tall about
  ' (110, 60).
  MUT inTurned AS Integer = 0
  MUT outUnturned AS Integer = 0
  MUT slot AS Integer = 0
  WHILE slot < 1000
    k_particleBox(40.0, 20.0, TRUE, k_particleHash(5, slot, 0, 1), k_particleHash(5, slot, 0, 2))
    k_particlePlace(10.0, 10.0, 100.0, 50.0, TRUE, 0.0, 1.0, 0.0 - 1.0, 0.0, 0.0, 0.0)
    LET x AS Float = K_PARTICLE_X - 110.0
    LET y AS Float = K_PARTICLE_Y - 60.0
    IF k_absF(x) <= 10.0 AND k_absF(y) <= 20.0 THEN
      inTurned = inTurned + 1
    END IF
    IF k_absF(y) > 10.0 THEN
      outUnturned = outUnturned + 1
    END IF
    slot = slot + 1
  END WHILE
  io::print("inTurned " & toString(inTurned))
  io::print("outUnturned " & toString(outUnturned))
  ' A transform that is not in effect only moves the sample.
  k_particleBox(40.0, 20.0, FALSE, 0.25, 0.5)
  k_particlePlace(10.0, 10.0, 100.0, 50.0, FALSE, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0)
  io::print("plain " & nano(K_PARTICLE_X) & " " & nano(K_PARTICLE_Y))
END FUNC

FUNC strokes() AS Nothing
  LET path AS List OF vector::Float2 = [at(0.0, 0.0), at(10.0, 0.0), at(10.0, 10.0)]
  MUT offLine AS Float = 0.0
  MUT offArc AS Float = 0.0
  MUT offOpen AS Float = 0.0
  MUT offClosed AS Float = 0.0
  MUT onLast AS Integer = 0
  MUT onClosing AS Integer = 0
  MUT slot AS Integer = 0
  WHILE slot < 2000
    LET r AS Float = k_particleHash(9, slot, 0, 1)
    k_particleSegment(1.0, 2.0, 11.0, 7.0, r)
    offLine = math::max(offLine, offSegment(1.0, 2.0, 11.0, 7.0))
    k_particleArc(30.0, 0.5, 2.5, r)
    offArc = math::max(offArc, k_absF(math::sqrt(K_PARTICLE_X * K_PARTICLE_X + K_PARTICLE_Y * K_PARTICLE_Y) - 30.0))
    ' Inside the sweep: below the centre for 0.5 to 2.5 radians, clockwise from +X.
    IF K_PARTICLE_Y < 14.0 THEN
      offArc = 1.0
    END IF
    LET found AS Boolean = k_particleAlong(path, FALSE, r)
    offOpen = math::max(offOpen, math::min(offSegment(0.0, 0.0, 10.0, 0.0), offSegment(10.0, 0.0, 10.0, 10.0)))
    IF K_PARTICLE_X = 10.0 AND K_PARTICLE_Y > 0.0 THEN
      onLast = onLast + 1
    END IF
    LET again AS Boolean = k_particleAlong(path, TRUE, r)
    offClosed = math::max(offClosed, math::min(math::min(offSegment(0.0, 0.0, 10.0, 0.0), offSegment(10.0, 0.0, 10.0, 10.0)), offSegment(10.0, 10.0, 0.0, 0.0)))
    IF K_PARTICLE_X > 0.0 AND K_PARTICLE_X < 10.0 AND K_PARTICLE_Y > 0.0 THEN
      onClosing = onClosing + 1
    END IF
    slot = slot + 1
  END WHILE
  io::print("line " & toString(offLine < 0.000000001))
  io::print("arc " & toString(offArc < 0.000000001))
  io::print("open " & toString(offOpen < 0.000000001))
  io::print("closed " & toString(offClosed < 0.000000001))
  io::print("onLast " & toString(onLast))
  io::print("onClosing " & toString(onClosing))
  LET none AS List OF vector::Float2 = []
  io::print("empty " & toString(k_particleAlong(none, FALSE, 0.5)))
END FUNC

FUNC polygon() AS Nothing
  ' An L: a 20 x 20 square with its (10..20, 10..20) corner cut out.
  LET ell AS List OF vector::Float2 = [at(0.0, 0.0), at(20.0, 0.0), at(20.0, 10.0), at(10.0, 10.0), at(10.0, 20.0), at(0.0, 20.0)]
  MUT found AS Integer = 0
  MUT notch AS Integer = 0
  MUT outside AS Integer = 0
  MUT slot AS Integer = 0
  WHILE slot < 2000
    IF k_particleInside([ell], FALSE, 11, slot, 0) THEN
      found = found + 1
      IF K_PARTICLE_X > 10.0 AND K_PARTICLE_Y > 10.0 THEN
        notch = notch + 1
      END IF
      IF K_PARTICLE_X < 0.0 OR K_PARTICLE_X > 20.0 OR K_PARTICLE_Y < 0.0 OR K_PARTICLE_Y > 20.0 THEN
        outside = outside + 1
      END IF
    END IF
    slot = slot + 1
  END WHILE
  io::print("found " & toString(found))
  io::print("notch " & toString(notch))
  io::print("outside " & toString(outside))
  ' A square with a square hole: even-odd leaves the hole empty, and non-zero fills it
  ' when both rings run the same way.
  LET outer AS List OF vector::Float2 = [at(0.0, 0.0), at(30.0, 0.0), at(30.0, 30.0), at(0.0, 30.0)]
  LET hole AS List OF vector::Float2 = [at(10.0, 10.0), at(20.0, 10.0), at(20.0, 20.0), at(10.0, 20.0)]
  io::print("hole " & toString(k_particleInRings([outer, hole], FALSE, 15.0, 15.0)) & " " & toString(k_particleInRings([outer, hole], TRUE, 15.0, 15.0)) & " " & toString(k_particleInRings([outer, hole], FALSE, 5.0, 5.0)))
  LET noRings AS List OF List OF vector::Float2 = []
  io::print("norings " & toString(k_particleInside(noRings, FALSE, 11, 0, 0)))
END FUNC

FUNC main() AS Nothing
  LET test AS String = os::getEnvOr("TEST", "")
  IF test = "determinism" THEN
    determinism()
  ELSEIF test = "liveness" THEN
    liveness()
  ELSEIF test = "burst" THEN
    burst()
  ELSEIF test = "drag" THEN
    drag()
  ELSEIF test = "disc" THEN
    disc()
  ELSEIF test = "placement" THEN
    placement()
  ELSEIF test = "strokes" THEN
    strokes()
  ELSEIF test = "polygon" THEN
    polygon()
  END IF
END FUNC
"#;

/// The model and its driver, built once for every test below.
fn model() -> &'static std::path::PathBuf {
    static BUILT: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    BUILT.get_or_init(|| {
        let source = format!(
            "IMPORT bits\nIMPORT collections\nIMPORT io\nIMPORT math\nIMPORT os\nIMPORT vector\n\n{}\n\n{}\n{MODEL_DRIVER}",
            helper_source("helper_shapes.rs", "const TRIG: &str =\nr#\""),
            helper_source("helper_particles.rs", "pub(super) const MODEL: &str =\nr#\""),
        );
        let project = common::temp_project("canvas_particle_model", &source);
        common::build_project(&project)
    })
}

/// Run one of the driver's cases and answer its output.
fn model_case(test: &str) -> String {
    let out = Command::new(model())
        .env("TEST", test)
        .output()
        .expect("run the model driver");
    assert!(
        out.status.success(),
        "the model driver {}:\n{}",
        common::exit_description(&out.status),
        String::from_utf8_lossy(&out.stderr),
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The words after `name ` on its own line of a case's output.
fn answer(out: &str, name: &str) -> String {
    out.lines()
        .find_map(|line| line.strip_prefix(name)?.strip_prefix(' '))
        .unwrap_or_else(|| panic!("no `{name}` line in:\n{out}"))
        .trim()
        .to_string()
}

fn number(out: &str, name: &str) -> f64 {
    answer(out, name)
        .parse()
        .unwrap_or_else(|e| panic!("`{name}` is not a number ({e}) in:\n{out}"))
}

/// **Determinism.** The same seed, slot, cycle and stream are the same draw — twice in
/// one run, and in a second run of the program — and changing any one of the four
/// changes it.
#[test]
fn the_same_seed_and_slot_give_the_same_draw() {
    let first = model_case("determinism");
    let second = model_case("determinism");
    assert_eq!(answer(&first, "same"), "TRUE");
    assert_eq!(answer(&first, "distinct"), "TRUE");
    assert_eq!(answer(&first, "draws"), answer(&second, "draws"));
    // Every draw is in [0, 1) -- printed in billionths -- and they are not all alike.
    let draws: Vec<i64> = answer(&first, "draws")
        .split_whitespace()
        .map(|d| d.parse().expect("a draw"))
        .collect();
    assert_eq!(draws.len(), 24);
    assert!(
        draws.iter().all(|v| (0..1_000_000_000).contains(v)),
        "{draws:?}"
    );
    let mut unique = draws.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), 24, "{draws:?}");
}

/// **Liveness.** A slot holds no particle before its birth and none after its
/// lifetime; it holds one again when the pool comes round to it, as a new life.
#[test]
fn a_slot_is_dead_before_birth_and_after_its_lifetime() {
    let out = model_case("liveness");
    assert_eq!(answer(&out, "before"), "FALSE");
    assert_eq!(answer(&out, "young"), "TRUE");
    assert_eq!(answer(&out, "old"), "FALSE");
    assert_eq!(answer(&out, "reborn"), "TRUE");
    assert_eq!(answer(&out, "cycle"), "1");
    assert_eq!(answer(&out, "outside"), "FALSE", "a slot past maxParticles");
    assert_eq!(
        answer(&out, "norate"),
        "FALSE",
        "a rate of zero emits nothing"
    );
    assert_eq!(
        answer(&out, "negative"),
        "FALSE",
        "a negative time draws nothing"
    );
}

/// **Burst schedule.** With a burst every two seconds and a one-second lifetime, a slot
/// is alive just after each burst and dead between them.
#[test]
fn a_burst_is_alive_after_each_period_and_dead_between() {
    let out = model_case("burst");
    assert_eq!(answer(&out, "t0.1"), "TRUE");
    assert_eq!(answer(&out, "t1.9"), "FALSE");
    assert_eq!(answer(&out, "t2.1"), "TRUE");
    assert_eq!(answer(&out, "past"), "FALSE", "a slot past `burst`");
    assert_eq!(
        answer(&out, "once"),
        "TRUE FALSE",
        "burstPeriod 0 is one burst"
    );
    assert_eq!(
        answer(&out, "clamped"),
        "TRUE FALSE",
        "burst above the pool"
    );
}

/// **Drag limit.** The path under a vanishing drag is the path under none: the two
/// differ by under 1e-4 px after one second, the two forms of the drag term agree where
/// one hands over to the other, and under heavy drag a falling particle settles at
/// gravity over drag.
#[test]
fn a_vanishing_drag_is_no_drag() {
    let out = model_case("drag");
    // 5 + 120 * 1 + 98 / 2, in billionths of a pixel.
    assert!((number(&out, "free") - 174.0e9).abs() <= 1.0, "{out}");
    // In trillionths: under 1e-4 px is under 1e8 of them.
    assert!(number(&out, "gap") < 1.0e8, "{out}");
    assert!(number(&out, "seam") < 1.0e8, "{out}");
    assert!((number(&out, "terminal") - 49.0e9).abs() < 1000.0, "{out}");
    // e^-1 to a trillionth.
    assert!(number(&out, "exp") <= 1.0, "{out}");
}

/// **Disc uniformity.** Of 10,000 samples of a disc, the inner half-radius disc — a
/// quarter of the area — holds a quarter, within two percent of the samples. A radius
/// drawn evenly, the classic mistake, puts half of them there.
#[test]
fn a_disc_is_sampled_evenly_over_its_area() {
    let out = model_case("disc");
    let inner = number(&out, "inner");
    assert!(
        (inner / 10_000.0 - 0.25).abs() < 0.02,
        "{inner} of 10000 samples fell inside half the radius"
    );
    assert_eq!(number(&out, "outside"), 0.0);
}

/// **Emitter placement.** A centred box under a quarter turn, at a position: every
/// sample is inside the turned box, and many are outside where the box would be
/// unturned — so the emitter's transform turns where particles are born.
#[test]
fn an_emitter_is_placed_and_turned_by_its_own_fields() {
    let out = model_case("placement");
    assert_eq!(number(&out, "inTurned"), 1000.0);
    assert!(number(&out, "outUnturned") > 300.0, "{out}");
    // A quarter of 40 and half of 20, moved by (110, 60).
    assert_eq!(answer(&out, "plain"), "120000000000 70000000000");
}

/// **Stroke emitters.** Every sample of a segment, an arc and a path lies on its curve
/// to 1e-9 px — an open path and one closed back to its start, the form a `PolyCurve`
/// that ends where it began takes — and the path's later edges get their share.
#[test]
fn a_stroke_emitter_samples_its_curve() {
    let out = model_case("strokes");
    assert_eq!(answer(&out, "line"), "TRUE");
    assert_eq!(answer(&out, "arc"), "TRUE");
    assert_eq!(answer(&out, "open"), "TRUE");
    assert_eq!(answer(&out, "closed"), "TRUE");
    // Two equal edges: about half the samples on each.
    assert!((number(&out, "onLast") - 1000.0).abs() < 100.0, "{out}");
    // Closed, the diagonal back is 14.1 of 34.1 px.
    assert!((number(&out, "onClosing") - 828.0).abs() < 100.0, "{out}");
    assert_eq!(answer(&out, "empty"), "FALSE");
}

/// **Polygon emitter.** No sample of an L-shaped polygon falls in its notch or outside
/// its box, nearly every slot finds a point, and the fill rule decides a hole — the
/// rule a `CurvePolygon`'s flattened outline is sampled under too.
#[test]
fn a_polygon_emitter_samples_only_its_inside() {
    let out = model_case("polygon");
    assert_eq!(number(&out, "notch"), 0.0);
    assert_eq!(number(&out, "outside"), 0.0);
    assert!(number(&out, "found") >= 1999.0, "{out}");
    assert_eq!(answer(&out, "hole"), "FALSE TRUE TRUE");
    assert_eq!(answer(&out, "norings"), "FALSE");
}

// ---- Drawing (plan-150-A Phase 4) ------------------------------------------------------

const WIDTH: u32 = 900;
const HEIGHT: u32 = 640;

/// Render a scene headless and synchronously, returning the last frame and the stats
/// line of every frame. `env` adds to the run — `MFB_CANVAS_GPU`, a scripted pointer.
fn draw(name: &str, source: &str, env: &[(&str, &str)]) -> (Frame, Vec<String>) {
    let project = common::temp_project(name, source);
    std::fs::write(project.join("fixture.ttf"), common::fixture_truetype())
        .expect("write the font fixture");
    let binary = common::build_app_debug(&project, name);
    let frame = project.join("frame.rgba");
    let stats = project.join("stats.txt");
    let mut command = Command::new(&binary);
    command
        .current_dir(&project)
        .env("MFB_HEADLESS", "1")
        .env("MFB_CANVAS_SYNC", "1")
        .env("MFB_CANVAS_STATS", &stats)
        .env("MFB_CANVAS_DUMP", &frame);
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
    let pixels = std::fs::read(&frame).expect("canvas dump written");
    let lines = std::fs::read_to_string(&stats)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect();
    let _ = std::fs::remove_dir_all(&project);
    (Frame::from_rgba(WIDTH, HEIGHT, pixels), lines)
}

fn pixel(frame: &Frame, x: u32, y: u32) -> [u8; 4] {
    let at = ((y * frame.width + x) * 4) as usize;
    [
        frame.pixels[at],
        frame.pixels[at + 1],
        frame.pixels[at + 2],
        frame.pixels[at + 3],
    ]
}

/// The pixels that are not the surface's own opaque black, as (count, min x, min y,
/// max x, max y).
fn lit(frame: &Frame) -> (usize, u32, u32, u32, u32) {
    let (mut count, mut x0, mut y0, mut x1, mut y1) = (0, u32::MAX, u32::MAX, 0, 0);
    for y in 0..frame.height {
        for x in 0..frame.width {
            if pixel(frame, x, y) != [0, 0, 0, 255] {
                count += 1;
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    (count, x0, y0, x1, y1)
}

fn stat(line: &str, key: &str) -> String {
    line.split_whitespace()
        .find_map(|field| field.strip_prefix(key))
        .unwrap_or_else(|| panic!("no `{key}` in `{line}`"))
        .to_string()
}

/// What every drawing scene below is written with. `single` is a system of exactly one
/// particle that never moves: one burst of one, from a point, at no speed — so the
/// particle's point is the system's `position`, its turn is `rotation`, its size is
/// `scale` and its tint is `tint`, all known without running the model by hand.
const SCENE_PRELUDE: &str = r#"IMPORT app
IMPORT canvas
IMPORT collections
IMPORT color
IMPORT io
IMPORT math
IMPORT vector

FUNC none() AS canvas::Transform
  RETURN canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
END FUNC

FUNC at(x AS Float, y AS Float) AS vector::Float2
  RETURN vector::Float2[x := x, y := y]
END FUNC

FUNC white() AS color::Color
  RETURN color::rgb(255, 255, 255)
END FUNC

FUNC point() AS canvas::Shape
  MUT c AS canvas::Circle
  LET s AS canvas::Shape = c
  RETURN s
END FUNC

FUNC still(lifetime AS Float) AS canvas::Emission
  RETURN canvas::Emission[mode := canvas::EmissionMode.Burst, maxParticles := 1, rate := 0.0, burst := 1, burstPeriod := 0.0, lifetime := lifetime, lifetimeVar := 0.0]
END FUNC

FUNC look(first AS color::Color, last AS color::Color, scale AS Float) AS canvas::Appearance
  RETURN canvas::Appearance[startColor := first, endColor := last, startScale := scale, endScale := scale, scaleVar := 0.0]
END FUNC

FUNC single(item AS canvas::AutoShape, x AS Float, y AS Float, rotation AS Float, scale AS Float, tint AS color::Color) AS canvas::DrawItem
  MUT motion AS canvas::Motion
  LET ps AS canvas::DrawItem = canvas::ParticleSystem[position := at(x, y), item := item, emitter := point(), emission := still(10.0), motion := WITH motion { rotation := rotation }, appearance := look(tint, tint, scale), seed := 1, time := 1.0]
  RETURN ps
END FUNC

FUNC box(x AS Float, y AS Float, w AS Float, h AS Float, paint AS canvas::Paint, t AS canvas::Transform) AS canvas::Rectangle
  MUT r AS canvas::Rectangle
  RETURN WITH r { position := at(x, y), size := canvas::Size[width := w, height := h], paint := paint, transform := t }
END FUNC

FUNC disc(radius AS Float, paint AS canvas::Paint) AS canvas::Circle
  MUT c AS canvas::Circle
  RETURN WITH c { radius := radius, paint := paint }
END FUNC

FUNC node(name AS String, x AS Float, y AS Float, t AS canvas::Transform) AS canvas::DrawItem
  MUT g AS canvas::Group
  LET item AS canvas::DrawItem = WITH g { name := name, position := at(x, y), transform := t }
  RETURN item
END FUNC

FUNC scaled(by AS Float) AS canvas::Transform
  RETURN canvas::Transform[a := by, b := 0.0, c := 0.0, d := by, tx := 0.0, ty := 0.0]
END FUNC
"#;

fn scene(body: &str) -> String {
    format!(
        "{SCENE_PRELUDE}\nFUNC main() AS Nothing\n  app::setMode(app::Mode.Canvas)\n{body}  io::print(\"rendered\")\nEND FUNC\n"
    )
}

/// A template that tests the composition: a stroked rectangle that is not at the
/// particle's point and has a transform of its own — a turn of 36.87 degrees (0.8, 0.6)
/// and an offset.
const TILTED: &str = "box(6.0, 4.0, 60.0, 30.0, canvas::fillStroke(color::rgb(40, 110, 255), white(), 3.0), canvas::Transform[a := 0.8, b := 0.6, c := 0.0 - 0.6, d := 0.8, tx := 5.0, ty := 0.0 - 3.0])";

/// **A particle equals its definition.** One live particle renders pixel for pixel as
/// its template inside a group node at the particle's point, with the particle's
/// transform — for a particle that is only moved, and for one that is scaled, with a
/// template that carries a position and a transform of its own.
///
/// The moved particle is at a point that is not a whole pixel on purpose: see
/// `a_moved_particle_is_the_template_moved` for what a whole one shows.
#[test]
fn a_particle_is_its_template_in_a_group_node() {
    for (name, at, rotation_scale, group_transform) in [
        ("moved", "300.25, 200.5", "0.0, 1.0", "none()"),
        ("scaled", "300.0, 200.0", "0.0, 1.5", "scaled(1.5)"),
    ] {
        let (particle, _) = draw(
            &format!("canvas_particle_is_{name}"),
            &scene(&format!(
                "  canvas::present([single({TILTED}, {at}, {rotation_scale}, white())])\n"
            )),
            &[],
        );
        let (grouped, _) = draw(
            &format!("canvas_particle_group_{name}"),
            &scene(&format!(
                "  LET shape AS canvas::DrawItem = {TILTED}\n  canvas::setGroup(\"p\", [shape])\n  canvas::present([node(\"p\", {at}, {group_transform})])\n"
            )),
            &[],
        );
        assert!(lit(&grouped).0 > 1000, "{name}: the reference drew nothing");
        if let Err(diff) = compare_exact(&particle, &grouped) {
            panic!("{name}: a particle is not its template in a group node: {diff}");
        }
    }
}

/// A particle that is neither turned nor scaled IS the template moved: the item the
/// expansion appends is the template with the particle's point added to its
/// `position`, and it renders exactly as that ordinary item does.
///
/// It is NOT always exactly the template in a group moved to that point, and that is
/// the renderer's own property, not the expansion's — the comparison below has no
/// particle in it. A shape with a transform is placed through an inverse held in single
/// precision; at (306, 204) that inverse carries the whole position, inside a group it
/// carries only (6, 4) and the group's (300, 200) is subtracted exactly. The two differ
/// by about 1e-5 px. This template is turned by (0.8, 0.6) on whole coordinates, so its
/// edges cross pixel centres at exact tenths and many coverages sit on a rounding tie:
/// 97 of 576,000 pixels land a level or two apart. Measured, and bounded here so that a
/// real difference between the two paths would not hide behind it.
#[test]
fn a_moved_particle_is_the_template_moved() {
    let (particle, _) = draw(
        "canvas_particle_moved",
        &scene(&format!(
            "  canvas::present([single({TILTED}, 300.0, 200.0, 0.0, 1.0, white())])\n"
        )),
        &[],
    );
    let ordinary_item = TILTED.replacen("box(6.0, 4.0,", "box(306.0, 204.0,", 1);
    let (ordinary, _) = draw(
        "canvas_particle_moved_ordinary",
        &scene(&format!(
            "  LET shape AS canvas::DrawItem = {ordinary_item}\n  canvas::present([shape])\n"
        )),
        &[],
    );
    assert!(lit(&ordinary).0 > 1000);
    if let Err(diff) = compare_exact(&particle, &ordinary) {
        panic!("a moved particle is not the template moved: {diff}");
    }
    let (grouped, _) = draw(
        "canvas_particle_moved_group",
        &scene(&format!(
            "  LET shape AS canvas::DrawItem = {TILTED}\n  canvas::setGroup(\"p\", [shape])\n  canvas::present([node(\"p\", 300.0, 200.0, none())])\n"
        )),
        &[],
    );
    let placement = Tolerance {
        max_channel_delta: 2,
        max_differing_fraction: 0.0002,
    };
    if let Err(diff) = compare_within_tolerance(&ordinary, &grouped, placement) {
        panic!("a placed shape and the same shape in a moved group are further apart than single-precision placement explains: {diff}");
    }
}

/// The same for a particle that is TURNED. The model turns it with the package's own
/// sine and cosine, which a program cannot call, so the reference group is turned with
/// `math::`'s — the same angle to a few parts in ten million. That is far below a
/// coverage step, so the two frames may differ only in the last level of a handful of
/// edge pixels.
#[test]
fn a_turned_particle_is_its_template_in_a_turned_group_node() {
    let (particle, _) = draw(
        "canvas_particle_is_turned",
        &scene(&format!(
            "  canvas::present([single({TILTED}, 300.0, 200.0, 0.5, 1.5, white())])\n"
        )),
        &[],
    );
    let (grouped, _) = draw(
        "canvas_particle_group_turned",
        &scene(&format!(
            "  LET shape AS canvas::DrawItem = {TILTED}\n  canvas::setGroup(\"p\", [shape])\n  LET co AS Float = 1.5 * math::cos(0.5)\n  LET si AS Float = 1.5 * math::sin(0.5)\n  canvas::present([node(\"p\", 300.0, 200.0, canvas::Transform[a := co, b := si, c := 0.0 - si, d := co, tx := 0.0, ty := 0.0])])\n"
        )),
        &[],
    );
    assert!(lit(&grouped).0 > 1000);
    let close = Tolerance {
        max_channel_delta: 1,
        max_differing_fraction: 0.0001,
    };
    if let Err(diff) = compare_within_tolerance(&particle, &grouped, close) {
        panic!("a turned particle is not its template in a turned group node: {diff}");
    }
}

/// **A scale that reaches zero draws nothing** — not the shape at full size, which is
/// what the shape rule would make of a transform with no area.
#[test]
fn a_particle_scaled_to_nothing_is_not_drawn() {
    let (frame, _) = draw(
        "canvas_particle_zero_scale",
        &scene(
            "  canvas::present([single(disc(60.0, canvas::fill(white())), 300.0, 200.0, 0.0, 0.0, white())])\n",
        ),
        &[],
    );
    assert_eq!(lit(&frame).0, 0, "a particle of no size was drawn");
    // The same system at a size draws: the frame above is black because of the scale.
    let (frame, _) = draw(
        "canvas_particle_small_scale",
        &scene(
            "  canvas::present([single(disc(60.0, canvas::fill(white())), 300.0, 200.0, 0.0, 0.5, white())])\n",
        ),
        &[],
    );
    assert_eq!(pixel(&frame, 300, 200), [255, 255, 255, 255]);
    assert_eq!(pixel(&frame, 300, 240), [0, 0, 0, 255], "half of 60 is 30");
}

/// **A system inside a turned, scaled group** is turned and scaled with it, like any
/// item: it renders as the nested groups its definition makes of it.
#[test]
fn a_system_in_a_turned_group_is_turned_with_it() {
    let outer =
        "canvas::Transform[a := 0.96, b := 0.56, c := 0.0 - 0.56, d := 0.96, tx := 0.0, ty := 0.0]";
    let (system, _) = draw(
        "canvas_particle_in_group",
        &scene(&format!(
            "  canvas::setGroup(\"g\", [single({TILTED}, 50.25, 20.5, 0.0, 1.5, white())])\n  canvas::present([node(\"g\", 300.0, 200.0, {outer})])\n"
        )),
        &[],
    );
    let (nested, _) = draw(
        "canvas_particle_in_group_reference",
        &scene(&format!(
            "  LET shape AS canvas::DrawItem = {TILTED}\n  canvas::setGroup(\"p\", [shape])\n  canvas::setGroup(\"g\", [node(\"p\", 50.25, 20.5, scaled(1.5))])\n  canvas::present([node(\"g\", 300.0, 200.0, {outer})])\n"
        )),
        &[],
    );
    assert!(lit(&nested).0 > 1000);
    if let Err(diff) = compare_exact(&system, &nested) {
        panic!("a system in a turned group is not its definition there: {diff}");
    }
}

/// **The tint multiplies.** Each channel of the template's fill is multiplied by the
/// particle's, so white leaves it alone, and the change from `startColor` to `endColor`
/// is even in linear light: halfway from white to black is sRGB 188, not 128.
#[test]
fn the_tint_multiplies_and_fades_in_linear_light() {
    let (frame, _) = draw(
        "canvas_particle_tint",
        &scene(
            "  canvas::present([single(disc(40.0, canvas::fill(color::rgb(200, 100, 50))), 300.0, 200.0, 0.0, 1.0, color::rgb(128, 255, 0))])\n",
        ),
        &[],
    );
    // (200 * 128 + 127) / 255, 100 * 255 / 255, 50 * 0.
    assert_eq!(pixel(&frame, 300, 200), [100, 100, 0, 255]);

    let (frame, _) = draw(
        "canvas_particle_fade",
        &scene(
            "  MUT motion AS canvas::Motion\n  LET ps AS canvas::DrawItem = canvas::ParticleSystem[position := at(300.0, 200.0), item := disc(40.0, canvas::fill(white())), emitter := point(), emission := still(2.0), motion := motion, appearance := look(white(), color::rgb(0, 0, 0), 1.0), seed := 1, time := 1.0]\n  canvas::present([ps])\n",
        ),
        &[],
    );
    assert_eq!(pixel(&frame, 300, 200), [188, 188, 188, 255]);
}

/// **A `Text` template.** A run of text is a particle like any shape: placed, scaled
/// about the particle's point and tinted. The fixture font's one glyph inks the square
/// (100, 0)-(400, 300) of a 1000-unit em, so at size 40 and twice the size `A` is a
/// 24 px square from 8 px right of the point, standing on it.
#[test]
fn a_text_template_is_placed_scaled_and_tinted() {
    let (frame, _) = draw(
        "canvas_particle_text",
        &scene(
            "  RES face AS canvas::Font = canvas::loadFont(\"fixture.ttf\") TRAP(e)\n    RETURN\n  END TRAP\n  LET label AS canvas::AutoShape = canvas::Text[id := \"\", position := at(0.0, 0.0), text := \"A\", fonts := [face], fontSize := 40.0, paint := canvas::fill(white()), transform := none()]\n  canvas::present([single(label, 300.0, 200.0, 0.0, 2.0, color::rgb(255, 128, 0))])\n",
        ),
        &[],
    );
    assert_eq!(
        pixel(&frame, 320, 188),
        [255, 128, 0, 255],
        "inside the glyph"
    );
    let (count, x0, y0, x1, y1) = lit(&frame);
    assert!(count > 400, "the glyph is 24 px square");
    assert!(
        x0 >= 307 && x1 <= 332 && y0 >= 175 && y1 <= 200,
        "the glyph is outside its scaled box: ({x0}, {y0})-({x1}, {y1})"
    );
}

/// **An `AutoPaint` template draws its shape's own paint**, whatever the mouse is
/// doing: with the pointer resting on the particle and a hover paint in use, the
/// particle is still the shape's own green, and the frame notes no `AutoPaint`.
#[test]
fn an_autopaint_template_draws_its_own_paint() {
    let (frame, stats) = draw(
        "canvas_particle_autopaint",
        &scene(
            "  LET red AS canvas::Paint = canvas::fill(color::rgb(255, 0, 0))\n  LET button AS canvas::AutoShape = canvas::AutoPaint[shape := disc(40.0, canvas::fill(color::rgb(0, 255, 0))), useEnter := TRUE, useExit := TRUE, useHover := TRUE, usePressed := TRUE, enterPaint := red, exitPaint := red, hoverPaint := red, pressedPaint := red]\n  canvas::present([single(button, 300.0, 200.0, 0.0, 1.0, white())])\n  canvas::present([single(button, 300.0, 200.0, 0.0, 1.0, white()), single(button, 600.0, 200.0, 0.0, 1.0, white())])\n",
        ),
        &[("MFB_CANVAS_POINTER", "0,300,200,1,1")],
    );
    assert_eq!(pixel(&frame, 300, 200), [0, 255, 0, 255]);
    assert_eq!(pixel(&frame, 600, 200), [0, 255, 0, 255]);
    let last = stats.last().expect("a frame");
    // `auto=` lists the frame's `AutoPaint` instances; `-` is none.
    assert_eq!(
        stat(last, "auto="),
        "-",
        "a particle was noted as an AutoPaint instance: {last}"
    );
}

/// **Curve emitters.** A `PolyCurve` emits along its curve and a `CurvePolygon` over
/// its inside: every particle of each lands within the curve's own extent (the arch
/// from (0, 0) over (20, 30) to (40, 0) peaks at y = 15), and they are spread across
/// it, not piled on a point.
#[test]
fn curve_shapes_are_emitters() {
    for (name, emitter, y_from, y_to) in [
        ("polycurve", "WITH pc { curves := [bend] }", 0, 15),
        (
            "curvepolygon",
            "WITH cp { contours := [[bend, back]] }",
            -15,
            15,
        ),
    ] {
        let (frame, _) = draw(
            &format!("canvas_particle_emit_{name}"),
            &scene(&format!(
                "  LET bend AS canvas::BezierCurve = canvas::BezierCurve[points := [at(0.0, 0.0), at(20.0, 30.0), at(40.0, 0.0)], weights := []]\n  LET back AS canvas::BezierCurve = canvas::BezierCurve[points := [at(40.0, 0.0), at(20.0, 0.0 - 30.0), at(0.0, 0.0)], weights := []]\n  MUT pc AS canvas::PolyCurve\n  MUT cp AS canvas::CurvePolygon\n  LET emitter AS canvas::Shape = {emitter}\n  MUT motion AS canvas::Motion\n  MUT emission AS canvas::Emission\n  LET ps AS canvas::DrawItem = canvas::ParticleSystem[position := at(300.0, 200.0), item := disc(1.0, canvas::fill(white())), emitter := emitter, emission := WITH emission {{ mode := canvas::EmissionMode.Burst, maxParticles := 300, burst := 300, lifetime := 10.0 }}, motion := motion, appearance := look(white(), white(), 1.0), seed := 4, time := 1.0]\n  canvas::present([ps])\n"
            )),
            &[],
        );
        let (count, x0, y0, x1, y1) = lit(&frame);
        // 300 dots: along a 50 px arch they overlap, over the lens they mostly do not.
        assert!(count > 100, "{name}: {count} lit pixels");
        // The emitter's extent, a pixel of slack and the particle's own radius.
        assert!(
            x0 >= 298 && x1 <= 342 && y0 as i64 >= 198 + y_from && y1 as i64 <= 202 + y_to,
            "{name}: particles outside the emitter: ({x0}, {y0})-({x1}, {y1})"
        );
        assert!(x1 - x0 >= 30, "{name}: particles span only {x0}..{x1}");
    }
}

/// A thousand particles between two ordinary items, as one scene index: every one is
/// drawn, the items either side of the system are where they were, and the GPU draws
/// the same picture ON the GPU. This is the path an item of several entries takes
/// (`__canvas_sceneOffsets`), built for a `Picture`'s two and measured here at a
/// thousand.
const THOUSAND: &str = "  LET before AS canvas::DrawItem = box(20.0, 20.0, 60.0, 40.0, canvas::fill(color::rgb(255, 0, 0)), none())\n  LET after AS canvas::DrawItem = box(820.0, 580.0, 60.0, 40.0, canvas::fill(color::rgb(0, 0, 255)), none())\n  MUT ring AS canvas::Circle\n  LET emitter AS canvas::Shape = WITH ring { radius := 250.0 }\n  MUT motion AS canvas::Motion\n  MUT emission AS canvas::Emission\n  LET ps AS canvas::DrawItem = canvas::ParticleSystem[position := at(450.0, 320.0), item := disc(2.0, canvas::fill(color::rgb(0, 255, 0))), emitter := emitter, emission := WITH emission { mode := canvas::EmissionMode.Burst, maxParticles := 1000, burst := 1000, lifetime := 10.0 }, motion := WITH motion { rotation := 0.3 }, appearance := look(white(), white(), 1.5), seed := 2, time := 1.0]\n  canvas::present([before, ps, after])\n";

#[test]
fn a_thousand_particles_are_one_scene_index() {
    let (software, stats) = draw("canvas_particle_thousand", &scene(THOUSAND), &[]);
    let last = stats.last().expect("a frame");
    // The two rectangles and a record per particle.
    assert_eq!(stat(last, "entries="), "1002", "{last}");
    assert_eq!(pixel(&software, 50, 40), [255, 0, 0, 255]);
    assert_eq!(pixel(&software, 850, 600), [0, 0, 255, 255]);
    // 1000 discs of radius 3 over a disc of radius 250: most of ~28,000 px, less overlap.
    let green = (0..HEIGHT)
        .flat_map(|y| (0..WIDTH).map(move |x| (x, y)))
        .filter(|(x, y)| pixel(&software, *x, *y) == [0, 255, 0, 255])
        .count();
    assert!(green > 12_000, "{green} wholly green pixels");

    let (gpu, stats) = draw(
        "canvas_particle_thousand_gpu",
        &scene(THOUSAND),
        &[("MFB_CANVAS_GPU", "1")],
    );
    let last = stats.last().expect("a frame");
    if !last.contains("metalReady=TRUE") && !last.contains("vulkanReady=TRUE") {
        eprintln!("skip the GPU half: this host built no GPU pipeline\n{last}");
        return;
    }
    assert_ne!(stat(last, "gpuFrames="), "0", "the GPU declined: {last}");
    assert_eq!(stat(last, "blocks="), "1002", "{last}");
    if let Err(diff) = compare_within_tolerance(&gpu, &software, Tolerance::GPU_DEFAULT) {
        panic!("the GPU's thousand particles disagree with the oracle: {diff}");
    }
}

/// A system that is a GROUP's child reaches the GPU as blocks of that group's run
/// (`__canvas_memoGroup`), one per live particle, drawn under the group's matrix — and
/// the group's other items are still either side of them.
#[test]
fn a_system_in_a_group_reaches_the_gpu() {
    let body = "  LET under AS canvas::DrawItem = box(0.0 - 40.0, 0.0 - 40.0, 80.0, 80.0, canvas::fill(color::rgb(90, 0, 0)), none())\n  LET over AS canvas::DrawItem = box(60.0, 0.0 - 10.0, 30.0, 20.0, canvas::fill(color::rgb(0, 0, 200)), none())\n  MUT ring AS canvas::Circle\n  LET emitter AS canvas::Shape = WITH ring { radius := 70.0 }\n  MUT motion AS canvas::Motion\n  MUT emission AS canvas::Emission\n  LET ps AS canvas::DrawItem = canvas::ParticleSystem[position := at(0.0, 0.0), item := box(0.0 - 3.0, 0.0 - 2.0, 6.0, 4.0, canvas::fillStroke(color::rgb(0, 220, 0), white(), 1.0), none()), emitter := emitter, emission := WITH emission { mode := canvas::EmissionMode.Burst, maxParticles := 150, burst := 150, lifetime := 10.0 }, motion := WITH motion { rotationVar := 3.0 }, appearance := look(white(), white(), 1.0), seed := 9, time := 1.0]\n  canvas::setGroup(\"g\", [under, ps, over])\n  LET turn AS canvas::Transform = canvas::Transform[a := 1.2, b := 0.9, c := 0.0 - 0.9, d := 1.2, tx := 0.0, ty := 0.0]\n  canvas::present([node(\"g\", 250.0, 250.0, turn), node(\"g\", 650.0, 400.0, none())])\n";
    let (software, _) = draw("canvas_particle_grouped", &scene(body), &[]);
    assert!(lit(&software).0 > 10_000);
    let (gpu, stats) = draw(
        "canvas_particle_grouped_gpu",
        &scene(body),
        &[("MFB_CANVAS_GPU", "1")],
    );
    let last = stats.last().expect("a frame");
    if !last.contains("metalReady=TRUE") && !last.contains("vulkanReady=TRUE") {
        eprintln!("skip: this host built no GPU pipeline\n{last}");
        return;
    }
    assert_ne!(stat(last, "gpuFrames="), "0", "the GPU declined: {last}");
    // One shared run for both uses of the group: two rectangles and 150 particles.
    assert_eq!(stat(last, "blocks="), "152", "{last}");
    if let Err(diff) = compare_within_tolerance(&gpu, &software, Tolerance::GPU_DEFAULT) {
        panic!("a grouped system on the GPU disagrees with the oracle: {diff}");
    }
}

/// **The raise.** A system whose `lifetime` is zero or less is refused by `present`, on
/// the worker, as `ErrCanvasParticleLifetime` (7-705-0028) — in the scene, in a group
/// the scene draws, and in a layer — and a `TRAP` catches it. Everything else that makes
/// a system draw nothing is not an error.
#[test]
fn a_lifetime_of_zero_raises_from_present() {
    let system = "  MUT motion AS canvas::Motion\n  MUT emission AS canvas::Emission\n  LET ps AS canvas::DrawItem = canvas::ParticleSystem[position := at(300.0, 200.0), item := disc(4.0, canvas::fill(white())), emitter := point(), emission := WITH emission { maxParticles := 10, rate := 5.0, lifetime := LIFETIME }, motion := motion, appearance := look(white(), white(), 1.0), seed := 1, time := TIME]\n";
    let trap = " TRAP(e)\n    io::print(\"raised \" & toString(e.code))\n    RETURN\n  END TRAP\n  io::print(\"NO RAISE\")\n";
    for (name, lifetime, present) in [
        ("scene", "0.0", "  canvas::present([ps])"),
        ("negative", "0.0 - 1.0", "  canvas::present([ps])"),
        (
            "group",
            "0.0",
            "  canvas::setGroup(\"g\", [ps])\n  canvas::present([node(\"g\", 0.0, 0.0, none())])",
        ),
        (
            "layer",
            "0.0",
            "  canvas::presentLayers([canvas::DrawLayer[items := [ps]]])",
        ),
    ] {
        let body = format!(
            "{}{present}{trap}",
            system.replace("LIFETIME", lifetime).replace("TIME", "1.0")
        );
        let run = run(&format!("canvas_particle_raise_{name}"), &scene(&body), &[]);
        assert!(
            run.stdout.contains("raised 77050028"),
            "{name}: expected ErrCanvasParticleLifetime from present, got: {}",
            run.stdout
        );
    }
    // Not errors: an empty pool, no rate, and a time before the first birth.
    for (name, edit, time) in [
        (
            "empty_pool",
            ("maxParticles := 10", "maxParticles := 0"),
            "1.0",
        ),
        ("no_rate", ("rate := 5.0", "rate := 0.0"), "1.0"),
        ("negative_time", ("rate := 5.0", "rate := 5.0"), "0.0 - 1.0"),
    ] {
        let body = format!(
            "{}  canvas::present([ps]){trap}",
            system
                .replace("LIFETIME", "1.0")
                .replace("TIME", time)
                .replace(edit.0, edit.1)
        );
        let (frame, _) = draw(&format!("canvas_particle_quiet_{name}"), &scene(&body), &[]);
        assert_eq!(lit(&frame).0, 0, "{name} drew something");
    }
}

/// The runtime proof: a burst animated over sixty frames by nothing but the program
/// advancing `time`. Every present draws a frame (a new `time` is a new scene — redraw
/// trigger 1, and no timer), the frame's blocks are the particles ALIVE at that time
/// and not the pool, and on a host with a GPU every one of the sixty is a GPU frame.
///
/// The burst is 100 of a pool of 1,000, each living between a quarter and three
/// quarters of a second, and `time` runs from 0 to 1 s: 100 blocks on the first frame,
/// fewer as they die, none by the end — and never 1,000.
#[test]
fn sixty_frames_follow_the_programs_clock() {
    let body = "  MUT motion AS canvas::Motion\n  MUT emission AS canvas::Emission\n  MUT ring AS canvas::Circle\n  LET emitter AS canvas::Shape = WITH ring { radius := 200.0 }\n  LET burst AS canvas::Emission = WITH emission { mode := canvas::EmissionMode.Burst, maxParticles := 1000, burst := 100, lifetime := 0.5, lifetimeVar := 0.5 }\n  MUT frame AS Integer = 0\n  WHILE frame < 60\n    LET ps AS canvas::DrawItem = canvas::ParticleSystem[position := at(450.0, 320.0), item := disc(3.0, canvas::fill(white())), emitter := emitter, emission := burst, motion := WITH motion { speed := 40.0, spread := 3.2, gravityY := 90.0 }, appearance := look(white(), color::rgba(255, 80, 0, 0), 1.0), seed := 60, time := toFloat(frame) / 59.0]\n    canvas::present([ps])\n    frame = frame + 1\n  END WHILE\n";
    let (_, stats) = draw(
        "canvas_particle_sixty_frames",
        &scene(body),
        &[("MFB_CANVAS_GPU", "1")],
    );
    assert_eq!(stats.len(), 60, "one frame per present");
    let blocks: Vec<u32> = stats
        .iter()
        .map(|line| stat(line, "blocks=").parse().expect("blocks="))
        .collect();
    let last = stats.last().expect("a frame");
    if last.contains("metalReady=TRUE") || last.contains("vulkanReady=TRUE") {
        assert_eq!(stat(last, "gpuFrames="), "60", "{last}");
        assert_eq!(
            blocks[0], 100,
            "every particle of the burst is alive at time 0"
        );
        assert_eq!(blocks[59], 0, "none outlives 0.75 s");
        assert!(
            blocks.windows(2).all(|pair| pair[1] <= pair[0]),
            "a burst only loses particles: {blocks:?}"
        );
        assert!(blocks.iter().all(|b| *b <= 100), "{blocks:?}");
        // Somewhere in the middle some are alive and some are dead.
        assert!(blocks[30] > 0 && blocks[30] < 100, "{blocks:?}");
    } else {
        eprintln!("skip the GPU half: this host built no GPU pipeline\n{last}");
    }
    // The geometry cache holds the frame's particles and nothing left over from the
    // frames before: what the previous frame built and this one did not use is dropped.
    let entries: u32 = stat(last, "entries=").parse().expect("entries=");
    assert!(entries <= 100, "the cache kept dead particles: {last}");
}

// ---- Damage (plan-150-B Phase 3) -------------------------------------------------------

/// A steady stream in the lower right quarter of the surface, over a background that
/// never changes, presented thirty times with `time` advanced. The number of live
/// particles differs on nearly every frame — the pool fills over the first 0.9 s — which
/// is what used to make every frame a full redraw.
const STREAM_OVER_BACKGROUND: &str = "  LET background AS canvas::DrawItem = box(0.0, 0.0, 900.0, 640.0, canvas::fill(color::rgb(20, 30, 60)), none())\n  MUT lip AS canvas::Line\n  LET emitter AS canvas::Shape = WITH lip { startAt := at(0.0 - 40.0, 0.0), endAt := at(40.0, 0.0) }\n  MUT emission AS canvas::Emission\n  MUT motion AS canvas::Motion\n  MUT frame AS Integer = 0\n  WHILE frame < 30\n    LET ps AS canvas::DrawItem = canvas::ParticleSystem[position := at(700.0, 520.0), item := disc(3.0, canvas::fill(color::rgb(255, 200, 80))), emitter := emitter, emission := WITH emission { maxParticles := 60, rate := 40.0, lifetime := 0.9 }, motion := WITH motion { speed := 120.0, angle := 0.0 - 1.5707963267948966, spread := 0.5, gravityY := 200.0 }, appearance := look(white(), color::rgba(255, 255, 255, 60), 1.0), seed := 3, time := toFloat(frame) / 30.0]\n    canvas::present([background, ps])\n    frame = frame + 1\n  END WHILE\n";

/// The damage rectangle of a stats line, as (x, y, width, height); `None` for `none`.
fn damage(line: &str) -> Option<(u32, u32, u32, u32)> {
    let text = stat(line, "damage=");
    if text == "none" {
        return None;
    }
    let parts: Vec<u32> = text
        .split(',')
        .map(|p| p.parse().expect("damage"))
        .collect();
    Some((parts[0], parts[1], parts[2], parts[3]))
}

/// **Under `MFB_CANVAS_DAMAGE`, a moving particle system repaints where it moved.** A
/// system is one unit of the damage diff, whatever its live count, so after the first
/// frame every frame is a partial redraw whose rectangle lies inside the quarter of the
/// surface the particles are in — and the picture it leaves is, pixel for pixel, the
/// picture a full redraw of the same scene makes.
#[test]
fn damage_of_a_system_is_bounded_by_its_particles() {
    let (partial, stats) = draw(
        "canvas_particle_damage",
        &scene(STREAM_OVER_BACKGROUND),
        &[("MFB_CANVAS_DAMAGE", "1")],
    );
    assert_eq!(stats.len(), 30, "one frame per present");
    let entries: Vec<String> = stats.iter().map(|l| stat(l, "blocks=")).collect();
    let mut distinct = entries.clone();
    distinct.sort();
    distinct.dedup();
    assert!(
        distinct.len() > 10,
        "the live count was meant to change from frame to frame: {entries:?}"
    );
    assert_eq!(
        damage(&stats[0]),
        Some((0, 0, 900, 640)),
        "the first frame is full"
    );
    for (frame, line) in stats.iter().enumerate().skip(1) {
        // A frame where nothing is alive yet and nothing was has no damage at all.
        let Some((x, y, w, h)) = damage(line) else {
            continue;
        };
        assert!(
            x >= 450 && y >= 320 && x + w <= 900 && y + h <= 640 && (w < 450 && h < 320),
            "frame {frame} damaged ({x}, {y}) {w}x{h}, outside the particles' quarter: {line}"
        );
    }
    assert_eq!(stat(stats.last().expect("a frame"), "partial="), "29");

    let (full, _) = draw(
        "canvas_particle_damage_off",
        &scene(STREAM_OVER_BACKGROUND),
        &[],
    );
    assert!(lit(&full).0 > 500_000, "the background fills the surface");
    if let Err(diff) = compare_exact(&partial, &full) {
        panic!("thirty partial redraws left a different picture from a full one: {diff}");
    }
}

/// A scene of ordinary items beside a system still diffs item by item: moving one small
/// rectangle far from the system damages that rectangle's two places and nothing of the
/// system, whose particles did not move.
#[test]
fn damage_of_an_item_beside_a_system_is_the_items() {
    let body = "  MUT ring AS canvas::Circle\n  LET emitter AS canvas::Shape = WITH ring { radius := 40.0 }\n  MUT motion AS canvas::Motion\n  MUT emission AS canvas::Emission\n  LET ps AS canvas::DrawItem = canvas::ParticleSystem[position := at(700.0, 500.0), item := disc(3.0, canvas::fill(white())), emitter := emitter, emission := WITH emission { mode := canvas::EmissionMode.Burst, maxParticles := 50, burst := 50, lifetime := 10.0 }, motion := motion, appearance := look(white(), white(), 1.0), seed := 8, time := 1.0]\n  LET first AS canvas::DrawItem = box(40.0, 40.0, 30.0, 20.0, canvas::fill(color::rgb(255, 0, 0)), none())\n  LET moved AS canvas::DrawItem = box(100.0, 60.0, 30.0, 20.0, canvas::fill(color::rgb(255, 0, 0)), none())\n  LET after AS canvas::DrawItem = box(300.0, 300.0, 10.0, 10.0, canvas::fill(color::rgb(0, 0, 255)), none())\n  canvas::present([first, ps, after])\n  canvas::present([moved, ps, after])\n";
    let (_, stats) = draw(
        "canvas_particle_damage_beside",
        &scene(body),
        &[("MFB_CANVAS_DAMAGE", "1")],
    );
    assert_eq!(stats.len(), 2);
    let (x, y, w, h) = damage(&stats[1]).expect("the rectangle moved");
    assert!(
        x >= 38 && y >= 38 && x + w <= 133 && y + h <= 83,
        "moving a rectangle damaged ({x}, {y}) {w}x{h}: {}",
        stats[1]
    );
}

// ---- The uncached record (plan-150-B Phase 2) ------------------------------------------

/// Ten ordinary items in the left half of the surface and, in the right half, a system of
/// 1,000 particles that are neither turned nor scaled, presented `FRAMES` times with
/// `time` advanced. `WITH_SYSTEM` is replaced by TRUE or FALSE.
const MIXED: &str = "  MUT items AS List OF canvas::DrawItem = []\n  MUT i AS Integer = 0\n  WHILE i < 10\n    LET item AS canvas::DrawItem = box(20.0 + toFloat(i) * 40.0, 60.0 + toFloat(i) * 50.0, 30.0, 30.0, canvas::fillStroke(color::rgb(200, 40 + i * 20, 90), white(), 2.0), none())\n    items = collections::append(items, item)\n    i = i + 1\n  END WHILE\n  MUT ring AS canvas::Circle\n  LET emitter AS canvas::Shape = WITH ring { radius := 150.0 }\n  MUT motion AS canvas::Motion\n  MUT emission AS canvas::Emission\n  MUT frame AS Integer = 0\n  WHILE frame < FRAMES\n    MUT scene AS List OF canvas::DrawItem = items\n    IF WITH_SYSTEM THEN\n      LET ps AS canvas::DrawItem = canvas::ParticleSystem[position := at(680.0, 320.0), item := disc(2.0, canvas::fill(color::rgb(90, 220, 255))), emitter := emitter, emission := WITH emission { mode := canvas::EmissionMode.Burst, maxParticles := 1000, burst := 1000, lifetime := 100.0 }, motion := WITH motion { speed := 15.0, spread := 3.2 }, appearance := look(white(), white(), 1.0), seed := 12, time := toFloat(frame) / 30.0]\n      scene = collections::append(scene, ps)\n    END IF\n    canvas::present(scene)\n    frame = frame + 1\n  END WHILE\n";

/// **A plain particle's record is not cached.** A thousand particles that are neither
/// turned nor scaled are a thousand blocks of the frame and not one entry of the
/// geometry cache — which holds the ten ordinary items and nothing else — and the
/// floats the cache's arena holds do not grow with the frames: what one frame's
/// particles took is dropped at the next frame boundary with everything else unused.
#[test]
fn a_plain_particles_record_is_not_cached() {
    let body = MIXED.replace("FRAMES", "12").replace("WITH_SYSTEM", "TRUE");
    let (_, stats) = draw(
        "canvas_particle_uncached",
        &scene(&body),
        &[("MFB_CANVAS_GPU", "1")],
    );
    assert_eq!(stats.len(), 12);
    let last = stats.last().expect("a frame");
    assert_eq!(stat(last, "entries="), "10", "{last}");
    if last.contains("metalReady=TRUE") || last.contains("vulkanReady=TRUE") {
        assert_eq!(stat(last, "blocks="), "1010", "{last}");
        assert_eq!(stat(last, "gpuFrames="), "12", "{last}");
    }
    let floats: Vec<u64> = stats
        .iter()
        .map(|line| stat(line, "floats=").parse().expect("floats="))
        .collect();
    // One frame's particles are about 70,000 floats. The arena may hold two frames'
    // worth before a boundary drops the older; it may not hold twelve.
    let most = *floats.iter().max().expect("frames");
    assert!(most < 250_000, "the arena grew with the frames: {floats:?}");
    assert!(
        floats[11] <= floats[5] + 1000,
        "the arena is still growing at the twelfth frame: {floats:?}"
    );
}

/// **The ordinary items are untouched by the system beside them.** The ten items draw
/// the same pixels with a thousand particles in the scene as without: the left half of
/// the surface, where the items are, is identical, and the right half is where every
/// particle is.
#[test]
fn ordinary_items_draw_the_same_beside_a_system() {
    let (with, _) = draw(
        "canvas_particle_mixed_with",
        &scene(&MIXED.replace("FRAMES", "3").replace("WITH_SYSTEM", "TRUE")),
        &[],
    );
    let (without, _) = draw(
        "canvas_particle_mixed_without",
        &scene(&MIXED.replace("FRAMES", "3").replace("WITH_SYSTEM", "FALSE")),
        &[],
    );
    let mut items = 0;
    let mut particles = 0;
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let a = pixel(&with, x, y);
            let b = pixel(&without, x, y);
            if x < 450 {
                assert_eq!(a, b, "an ordinary item's pixel moved at ({x}, {y})");
                if b != [0, 0, 0, 255] {
                    items += 1;
                }
            } else {
                assert_eq!(b, [0, 0, 0, 255], "an item reached the right half");
                if a != [0, 0, 0, 255] {
                    particles += 1;
                }
            }
        }
    }
    assert!(items > 8_000, "{items} item pixels");
    assert!(particles > 5_000, "{particles} particle pixels");
}

/// A `Text` particle is still a cached record: the native builder does not take text,
/// so a text particle goes the way every text item does — through the glyph run, with
/// its glyphs pinned by the slot the cache stamps for this frame. Forty text particles
/// are forty entries, every one drawn.
#[test]
fn a_text_particles_record_is_still_cached_and_its_glyphs_drawn() {
    let (frame, stats) = draw(
        "canvas_particle_text_cached",
        &scene(
            "  RES face AS canvas::Font = canvas::loadFont(\"fixture.ttf\") TRAP(e)\n    RETURN\n  END TRAP\n  LET label AS canvas::AutoShape = canvas::Text[id := \"\", position := at(0.0, 0.0), text := \"AB\", fonts := [face], fontSize := 30.0, paint := canvas::fill(white()), transform := none()]\n  MUT lip AS canvas::Line\n  LET emitter AS canvas::Shape = WITH lip { startAt := at(0.0 - 400.0, 0.0), endAt := at(360.0, 0.0) }\n  MUT motion AS canvas::Motion\n  MUT emission AS canvas::Emission\n  LET ps AS canvas::DrawItem = canvas::ParticleSystem[position := at(450.0, 320.0), item := label, emitter := emitter, emission := WITH emission { mode := canvas::EmissionMode.Burst, maxParticles := 40, burst := 40, lifetime := 10.0 }, motion := WITH motion { speed := 200.0, spread := 3.2 }, appearance := look(white(), white(), 1.0), seed := 5, time := 1.0]\n  canvas::present([ps])\n",
        ),
        &[],
    );
    let last = stats.last().expect("a frame");
    assert_eq!(stat(last, "entries="), "40", "{last}");
    // Two 9 px squares a particle; they overlap, but forty particles are far more ink
    // than a few.
    assert!(lit(&frame).0 > 2_500, "{} lit pixels", lit(&frame).0);
}
