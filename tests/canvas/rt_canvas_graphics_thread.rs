//! The canvas render loop runs on its own thread (plan-98-D Phase 2).
//!
//! The claims here are about *when and how often* a frame happens, which is
//! invisible in the pixels — an identical picture results whether it was drawn once
//! or a hundred times, on the worker or on a graphics thread. `MFB_CANVAS_STATS`
//! appends one line per rendered frame, so the frame count is observable; and
//! `MFB_CANVAS_SYNC` makes `present` wait for the frame it asked for, so a test can
//! assert per-present behaviour without racing the scheduler.
//!
//! `.ai/canvas-threading.md` is the protocol these check.

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

/// Build a `--app` program and run it headless, returning `(stdout, frame lines)`.
///
/// `sync` selects whether `present` waits for its frame. Both modes are exercised:
/// the deterministic one for per-present claims, the default one for the claim that
/// a program does not *need* to wait.
fn run(name: &str, source: &str, sync: bool) -> (String, Vec<String>) {
    run_with(name, source, sync, false)
}

/// As `run`, plus the `MFB_CANVAS_GPU` selector (plan-98-E).
fn run_with(name: &str, source: &str, sync: bool, metal: bool) -> (String, Vec<String>) {
    let gpu: &[(&str, &str)] = if metal {
        &[("MFB_CANVAS_GPU", "1")]
    } else {
        &[]
    };
    run_env(name, source, sync, gpu)
}

/// As `run`, plus arbitrary extra environment variables.
fn run_env(name: &str, source: &str, sync: bool, env: &[(&str, &str)]) -> (String, Vec<String>) {
    let project = common::temp_project(name, source);
    let stats = project.join("stats.txt");
    let binary = common::build_app_debug(&project, name);
    let mut command = Command::new(&binary);
    command
        .env("MFB_HEADLESS", "1")
        .env("MFB_CANVAS_STATS", &stats);
    if sync {
        command.env("MFB_CANVAS_SYNC", "1");
    }
    command.envs(env.iter().copied());
    let run = command
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", binary.display()));
    assert!(
        run.status.success(),
        "program {}:\n{}\n{}",
        common::exit_description(&run.status),
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr),
    );

    let lines = std::fs::read_to_string(&stats)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect();
    let out = String::from_utf8_lossy(&run.stdout).to_string();
    let _ = std::fs::remove_dir_all(&project);
    (out, lines)
}

/// A program body, wrapped in the canvas-mode boilerplate.
fn program(body: &str) -> String {
    format!(
        "IMPORT app\nIMPORT canvas\nIMPORT vector\nIMPORT color\nIMPORT io\n\nFUNC main() AS Nothing\n  \
         app::setMode(app::Mode.Canvas)\n{body}END FUNC\n"
    )
}

const ONE_BOX: &str =
    "  LET box AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 10.0, y := 10.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 50.0, height := 50.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n";

/// A program that presents and returns immediately still gets its frame drawn.
///
/// This is the one that matters most, and the one that was broken: the render is
/// asynchronous now, so `main` returning starts the shutdown while the graphics
/// thread may not have woken yet. Shutdown must **drain** the pending frame rather
/// than cancel it — the first version checked "stopping" before "pending" and dropped
/// the frame, nondeterministically. Run from a shell it drew; run under `cargo test`
/// it did not.
///
/// Deliberately **not** in sync mode: waiting for the frame is exactly what a program
/// must not have to do.
#[test]
fn a_present_then_immediate_return_still_renders() {
    let (stdout, frames) = run(
        "canvas_gfx_drain",
        &program(&format!(
            "{ONE_BOX}  canvas::present([box])\n  io::print(\"done\")\n"
        )),
        false,
    );
    assert!(
        stdout.contains("done"),
        "the program must run to completion"
    );
    assert_eq!(
        frames.len(),
        1,
        "the pending frame must be drained by shutdown, not cancelled: {frames:?}",
    );
}

/// A program that never presents starts no graphics thread and draws nothing.
///
/// The thread is spawned lazily by the first `present`, so entering canvas mode and
/// doing nothing costs neither a thread nor a frame. It also proves the shutdown join
/// is a no-op when nothing was started — otherwise this would hang.
#[test]
fn canvas_mode_without_a_present_renders_nothing() {
    let (stdout, frames) = run(
        "canvas_gfx_idle",
        &program("  io::print(\"idle\")\n"),
        false,
    );
    assert!(stdout.contains("idle"));
    assert!(
        frames.is_empty(),
        "no present means no frame and no graphics thread: {frames:?}",
    );
}

/// A static scene costs exactly one frame, however long the program lives.
///
/// Time is deliberately not a redraw trigger (`.ai/canvas-threading.md` §4), so the
/// render loop must be a real condition wait rather than a poll. A spinning loop
/// would show many frames here — and would also burn a core.
#[test]
fn a_static_scene_renders_once_and_does_not_spin() {
    let source = program(&format!(
        "{ONE_BOX}  canvas::present([box])\n  \
         LET deadline AS Integer = datetime::monotonicNanos() + 1000000000\n  \
         MUT spins AS Integer = 0\n  \
         WHILE datetime::monotonicNanos() < deadline\n    spins = spins + 1\n  END WHILE\n  \
         io::print(\"waited\")\n"
    ))
    .replace("IMPORT io\n", "IMPORT io\nIMPORT datetime\n");
    let (stdout, frames) = run("canvas_gfx_static", &source, false);
    assert!(stdout.contains("waited"));
    assert_eq!(
        frames.len(),
        1,
        "a scene that never changes must not be redrawn while the program idles: \
         {frames:?}",
    );
}

/// plan-155-C: the same guarantee under a `present` + `waitFrame` loop. `waitFrame`
/// waits and never repaints (`.ai/canvas-threading.md` §9), so fifty paced iterations
/// over an unchanged scene still cost exactly one frame.
#[test]
fn a_static_scene_under_a_wait_frame_loop_renders_once() {
    let source = program(&format!(
        "{ONE_BOX}  MUT i AS Integer = 0\n  \
         WHILE i < 50\n    canvas::present([box])\n    canvas::waitFrame()\n    i = i + 1\n  END WHILE\n  \
         io::print(\"waited\")\n"
    ));
    let (stdout, frames) = run_env(
        "canvas_gfx_static_wait",
        &source,
        false,
        &[("MFB_CANVAS_DISPLAY_HZ", "100")],
    );
    assert!(stdout.contains("waited"));
    assert_eq!(
        frames.len(),
        1,
        "waitFrame must not redraw an unchanged scene: {frames:?}",
    );
}

/// plan-155-A: `MFB_CANVAS_DISPLAY_HZ` pins the display rate the runtime reports.
///
/// A headless run has no monitor to ask, so without the override the published rate
/// is `0` and nothing downstream (plan-155-B's `displayHz`, C's `waitFrame` floor) could
/// be tested with a known rate. The stats line is the only window onto it.
#[test]
fn display_hz_override_reaches_the_stats_line() {
    let (_, frames) = run_env(
        "canvas_gfx_display_hz",
        &program(&format!("{ONE_BOX}  canvas::present([box])\n")),
        true,
        &[("MFB_CANVAS_DISPLAY_HZ", "144")],
    );
    assert_eq!(frames.len(), 1, "{frames:?}");
    let hz: f64 = stat_field(&frames[0], "displayHz=")
        .parse()
        .unwrap_or_else(|e| panic!("displayHz= is not a number ({e}): {frames:?}"));
    assert_eq!(hz, 144.0, "{frames:?}");
}

/// An identical re-present publishes nothing, so it draws nothing.
///
/// The frame skip is plan-98-B's, and it has to survive the move to a graphics
/// thread: `present` only signals when `publishScene` reports it actually published.
#[test]
fn an_identical_re_present_draws_no_second_frame() {
    let (_, frames) = run(
        "canvas_gfx_skip",
        &program(&format!(
            "{ONE_BOX}  canvas::present([box])\n  canvas::present([box])\n  \
             canvas::present([box])\n  io::print(\"done\")\n"
        )),
        true,
    );
    assert_eq!(
        frames.len(),
        1,
        "three presents of identical content must publish — and draw — once: \
         {frames:?}",
    );
}

/// The frame skip compares what a scene SAYS, not how its lists were allocated
/// (bug-968).
///
/// A nested list is embedded in its item with the capacity it was built with
/// (`mfb spec memory collections`), and a list entry has bytes no list writes, so two
/// scenes with the same content are different bytes whenever a nested list was built
/// another way. `scenes/frame_skip_nested_lists.mfb` builds one item for every
/// list-carrying field (a polygon's contours, a polyline's points and colours, a
/// gradient's stops, a paint's clip and dashes, a curve's points and weights, a curve
/// polygon's contours, and a polygon inside an `AutoPaint`) beside a group node, three
/// ways each (a literal, `collections::append`, and `collections::set` with a
/// `removeAt`), and presents them flat and inside layers. It prints the frame counter
/// after every present, so each line is one row of the matrix.
///
/// The second half is the one a comparison that reported "same" too often would fail:
/// one value changed deep inside each nested list, one letter of a name and one more
/// letter, one more contour, one more point, and another variant with the same points
/// must each draw a frame.
#[test]
fn content_equal_scenes_with_nested_lists_draw_one_frame() {
    let (out, _) = run(
        "canvas_gfx_skip_nested",
        include_str!("scenes/frame_skip_nested_lists.mfb"),
        true,
    );
    // The scene's `vary` values: one per nested list, then two string changes.
    const VARIED: usize = 11;
    let mut want = Vec::new();
    let mut frame = 0;
    for shape in ["flat", "layers"] {
        frame += 1;
        for how in 0..3 {
            want.push(format!("same {shape} how={how} {frame}"));
        }
        for vary in 1..=VARIED {
            frame += 1;
            for how in 0..3 {
                let prefix = if shape == "flat" { "" } else { "layers " };
                want.push(format!("{prefix}vary={vary} how={how} {frame}"));
            }
        }
        if shape == "flat" {
            want.push(format!("one polygon {}", frame + 1));
            want.push(format!("one polygon again {}", frame + 1));
            want.push(format!("two contours {}", frame + 2));
            want.push(format!("four points {}", frame + 3));
            want.push(format!("another variant {}", frame + 4));
            want.push(format!("another variant again {}", frame + 4));
            frame += 4;
        } else {
            want.push(format!("one layer {}", frame + 1));
            want.push(format!("one layer again {}", frame + 1));
        }
    }
    let got: Vec<String> = out.lines().map(str::to_string).collect();
    assert_eq!(
        got, want,
        "each line is `<what was presented> <frames drawn so far>`: a scene with the \
         same content as the one installed must draw nothing, however its lists were \
         built, and a scene that differs anywhere must draw one frame",
    );
}

/// In sync mode every present gets its own frame, which is what makes the
/// per-present assertions elsewhere deterministic.
///
/// Without it the frame count is a scheduling detail: presents that arrive between
/// two frames coalesce by design, and the same three-present program was observed
/// producing one, two and three frames across runs.
#[test]
fn sync_mode_gives_one_frame_per_changed_present() {
    let body = format!(
        "{ONE_BOX}  canvas::present([box])\n  \
         LET two AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 80.0, y := 10.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 50.0, height := 50.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(0, 255, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
         canvas::present([box, two])\n  \
         LET three AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 150.0, y := 10.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 50.0, height := 50.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(0, 0, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
         canvas::present([box, two, three])\n  io::print(\"done\")\n"
    );
    let (_, frames) = run("canvas_gfx_sync", &program(&body), true);
    assert_eq!(
        frames.len(),
        3,
        "sync mode must give each changed present its own frame: {frames:?}",
    );
}

/// D's frame counter, driven through the Metal renderer, still advances once per
/// changed present — and only after the GPU has finished the frame.
///
/// This is plan-98-E Phase 3's claim (Correction 12). The counter *is*
/// `lastCompletedFrame`: `__canvas_renderLoop` calls `canvas::frameDone()` after
/// `__canvas_renderFrame()` returns, and on the Metal path that return is behind
/// `[commandBuffer waitUntilCompleted]` — the GPU has finished before the counter can
/// move. Every consumer D built on it (the scene ring's retirement gate,
/// `MFB_CANVAS_SYNC`) therefore sees GPU-completion ordering with no change of its
/// own, which is exactly what "renderer-swappable frame-completion signal" was
/// supposed to buy.
///
/// Counting frames is what makes that observable from outside: a counter that moved
/// early would produce more frames than presents, and one that never moved would hang
/// the sync wait.
#[test]
fn the_metal_path_gives_one_completed_frame_per_changed_present() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let body = format!(
        "{ONE_BOX}  canvas::present([box])\n  \
         LET two AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 80.0, y := 10.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 50.0, height := 50.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(0, 255, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
         canvas::present([box, two])\n  \
         LET three AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 150.0, y := 10.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 50.0, height := 50.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(0, 0, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
         canvas::present([box, two, three])\n  io::print(\"done\")\n"
    );
    let (out, frames) = run_with("canvas_gfx_sync_metal", &program(&body), true, true);
    if !frames
        .last()
        .is_some_and(|line| line.contains("metalReady=TRUE"))
    {
        return; // no Metal device on this host
    }
    assert!(
        out.contains("done"),
        "the program must run to completion: {out}"
    );
    assert_eq!(
        frames.len(),
        3,
        "each changed present must produce exactly one completed Metal frame: {frames:?}",
    );
}

/// The scene ring's frame skip survives the Metal renderer.
///
/// The skip is decided in `canvas::present` on the *worker*, so it should be
/// renderer-independent — but "should be" is the claim under test. If the Metal path
/// advanced the frame counter differently the retirement gate would drain at a
/// different rate, and three identical presents would stop collapsing to one frame.
#[test]
fn an_identical_re_present_draws_no_second_metal_frame() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let (_, frames) = run_with(
        "canvas_gfx_skip_metal",
        &program(&format!(
            "{ONE_BOX}  canvas::present([box])\n  canvas::present([box])\n  \
             canvas::present([box])\n  io::print(\"done\")\n"
        )),
        true,
        true,
    );
    if !frames
        .last()
        .is_some_and(|line| line.contains("metalReady=TRUE"))
    {
        return;
    }
    assert_eq!(
        frames.len(),
        1,
        "three identical presents must publish — and draw — once on Metal too: \
         {frames:?}",
    );
}

/// The renderer seam reports the device this host actually has, and selecting it
/// is opt-in.
///
/// This is plan-98-E's first checkable claim, and it is deliberately about the
/// *plumbing* rather than about pixels: on macOS `metal=TRUE` means
/// `Metal.framework`'s install name resolved, its import-table row bound, and
/// `MTLCreateSystemDefaultDevice` returned a device. Every later piece of the Metal
/// backend rests on those three, and a failure in any of them is far cheaper to read
/// here than as a blank window several hundred lines later.
///
/// Off macOS the same field must read FALSE. That is not a weaker claim, it is the
/// other half of the same one: there is no `Metal.framework` on Windows or Linux, so
/// a `metal=TRUE` there would mean the field is fabricated rather than measured —
/// and a fabricated readiness flag is exactly what makes a GPU suite pass while
/// never touching the GPU.
///
/// `gpuSelected` must be FALSE by default on every platform. The software renderer
/// is the oracle the GPU path is measured against (plan-98-A invariant 7) and its
/// goldens are exact-match; if selecting the GPU were the default, every one of
/// those goldens would silently become a tolerance test against a reference that no
/// longer exists.
///
/// Whether `MFB_CANVAS_GPU` then *succeeds* depends on there being a device to
/// select, so that half is gated on the run's own readiness field — the same way
/// [`the_metal_path_gives_one_completed_frame_per_changed_present`] gates itself,
/// and for the same reason: a host with no GPU must not be able to fail this, and
/// a host with one must not be able to skip it.
#[test]
fn the_renderer_seam_reports_its_device_and_leaves_it_unselected() {
    let source = program(&format!(
        "{ONE_BOX}  canvas::present([box])\n  io::print(\"done\")\n"
    ));
    let field = |lines: &[String], name: &str| -> String {
        lines
            .first()
            .and_then(|l| {
                l.split_whitespace()
                    .find_map(|f| f.strip_prefix(name).map(str::to_string))
            })
            .unwrap_or_else(|| panic!("no {name} field in {lines:?}"))
    };

    let (_, default_run) = run_with("canvas_gpu_default", &source, true, false);
    assert_eq!(
        field(&default_run, "metal="),
        if cfg!(target_os = "macos") {
            "TRUE"
        } else {
            "FALSE"
        },
        "the metal field must report this host: MTLCreateSystemDefaultDevice binds \
         and returns a device on macOS, and there is no Metal to find anywhere else",
    );
    assert_eq!(
        field(&default_run, "gpuSelected="),
        "FALSE",
        "the software renderer must stay the default — it is the exact-match oracle",
    );

    let (_, selected) = run_with("canvas_gpu_selected", &source, true, true);
    let ready =
        field(&selected, "metalReady=") == "TRUE" || field(&selected, "vulkanReady=") == "TRUE";
    if !ready {
        eprintln!("skip: this host built no GPU pipeline\n{selected:?}");
        return;
    }
    assert_eq!(
        field(&selected, "gpuSelected="),
        "TRUE",
        "MFB_CANVAS_GPU must select the GPU renderer on a host that has one",
    );
}

/// One field of one stats line, by name.
///
/// Free rather than a closure inside a test, because the group tests below read the
/// *second* line as well as the first — the whole point of a frame-skip assertion is
/// what the later frames say.
fn stat_field(line: &str, name: &str) -> String {
    line.split_whitespace()
        .find_map(|f| f.strip_prefix(name).map(str::to_string))
        .unwrap_or_else(|| panic!("no {name} field in {line:?}"))
}

/// A group's contents changing must produce a second frame, even though the scene
/// list handed to `present` is byte-for-byte the one already published.
///
/// This is plan-116-G §4.4's least obvious requirement and the reason it is written
/// before any of the feature exists. `publishScene` decides whether to redraw by
/// comparing the raw bytes of the `DrawItem` list's data region
/// (`emit_compare_bytes_branch` in `gen_present.rs`). A `Group` node is `dx`, `dy` and
/// `name` — and after `setGroup("panel", …)` installs *different* items under the same
/// name, all three of those are unchanged. Two presents of the same list therefore
/// compare equal and the second is skipped, so the program draws the OLD panel forever
/// and nothing raises: a stale picture reported as success.
///
/// §4.4's answer is a parallel resolved-groups signature — `(slotIndex, revision)` per
/// resolved group node, in scene order — published and compared alongside the items.
/// This test does not care how it is done; it pins the observable consequence.
///
/// `A` is a box at the top-left and `A'` a box far away at the bottom-right, so
/// "the second frame shows A'" is readable from `damage=` without pixel access. If the
/// signature is missing, this fails at the frame count; if the signature exists but is
/// compared instead of the items rather than as well, the sibling below fails.
///
/// Un-ignored by **Phase 4**, which lands the resolution pass. Until then `setGroup`
/// does not exist and this cannot compile as a program, which is why it is `#[ignore]`d
/// rather than merely failing.
#[test]
#[ignore = "plan-116-G Phase 4 lands the resolution pass that makes this pass"]
fn a_group_replaced_between_two_identical_presents_draws_a_second_frame() {
    let body = "  LET a AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 10.0, y := 10.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 50.0, height := 50.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
         canvas::setGroup(\"panel\", [a])\n  \
         LET node AS canvas::DrawItem = canvas::Group[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"panel\"]\n  \
         canvas::present([node])\n  \
         LET b AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 700.0, y := 500.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 50.0, height := 50.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(0, 255, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
         canvas::setGroup(\"panel\", [b])\n  \
         canvas::present([node])\n  io::print(\"done\")\n";
    let (_, frames) = run("canvas_group_revision", &program(body), true);

    assert_eq!(
        frames.len(),
        2,
        "replacing a group's contents must redraw, even though the scene list handed \
         to the second `present` is byte-identical to the first: the group's revision \
         has to reach the content comparison, or the program draws the old panel \
         forever and nothing raises. {frames:?}",
    );

    let first = stat_field(&frames[0], "damage=");
    let second = stat_field(&frames[1], "damage=");
    assert_ne!(
        first, second,
        "the second frame drew the same region as the first, so it redrew the OLD \
         group: a revision that reaches the skip comparison but not the resolved \
         buffer is the failure this distinguishes. A is at (10,10) and A' at \
         (700,500), so the two damage rectangles cannot coincide if A' was drawn.",
    );
}

/// Its sibling, and the half that a careless fix breaks: with no `setGroup` between
/// them, two identical presents still draw **once**.
///
/// The cheap way to pass the test above is to stop comparing, or to fold something
/// per-frame-varying into the signature — either of which turns every group program
/// into an unconditional redraw and silently undoes the skip that
/// `an_identical_re_present_draws_no_second_frame` protects for group-free scenes.
/// Pairing the two is what makes the requirement two-sided.
///
/// Un-ignored by **Phase 4**, with its sibling.
#[test]
#[ignore = "plan-116-G Phase 4 lands the resolution pass that makes this pass"]
fn two_identical_presents_of_an_unchanged_group_draw_one_frame() {
    let body = "  LET a AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 10.0, y := 10.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 50.0, height := 50.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
         canvas::setGroup(\"panel\", [a])\n  \
         LET node AS canvas::DrawItem = canvas::Group[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"panel\"]\n  \
         canvas::present([node])\n  \
         canvas::present([node])\n  \
         canvas::present([node])\n  io::print(\"done\")\n";
    let (_, frames) = run("canvas_group_no_revision", &program(body), true);

    assert_eq!(
        frames.len(),
        1,
        "three presents of an unchanged group must draw once: the resolved-groups \
         signature has to compare EQUAL when nothing changed, or every group program \
         redraws every present forever. {frames:?}",
    );
}

// --- bug-484: a Picture's image and the graphics thread (race rows R1, R2, R11) -------

const FRAME_W: usize = 900;
const FRAME_H: usize = 640;

/// Build a picture program, run it headless with `env`, and return the last dumped frame.
fn picture_frame(name: &str, body: &str, env: &[(&str, &str)]) -> Vec<u8> {
    let source = format!(
        "IMPORT app\nIMPORT canvas\nIMPORT vector\nIMPORT color\nIMPORT os\n\nFUNC main() AS Nothing\n  \
         app::setMode(app::Mode.Canvas)\n  \
         RES img AS canvas::Image = canvas::createImage(1, 1, [toByte(0), toByte(255), toByte(0), toByte(255)])\n  \
         LET tile AS canvas::DrawItem = canvas::Picture[id := \"\", shape := canvas::Rectangle[id := \"\", position := vector::Float2[x := 100.0, y := 100.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 32.0, height := 32.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]], clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img]\n  \
         LET mark AS canvas::DrawItem = canvas::Circle[id := \"\", position := vector::Float2[x := 700.0, y := 500.0], radius := 30.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n\
         {body}END FUNC\n"
    );
    let project = common::temp_project(name, &source);
    let frame_path = project.join("frame.rgba");
    let binary = common::build_app_debug(&project, name);
    let mut command = Command::new(&binary);
    command
        .env("MFB_HEADLESS", "1")
        .env("MFB_CANVAS_DUMP", &frame_path);
    for (key, value) in env {
        command.env(key, value);
    }
    let run = command
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", binary.display()));
    assert!(
        run.status.success(),
        "the program did not exit cleanly — a read of a destroyed image's pixels is what \
         these rows exist to catch, and it presents as a signal here. {}\n{}\n{}",
        common::exit_description(&run.status),
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr),
    );
    let frame = std::fs::read(&frame_path).expect("a frame must have been written");
    assert_eq!(
        frame.len(),
        FRAME_W * FRAME_H * 4,
        "the frame was torn or truncated"
    );
    let _ = std::fs::remove_dir_all(&project);
    frame
}

fn rgba_at(frame: &[u8], x: usize, y: usize) -> (u8, u8, u8, u8) {
    let i = (y * FRAME_W + x) * 4;
    (frame[i], frame[i + 1], frame[i + 2], frame[i + 3])
}

/// R1: `destroyImage` while a frame that draws the image is in flight — the frame
/// completes normally and still shows the picture.
///
/// `MFB_CANVAS_FRAME_HOLD_MS` parks the graphics thread after the geometry is built, so
/// the destroy lands while the frame is demonstrably still working from the image; the
/// worker's 120 ms sleep keeps it from winning the race to the frame's start (see
/// `removing_a_group_mid_frame_lets_the_frame_finish` in `rt_canvas_rasteriser.rs`).
/// The frame samples a pixel block the close did not free — closing an `Image` sets a
/// flag and nothing else (`.ai/canvas-threading.md` section 7, "close never frees").
#[test]
fn destroying_an_image_mid_frame_lets_the_frame_finish() {
    let frame = picture_frame(
        "canvas_picture_r1",
        "  canvas::present([tile, mark])\n  os::sleep(120)\n  canvas::destroyImage(img)\n  os::sleep(1200)\n",
        &[("MFB_CANVAS_FRAME_HOLD_MS", "600")],
    );
    assert_eq!(
        rgba_at(&frame, 116, 116),
        (0, 255, 0, 255),
        "the frame already drawing when destroyImage arrived must finish drawing the picture"
    );
    assert_eq!(
        rgba_at(&frame, 700, 500),
        (255, 0, 0, 255),
        "the rest of the frame was lost"
    );
}

/// R2: the frame AFTER the destroy skips the picture, and the rest of it draws.
#[test]
fn the_frame_after_a_destroy_skips_the_picture() {
    let frame = picture_frame(
        "canvas_picture_r2",
        "  canvas::present([tile, mark])\n  os::sleep(120)\n  canvas::destroyImage(img)\n  os::sleep(1200)\n  \
         LET moved AS canvas::DrawItem = canvas::Circle[id := \"\", position := vector::Float2[x := 600.0, y := 500.0], radius := 30.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
         canvas::present([tile, moved])\n  os::sleep(1200)\n",
        &[("MFB_CANVAS_FRAME_HOLD_MS", "600")],
    );
    assert_eq!(
        rgba_at(&frame, 116, 116),
        (0, 0, 0, 255),
        "a destroyed image draws nothing in a new frame"
    );
    assert_eq!(
        rgba_at(&frame, 600, 500),
        (255, 0, 0, 255),
        "the frame around it rendered"
    );
}

/// R11: `setBytes` then `destroyImage` then a frame — the closed image draws nothing and
/// nothing reads past the close.
#[test]
fn set_bytes_then_destroy_then_a_frame_draws_nothing() {
    let frame = picture_frame(
        "canvas_picture_r11",
        "  canvas::present([tile, mark])\n  \
         canvas::setBytes(img, [toByte(0), toByte(0), toByte(255), toByte(255)])\n  \
         canvas::destroyImage(img)\n  \
         LET moved AS canvas::DrawItem = canvas::Circle[id := \"\", position := vector::Float2[x := 600.0, y := 500.0], radius := 30.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
         canvas::present([tile, moved])\n",
        &[("MFB_CANVAS_SYNC", "1")],
    );
    assert_eq!(
        rgba_at(&frame, 116, 116),
        (0, 0, 0, 255),
        "the destroyed image drew"
    );
    assert_eq!(
        rgba_at(&frame, 600, 500),
        (255, 0, 0, 255),
        "the frame around it rendered"
    );
}

// ---------------------------------------------------------------------------------
// plan-172-K: `canvas::AutoPaint` — a frame the worker did not ask for, and a wait that
// can run out. Rows R26–R29 of `.ai/canvas-threading.md` section 8.
// ---------------------------------------------------------------------------------

/// One `AutoPaint` button at x 100..300, y 100..180 (moved to `x` by `button(x)`), its
/// four `use…` flags read from `FLAGS` (1 enter, 2 exit, 4 hover, 8 pressed). Each test
/// below appends its own `main`.
const AUTO_BUTTON: &str = r#"IMPORT app
IMPORT canvas
IMPORT color
IMPORT datetime
IMPORT io
IMPORT os
IMPORT vector

FUNC flag(bit AS Integer) AS Boolean
  RETURN (toInt(os::getEnvOr("FLAGS", "15")) / bit) MOD 2 = 1
END FUNC

FUNC button(x AS Float) AS canvas::DrawItem
  MUT r AS canvas::Rectangle
  r = WITH r { id := "btn", position := vector::Float2[x := x, y := 100.0], size := canvas::Size[width := 200.0, height := 80.0], paint := canvas::fill(color::rgb(40, 90, 200)) }
  LET a AS canvas::AutoPaint = canvas::AutoPaint[shape := r, useEnter := flag(1), useExit := flag(2), useHover := flag(4), usePressed := flag(8), enterPaint := canvas::fill(color::rgb(0, 255, 0)), exitPaint := canvas::fill(color::rgb(255, 255, 0)), hoverPaint := canvas::fill(color::rgb(255, 0, 0)), pressedPaint := canvas::fill(color::rgb(255, 0, 255))]
  LET item AS canvas::DrawItem = a
  RETURN item
END FUNC

"#;

/// The `auto=` field of each stats line, a run of equal values written once.
fn auto_states(lines: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in lines {
        let value = stat_field(line, "auto=");
        if out.last() != Some(&value) {
            out.push(value);
        }
    }
    out
}

/// R26: the pointer changes while a frame is in flight. The frame completes with the
/// state it resolved, and the next frame has the new one.
///
/// A frame reads the pointer ONCE, at its resolve, and everything it draws follows from
/// that one reading. Here the frame is held for a second after its resolve
/// (`MFB_CANVAS_FRAME_HOLD_MS`) and the pointer's next position falls due 300 ms into
/// the hold. The held frame's line still says hover, at the position it read; the
/// frame after it says the shape's own paint, at the new one — and that second frame
/// was asked for by nobody but the pointer.
#[test]
fn auto_a_pointer_change_during_a_frame_belongs_to_the_next_frame() {
    let source =
        format!("{AUTO_BUTTON}FUNC main() AS Nothing\n  app::setMode(app::Mode.Canvas)\n  canvas::present([button(100.0)])\n  os::sleep(3000)\nEND FUNC\n");
    let (_, lines) = run_env(
        "canvas_auto_r26",
        &source,
        true,
        &[
            ("FLAGS", "4"),
            ("MFB_CANVAS_POINTER", "0,150,120,0,1;300,500,500,0,1"),
            ("MFB_CANVAS_FRAME_HOLD_MS", "1000"),
        ],
    );
    assert_eq!(auto_states(&lines), vec!["3", "0"], "{}", lines.join("\n"));
    assert_eq!(stat_field(&lines[0], "pointer="), "150,120,0,1");
    assert_eq!(stat_field(&lines[0], "frames="), "1");
    let last = lines.last().expect("a stats line");
    assert_eq!(stat_field(last, "pointer="), "500,500,0,1");
    assert_eq!(stat_field(last, "frames="), "2");
}

/// R27: a `present` lands during an enter window. The window is not restarted.
///
/// The window is stretched to six seconds. The pointer arrives on the frame AFTER the
/// first — the first frame of a headless run can take seconds when the machine is busy
/// (the first cut of this test timed the window from it and found it over before the
/// first `present` had returned), and that frame is where the program's clock and the
/// window's would otherwise part. From there the program waits three seconds, presents
/// the button again ten pixels over with the pointer still on it, and waits four and a
/// half more. An instance's state belongs to its place in the scene, and the deadline
/// is part of that state, so the window still ends six seconds after the pointer
/// arrived: hover shows a second and a half before the program ends. Restarted at the
/// second present, it would end a second and a half AFTER, and the last line would
/// still say enter.
#[test]
fn auto_a_present_during_an_enter_window_does_not_restart_it() {
    let source = format!(
        "{AUTO_BUTTON}FUNC main() AS Nothing\n  app::setMode(app::Mode.Canvas)\n  canvas::present([button(100.0)])\n  os::sleep(3000)\n  canvas::present([button(110.0)])\n  os::sleep(4500)\nEND FUNC\n"
    );
    let (_, lines) = run_env(
        "canvas_auto_r27",
        &source,
        true,
        &[
            ("MFB_CANVAS_POINTER", "1,150,120,0,1"),
            ("MFB_CANVAS_AUTO_WINDOW_MS", "6000"),
        ],
    );
    assert_eq!(
        auto_states(&lines),
        vec!["0", "1", "3"],
        "{}",
        lines.join("\n")
    );
    // Four frames drawn: the first present, the pointer's arrival, the second present
    // — still inside the window — and the window's end.
    let re_present = lines
        .iter()
        .find(|line| stat_field(line, "frames=") == "3")
        .unwrap_or_else(|| panic!("no third frame:\n{}", lines.join("\n")));
    assert_eq!(stat_field(re_present, "auto="), "1", "{}", lines.join("\n"));
    assert_eq!(
        stat_field(lines.last().expect("a stats line"), "frames="),
        "4",
        "{}",
        lines.join("\n")
    );
}

/// R28: the graphics thread stops while a deadline is pending. Shutdown completes.
///
/// The render loop is in a TIMED wait — two minutes of enter window — when the program
/// returns. The stop signals the same condition variable the timed wait is on, and the
/// wait checks `STOPPING` before it looks at its deadline, so the loop leaves at once
/// rather than when the window would have ended.
#[test]
fn auto_shutdown_completes_while_a_window_is_pending() {
    let source = format!(
        "{AUTO_BUTTON}FUNC main() AS Nothing\n  app::setMode(app::Mode.Canvas)\n  canvas::present([button(100.0)])\nEND FUNC\n"
    );
    let started = std::time::Instant::now();
    let (_, lines) = run_env(
        "canvas_auto_r28",
        &source,
        true,
        &[
            ("MFB_CANVAS_POINTER", "0,150,120,0,1"),
            ("MFB_CANVAS_AUTO_WINDOW_MS", "120000"),
        ],
    );
    let took = started.elapsed().as_secs();
    assert_eq!(auto_states(&lines), vec!["1"], "{}", lines.join("\n"));
    assert!(
        took < 60,
        "the program took {took} s to exit with a 120 s window pending — shutdown waited \
         for the deadline"
    );
}

/// R29: `waitFrame` is in progress when a deadline expires. It returns as it would
/// have.
///
/// The display floor is half a second (`MFB_CANVAS_DISPLAY_HZ=2`) and the enter window
/// 700 ms. The pointer arrives on the frame after the first — see R27 for why nothing is
/// timed from the first — which is as the first `present` returns, so the window ends,
/// and the render loop draws the hover frame on its own, while the worker is inside one
/// of the five `waitFrame` calls that follow. Those calls wait on their own condition
/// variable and for their own deadline; the frame neither wakes them early nor owes
/// them anything. Four calls after a first are at least three whole intervals.
#[test]
fn auto_wait_frame_is_not_cut_short_by_a_window_ending() {
    let source = format!(
        "{AUTO_BUTTON}FUNC main() AS Nothing\n  app::setMode(app::Mode.Canvas)\n  canvas::present([button(100.0)])\n  canvas::waitFrame()\n  LET t0 AS Integer = datetime::monotonicNanos()\n  MUT i AS Integer = 0\n  WHILE i < 4\n    canvas::waitFrame()\n    i = i + 1\n  END WHILE\n  io::print(\"loopMs=\" & toString((datetime::monotonicNanos() - t0) / 1000000))\nEND FUNC\n"
    );
    let (out, lines) = run_env(
        "canvas_auto_r29",
        &source,
        true,
        &[
            ("MFB_CANVAS_POINTER", "1,150,120,0,1"),
            ("MFB_CANVAS_AUTO_WINDOW_MS", "700"),
            ("MFB_CANVAS_DISPLAY_HZ", "2"),
        ],
    );
    let ms: i64 = out
        .lines()
        .find_map(|l| l.strip_prefix("loopMs="))
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or_else(|| panic!("no loopMs= line in:\n{out}"));
    assert!(
        ms >= 1450,
        "four waitFrame calls at a 500 ms floor took {ms} ms — the window's end cut one \
         short:\n{out}"
    );
    assert_eq!(
        auto_states(&lines),
        vec!["0", "1", "3"],
        "the window did not end inside the loop:\n{}",
        lines.join("\n")
    );
}
