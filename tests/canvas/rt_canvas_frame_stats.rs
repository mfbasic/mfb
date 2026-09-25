//! `canvas::getFrameStats` (plan-155-B).
//!
//! Every field's contract, run headless: what `presents` and `frames` count and do not
//! count, `dt` as the gap between the last two presents, the one-second window both
//! rates fall out of, the saturation rule that keeps a fast loop measured rather than
//! capped at the ring size, and what the last drawn frame reports.

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

/// Build a `--app` program and run it headless; return its stdout.
fn run(name: &str, source: &str, env: &[(&str, &str)]) -> String {
    run_with_stats(name, source, env).0
}

/// As `run`, plus the `MFB_CANVAS_STATS` lines — the only view of whether a GPU
/// backend was available to draw.
fn run_with_stats(name: &str, source: &str, env: &[(&str, &str)]) -> (String, String) {
    let project = common::temp_project(name, source);
    let stats = project.join("stats.txt");
    let binary = common::build_app_debug(&project, name);
    let run = Command::new(&binary)
        .env("MFB_MACAPP_HEADLESS", "1")
        .env("MFB_WINAPP_HEADLESS", "1")
        .env("MFB_GTKAPP_HEADLESS", "1")
        .env("MFB_CANVAS_STATS", &stats)
        .envs(env.iter().copied())
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", binary.display()));
    let out = String::from_utf8_lossy(&run.stdout).to_string();
    assert!(
        run.status.success(),
        "program {}:\n{out}\n{}",
        common::exit_description(&run.status),
        String::from_utf8_lossy(&run.stderr),
    );
    let lines = std::fs::read_to_string(&stats).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&project);
    (out, lines)
}

/// A program body, wrapped in the canvas-mode boilerplate, with `stats(tag)` printing
/// every field of a snapshot on one line.
fn program(body: &str) -> String {
    format!(
        "IMPORT app\nIMPORT canvas\nIMPORT color\nIMPORT io\nIMPORT os\n\n\
         SUB stats(tag AS String)\n  \
         LET s AS canvas::FrameStats = canvas::getFrameStats()\n  \
         MUT r AS String = \"software\"\n  \
         IF s.renderer = canvas::Renderer.Metal THEN\n    r = \"metal\"\n  END IF\n  \
         IF s.renderer = canvas::Renderer.Vulkan THEN\n    r = \"vulkan\"\n  END IF\n  \
         io::print(tag & \" dt=\" & toString(s.dt) & \" pps=\" & toString(s.pps) & \" fps=\" & \
         toString(s.fps) & \" displayHz=\" & toString(s.displayHz) & \" frameSeconds=\" & \
         toString(s.frameSeconds) & \" presents=\" & toString(s.presents) & \" frames=\" & \
         toString(s.frames) & \" renderer=\" & r)\nEND SUB\n\n\
         FUNC box(x AS Float) AS canvas::DrawItem\n  \
         RETURN canvas::Rectangle[x := x, y := 10.0, w := 20.0, h := 20.0, \
         paint := canvas::fill(color::rgb(255, 0, 0))]\nEND FUNC\n\n\
         SUB main()\n  app::setMode(app::Mode.Canvas)\n{body}END SUB\n"
    )
}

/// The fields of the line tagged `tag`.
fn fields(out: &str, tag: &str) -> Vec<(String, String)> {
    let line = out
        .lines()
        .find(|l| l.starts_with(&format!("{tag} ")))
        .unwrap_or_else(|| panic!("no {tag} line in:\n{out}"));
    line.split_whitespace()
        .skip(1)
        .filter_map(|f| f.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn field(out: &str, tag: &str, name: &str) -> String {
    fields(out, tag)
        .into_iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v)
        .unwrap_or_else(|| panic!("no {name} on the {tag} line:\n{out}"))
}

fn number(out: &str, tag: &str, name: &str) -> f64 {
    let raw = field(out, tag, name);
    raw.parse()
        .unwrap_or_else(|e| panic!("{name}={raw} is not a number ({e}):\n{out}"))
}

#[test]
fn before_any_present_every_field_is_zero() {
    let out = run(
        "canvas_stats_zero",
        &program("  stats(\"before\")\n"),
        &[("MFB_CANVAS_DISPLAY_HZ", "90")],
    );
    for name in ["dt", "pps", "fps", "frameSeconds", "presents", "frames"] {
        assert_eq!(number(&out, "before", name), 0.0, "{name}:\n{out}");
    }
    assert_eq!(field(&out, "before", "renderer"), "software", "{out}");
    assert_eq!(number(&out, "before", "displayHz"), 90.0, "{out}");
}

/// An identical re-present is still a present — it is what the program offered — but it
/// draws nothing, so it is not a frame. Under `MFB_CANVAS_SYNC` each changing present
/// has drawn its frame before `present` returns, so the count is exact.
#[test]
fn identical_presents_count_as_presents_not_frames() {
    let body = "  MUT i AS Integer = 0\n  \
                WHILE i < 5\n    canvas::present([box(toFloat(i) * 30.0)])\n    i = i + 1\n  END WHILE\n  \
                i = 0\n  \
                WHILE i < 5\n    canvas::present([box(120.0)])\n    i = i + 1\n  END WHILE\n  \
                stats(\"after\")\n";
    let out = run(
        "canvas_stats_identical",
        &program(body),
        &[("MFB_CANVAS_SYNC", "1")],
    );
    assert_eq!(number(&out, "after", "presents"), 10.0, "{out}");
    assert_eq!(number(&out, "after", "frames"), 5.0, "{out}");
}

#[test]
fn dt_is_the_gap_between_the_last_two_presents() {
    let body = "  canvas::present([box(0.0)])\n  stats(\"one\")\n  \
                os::sleep(50)\n  canvas::present([box(10.0)])\n  stats(\"two\")\n";
    let out = run("canvas_stats_dt", &program(body), &[]);
    assert_eq!(number(&out, "one", "dt"), 0.0, "{out}");
    let dt = number(&out, "two", "dt");
    assert!(dt >= 0.05, "dt={dt} for presents 50 ms apart:\n{out}");
    assert!(
        dt < 1.0,
        "dt={dt} is not the gap between two presents:\n{out}"
    );
}

/// Both rates cover the trailing second only, so they fall to zero a second after the
/// activity stops — an average updated per event would report the last rate forever.
#[test]
fn rates_fall_to_zero_after_a_second_idle() {
    let body = "  MUT i AS Integer = 0\n  \
                WHILE i < 20\n    canvas::present([box(toFloat(i))])\n    i = i + 1\n  END WHILE\n  \
                stats(\"busy\")\n  os::sleep(1200)\n  stats(\"idle\")\n";
    let out = run(
        "canvas_stats_idle",
        &program(body),
        &[("MFB_CANVAS_SYNC", "1")],
    );
    assert!(number(&out, "busy", "pps") > 0.0, "{out}");
    assert!(number(&out, "busy", "fps") > 0.0, "{out}");
    assert_eq!(number(&out, "idle", "pps"), 0.0, "{out}");
    assert_eq!(number(&out, "idle", "fps"), 0.0, "{out}");
    assert_eq!(number(&out, "idle", "presents"), 20.0, "{out}");
}

/// The ring holds 64 timestamps. A loop presenting faster than 64 a second fills it
/// inside the window, and the rate is then measured from the ring's span — not
/// reported as 64.
#[test]
fn pps_above_the_ring_is_measured_not_capped() {
    let body = "  MUT i AS Integer = 0\n  \
                WHILE i < 500\n    canvas::present([box(5.0)])\n    i = i + 1\n  END WHILE\n  \
                stats(\"fast\")\n";
    let out = run("canvas_stats_fast", &program(body), &[]);
    let pps = number(&out, "fast", "pps");
    assert!(
        pps > 64.0,
        "500 back-to-back presents reported pps={pps}:\n{out}"
    );
    assert_eq!(number(&out, "fast", "presents"), 500.0, "{out}");
}

#[test]
fn frame_seconds_and_renderer_after_a_drawn_frame() {
    let body = "  canvas::present([box(0.0)])\n  stats(\"drawn\")\n";
    let out = run(
        "canvas_stats_drawn",
        &program(body),
        &[("MFB_CANVAS_SYNC", "1")],
    );
    assert!(number(&out, "drawn", "frameSeconds") > 0.0, "{out}");
    assert_eq!(number(&out, "drawn", "frames"), 1.0, "{out}");
    assert_eq!(field(&out, "drawn", "renderer"), "software", "{out}");

    // The GPU arm, gated like the graphics-thread suite's Metal cases: only on macOS,
    // and skipped only when the stats line says no Metal pipeline could be built.
    if cfg!(target_os = "macos") {
        let (out, stats) = run_with_stats(
            "canvas_stats_drawn_metal",
            &program(body),
            &[("MFB_CANVAS_SYNC", "1"), ("MFB_CANVAS_GPU", "1")],
        );
        if stats
            .lines()
            .last()
            .is_some_and(|l| l.contains("metalReady=TRUE"))
        {
            assert_eq!(field(&out, "drawn", "renderer"), "metal", "{out}\n{stats}");
            assert!(number(&out, "drawn", "frameSeconds") > 0.0, "{out}");
        }
    }
}

/// Seconds a run took, measured around the whole process — the pacing tests assert a
/// lower bound only, so process start-up and shutdown only ever make them safer.
fn timed(name: &str, source: &str, env: &[(&str, &str)]) -> (String, String, f64) {
    let started = std::time::Instant::now();
    let (out, stats) = run_with_stats(name, source, env);
    (out, stats, started.elapsed().as_secs_f64())
}

/// A loop of `present` + `waitFrame` is paced by the display interval: at 50 Hz, 25
/// iterations cannot finish in under 480 ms (24 full intervals after the first return).
#[test]
fn wait_frame_paces_a_changing_scene() {
    let body = "  canvas::present([box(0.0)])\n  canvas::waitFrame()\n  \
                LET t0 AS Integer = datetime::monotonicNanos()\n  \
                MUT i AS Integer = 0\n  \
                WHILE i < 25\n    canvas::present([box(toFloat(i) * 4.0)])\n    canvas::waitFrame()\n    i = i + 1\n  END WHILE\n  \
                io::print(\"loopMs=\" & toString((datetime::monotonicNanos() - t0) / 1000000))\n";
    let source = program(body).replace("IMPORT io\n", "IMPORT io\nIMPORT datetime\n");
    let (out, _, _) = timed(
        "canvas_wait_changing",
        &source,
        &[("MFB_CANVAS_DISPLAY_HZ", "50")],
    );
    let ms = loop_ms(&out);
    assert!(ms >= 480, "25 paced frames at 50 Hz took {ms} ms:\n{out}");
}

/// The spin guard. An unchanged scene publishes nothing, so no redraw is ever owed — the
/// floor alone must still hold the loop to the display rate.
#[test]
fn wait_frame_paces_an_unchanged_scene() {
    let body = "  canvas::present([box(0.0)])\n  canvas::waitFrame()\n  \
                LET t0 AS Integer = datetime::monotonicNanos()\n  \
                MUT i AS Integer = 0\n  \
                WHILE i < 25\n    canvas::present([box(0.0)])\n    canvas::waitFrame()\n    i = i + 1\n  END WHILE\n  \
                io::print(\"loopMs=\" & toString((datetime::monotonicNanos() - t0) / 1000000))\n";
    let source = program(body).replace("IMPORT io\n", "IMPORT io\nIMPORT datetime\n");
    let (out, _, _) = timed(
        "canvas_wait_unchanged",
        &source,
        &[("MFB_CANVAS_DISPLAY_HZ", "50")],
    );
    let ms = loop_ms(&out);
    assert!(
        ms >= 480,
        "25 unchanged presents at 50 Hz took {ms} ms — a spin:\n{out}"
    );
}

/// R23: `waitFrame` before any present returns (nothing is owed, so only the floor
/// applies) and the program exits normally — the call starts the graphics thread
/// itself, so there is a mutex to wait under.
#[test]
fn wait_frame_before_any_present_returns() {
    let body = "  canvas::waitFrame()\n  io::print(\"returned\")\n";
    let out = run("canvas_wait_first", &program(body), &[]);
    assert!(out.contains("returned"), "{out}");
}

/// R24: `waitFrame` waits for the renderer to PICK UP the redraw, not only for the floor.
/// With the display floor at 1 ms and every frame held 100 ms inside the renderer, each
/// present after the first finds the previous frame still in the hold, so its pickup —
/// and the `waitFrame` behind it — waits for that frame to finish: five presents after
/// the first cannot take under 400 ms.
#[test]
fn wait_frame_waits_for_pickup() {
    let body = "  LET t0 AS Integer = datetime::monotonicNanos()\n  \
                MUT i AS Integer = 0\n  \
                WHILE i < 6\n    canvas::present([box(toFloat(i) * 10.0)])\n    canvas::waitFrame()\n    i = i + 1\n  END WHILE\n  \
                io::print(\"loopMs=\" & toString((datetime::monotonicNanos() - t0) / 1000000))\n";
    let source = program(body).replace("IMPORT io\n", "IMPORT io\nIMPORT datetime\n");
    let (out, _, _) = timed(
        "canvas_wait_pickup",
        &source,
        &[
            ("MFB_CANVAS_DISPLAY_HZ", "1000"),
            ("MFB_CANVAS_FRAME_HOLD_MS", "100"),
        ],
    );
    let ms = loop_ms(&out);
    assert!(
        ms >= 400,
        "six held frames with a 1 ms floor took {ms} ms:\n{out}"
    );
}

/// R25: a resize landing while the loop waits neither hangs it nor is lost. The scripted
/// resize (`MFB_CANVAS_RESIZE_W`/`_H`) exists on the GTK and Windows backends only; there
/// the stats must show a frame drawn at the new size. On macOS, which has no scripted
/// resize, the loop finishing is the whole claim.
#[test]
fn wait_frame_survives_a_resize() {
    let body = "  MUT i AS Integer = 0\n  \
                WHILE i < 40\n    canvas::present([box(toFloat(i) * 2.0)])\n    canvas::waitFrame()\n    i = i + 1\n  END WHILE\n  \
                io::print(\"finished\")\n";
    let (out, stats) = run_with_stats(
        "canvas_wait_resize",
        &program(body),
        &[
            ("MFB_CANVAS_DISPLAY_HZ", "100"),
            ("MFB_CANVAS_RESIZE_W", "640"),
            ("MFB_CANVAS_RESIZE_H", "480"),
        ],
    );
    assert!(out.contains("finished"), "{out}");
    if !cfg!(target_os = "macos") {
        assert!(
            stats.lines().any(|l| l.contains("damage=0,0,640,480")),
            "no frame was drawn at the resized 640x480:\n{stats}"
        );
    }
}

fn loop_ms(out: &str) -> i64 {
    out.lines()
        .find_map(|l| l.strip_prefix("loopMs="))
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or_else(|| panic!("no loopMs= line in:\n{out}"))
}
