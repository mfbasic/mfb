//! plan-176-D: `rt_canvas_particles`'s system lifecycle tests (present, copy, drop,
//! leaks, raises, the program's clock) and the emission model tests driven by
//! `MODEL_DRIVER`. The render helpers live in the root, `rt_canvas_particles.rs`.

use super::*;

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
  LET e5 AS canvas::Shape = canvas::Text[id := "", position := origin(), text := "AB", fonts := [fnt], fontSize := 24.0, letterSpacing := 0.0, paint := canvas::fill(color::rgb(0, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
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
