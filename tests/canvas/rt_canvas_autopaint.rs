//! `canvas::AutoPaint` (plan-172-K): a shape with up to four alternative paints that the
//! graphics thread chooses between from the pointer.
//!
//! Everything here is read off the software frame a `--debug` build dumps
//! (`MFB_CANVAS_DUMP`) and its stats lines (`MFB_CANVAS_STATS`, one per frame the
//! graphics thread considered, drawn or not), with the pointer put somewhere by
//! `MFB_CANVAS_POINTER` — the script that calls the writer the platforms' own pointer
//! handlers call (`tests/canvas/rt_canvas_pointer.rs`).
//!
//! **Reading a state.** The stats line's `auto=` field is the paint each `AutoPaint` of
//! the frame shows, one digit each in scene order: 0 the shape's own, 1 enter, 2 exit,
//! 3 hover, 4 pressed. A state that is still showing when the program ends is also read
//! off the frame, as pixels of that paint's colour. The enter and exit paints last a
//! tenth of a second by default and are over long before a `--debug` software frame
//! could be compared, so the tests that photograph one stretch the window with
//! `MFB_CANVAS_AUTO_WINDOW_MS`; the tests that prove it ENDS use the real one.
//!
//! **Timing.** A script entry is applied at the first frame at or after its time, and
//! the render loop wakes for it. What a test asserts between two entries needs the
//! frames of the first to be over before the second falls due, and a `--debug` software
//! frame is slow — slower still with a dozen of them rendering at once, which is how the
//! first cut of this file lost a hover between an entry at 0 and one at 1000 ms. So the
//! runs take turns ([`run`]), entries are two seconds apart, and the program sleeps two
//! seconds past the last.

#[path = "../common/mod.rs"]
mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

/// A built `--debug` app and the project it lives in.
struct Built {
    name: &'static str,
    project: PathBuf,
    binary: PathBuf,
}

impl Drop for Built {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.project);
    }
}

/// What one run left behind.
struct Ran {
    /// The last frame drawn, as RGBA rows.
    frame: Vec<u8>,
    /// One `MFB_CANVAS_STATS` line per frame the graphics thread considered.
    stats: Vec<String>,
    stderr: String,
}

/// The headless surface.
const WIDTH: usize = 900;

impl Ran {
    /// How many pixels of the last frame are exactly `rgb`.
    fn count(&self, rgb: [u8; 3]) -> usize {
        self.frame.chunks(4).filter(|p| p[..3] == rgb).count()
    }

    /// The colour of the last frame at a surface pixel.
    fn pixel(&self, x: usize, y: usize) -> [u8; 3] {
        let at = (y * WIDTH + x) * 4;
        [self.frame[at], self.frame[at + 1], self.frame[at + 2]]
    }

    /// The `auto=` field of every stats line, with a run of equal values written once:
    /// the sequence of states the scene went through.
    fn autos(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for line in &self.stats {
            let value = field(line, "auto");
            if out.last() != Some(&value) {
                out.push(value);
            }
        }
        out
    }

    /// A numeric field of the LAST stats line.
    fn last(&self, name: &str) -> u64 {
        let line = self.stats.last().expect("at least one stats line");
        field(line, name)
            .parse()
            .unwrap_or_else(|_| panic!("`{name}=` is not a number in: {line}"))
    }
}

/// The value of ` name=value` in a stats line.
fn field(line: &str, name: &str) -> String {
    let key = format!("{name}=");
    line.split_whitespace()
        .find_map(|part| part.strip_prefix(key.as_str()))
        .unwrap_or_else(|| panic!("no `{key}` in: {line}"))
        .to_string()
}

fn build(name: &'static str, source: &str) -> Built {
    let project = common::temp_project(name, source);
    std::fs::write(project.join("fixture.ttf"), common::fixture_truetype())
        .expect("write the font fixture");
    let binary = common::build_app_debug(&project, name);
    Built {
        name,
        project,
        binary,
    }
}

/// Run headless with `envs` added. `sync` makes each `present` wait for its frame.
///
/// One run at a time in this test binary: the state tests are about WHEN a frame
/// happens, and their own siblings are the load that moves it.
fn run(built: &Built, sync: bool, envs: &[(&str, &str)]) -> Ran {
    static TURN: Mutex<()> = Mutex::new(());
    let _turn = TURN.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    // Its own two files: one built program is run by several tests at once.
    let nonce = common::unique_nonce();
    let frame_path = built.project.join(format!("frame_{nonce}.rgba"));
    let stats_path = built.project.join(format!("stats_{nonce}.txt"));
    let mut command = Command::new(&built.binary);
    command
        .current_dir(&built.project)
        .env("MFB_MACAPP_HEADLESS", "1")
        .env("MFB_WINAPP_HEADLESS", "1")
        .env("MFB_GTKAPP_HEADLESS", "1")
        .env("MFB_CANVAS_STATS", &stats_path)
        .env("MFB_CANVAS_DUMP", &frame_path);
    if sync {
        command.env("MFB_CANVAS_SYNC", "1");
    }
    for (key, value) in envs {
        command.env(key, value);
    }
    let output = command
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", built.binary.display()));
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "{} {}:\n{}\n{stderr}",
        built.name,
        common::exit_description(&output.status),
        String::from_utf8_lossy(&output.stdout),
    );
    Ran {
        frame: read_or_empty(&frame_path),
        stats: std::fs::read_to_string(&stats_path)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect(),
        stderr,
    }
}

fn read_or_empty(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_default()
}

/// The three shapes every Phase 1 row draws: a rounded rectangle with a stroke, a
/// circle, and a `Text` — the kind whose `AutoPaint` arms are their own (it names a
/// font). `wrap` chooses whether each is presented bare or inside an `AutoPaint` whose
/// four states are all in use, so a state paint reaching the frame would show.
const SHAPES: &str = r#"FUNC auto(s AS canvas::Shape) AS canvas::DrawItem
  LET red AS canvas::Paint = canvas::fill(color::rgb(255, 0, 0))
  LET a AS canvas::AutoPaint = canvas::AutoPaint[shape := s, useEnter := TRUE, useExit := TRUE, useHover := TRUE, usePressed := TRUE, enterPaint := red, exitPaint := red, hoverPaint := red, pressedPaint := red]
  LET item AS canvas::DrawItem = a
  RETURN item
END FUNC

FUNC shapes(face AS canvas::Font, wrap AS Boolean, shift AS Float) AS List OF canvas::DrawItem
  MUT r AS canvas::Rectangle
  r = WITH r { id := "box", position := vector::Float2[x := 40.0 + shift, y := 40.0], size := canvas::Size[width := 200.0, height := 80.0], radiusTL := 12.0, radiusTR := 12.0, radiusBL := 12.0, radiusBR := 12.0, paint := canvas::fillStroke(color::rgb(40, 90, 200), color::rgb(255, 255, 255), 3.0) }
  MUT c AS canvas::Circle
  c = WITH c { id := "dot", position := vector::Float2[x := 400.0, y := 90.0], radius := 45.0, paint := canvas::fill(color::rgb(30, 160, 90)) }
  LET t AS canvas::Text = canvas::Text[id := "label", position := vector::Float2[x := 60.0, y := 240.0], text := "AA", font := face, fontSize := 60.0, paint := canvas::fill(color::rgb(240, 240, 120)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  IF wrap THEN
    RETURN [auto(r), auto(c), auto(t)]
  END IF
  LET ri AS canvas::DrawItem = r
  LET ci AS canvas::DrawItem = c
  LET ti AS canvas::DrawItem = t
  RETURN [ri, ci, ti]
END FUNC
"#;

const HEADER: &str = r#"IMPORT app
IMPORT canvas
IMPORT color
IMPORT io
IMPORT os
IMPORT vector

"#;

fn program(main: &str) -> String {
    format!("{HEADER}{SHAPES}\n{main}")
}

/// With no pointer on the surface an `AutoPaint` is in its default state, and its
/// default state is its shape: the frame is the bare shapes' frame, byte for byte.
#[test]
fn an_autopaint_with_no_pointer_draws_exactly_its_bare_shape() {
    let built = build(
        "canvas_autopaint_bare",
        &program(
            r#"FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e)
    io::print("no font")
    RETURN
  END TRAP
  canvas::present(shapes(face, os::getEnvOr("WRAP", "") = "1", 0.0))
END FUNC
"#,
        ),
    );
    let bare = run(&built, true, &[]);
    let wrapped = run(&built, true, &[("WRAP", "1")]);
    assert!(!bare.frame.is_empty(), "the bare scene drew no frame");
    assert!(
        bare.frame.chunks(4).any(|p| p[0] == 40 && p[2] == 200),
        "the bare scene's rectangle is not in its frame"
    );
    assert!(
        bare.frame == wrapped.frame,
        "an AutoPaint with no pointer drew a different frame from its bare shape"
    );
}

/// The same inside a group: the group's walk (`__canvas_appendDraw`'s group arm, and the
/// GPU walk's `__canvas_memoGroup`) reaches an `AutoPaint` child and draws its shape.
#[test]
fn an_autopaint_inside_a_group_draws_exactly_its_bare_shape() {
    let built = build(
        "canvas_autopaint_group",
        &program(
            r#"FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e)
    io::print("no font")
    RETURN
  END TRAP
  canvas::setGroup("panel", shapes(face, os::getEnvOr("WRAP", "") = "1", 0.0))
  MUT g AS canvas::Group
  g = WITH g { id := "p", position := vector::Float2[x := 30.0, y := 20.0], name := "panel" }
  LET node AS canvas::DrawItem = g
  canvas::present([node])
END FUNC
"#,
        ),
    );
    let bare = run(&built, true, &[]);
    let wrapped = run(&built, true, &[("WRAP", "1")]);
    assert!(
        bare.frame.chunks(4).any(|p| p[0] == 40 && p[2] == 200),
        "the bare group's rectangle is not in its frame"
    );
    assert!(
        bare.frame == wrapped.frame,
        "an AutoPaint in a group drew a different frame from its bare shape"
    );
}

/// `arena.0.live_bytes` at exit, with `{N}` present / drop cycles.
fn live_bytes(built_name: &'static str, source: &str, cycles: usize) -> u64 {
    let built = build(built_name, &source.replace("{N}", &cycles.to_string()));
    let ran = run(&built, true, &[]);
    let lines = common::debug_report::arena_lines(built_name, &ran.stderr);
    common::debug_report::counter(built_name, &lines, 0, "live_bytes")
}

/// Presenting a scene of `AutoPaint`s and dropping it again reaches a steady state: the
/// worker's arena holds the same bytes after 200 cycles as after 100.
///
/// Each cycle's scene differs from the last (the rectangle moves), so each is a real
/// publish, a real retirement, and — for the `Text` — a font the retired scene named
/// and the next still does. Under `MFB_CANVAS_SYNC` retirement drains on every present,
/// so growth here is a leak and not a backlog (`rt_canvas_present_leak.rs`).
#[test]
fn presenting_and_dropping_autopaints_holds_steady_memory() {
    let source = program(
        r#"FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e)
    io::print("no font")
    RETURN
  END TRAP
  MUT i AS Integer = 0
  WHILE i < {N}
    canvas::present(shapes(face, TRUE, toFloat(i MOD 7)))
    canvas::present([])
    i = i + 1
  END WHILE
END FUNC
"#,
    );
    let few = live_bytes("canvas_autopaint_leak_few", &source, 100);
    let many = live_bytes("canvas_autopaint_leak_many", &source, 200);
    let grew = many.saturating_sub(few);
    assert!(
        grew <= 48 * 1024 + 64 * 100,
        "the arena held {few} live bytes after 100 present/drop cycles and {many} after \
         200 — {grew} bytes of growth, {} per cycle",
        grew / 100,
    );
}

// ---------------------------------------------------------------------------------
// Phase 3: state, resolved on the graphics thread.
// ---------------------------------------------------------------------------------

/// The colours of the state scenes. Every button is 200 by 80 on whole pixels with no
/// stroke, so a paint that fills it fills exactly [`BUTTON`] pixels.
const BASE: [u8; 3] = [40, 90, 200];
const ENTER_RGB: [u8; 3] = [0, 255, 0];
const EXIT_RGB: [u8; 3] = [255, 255, 0];
const HOVER_RGB: [u8; 3] = [255, 0, 0];
const PRESSED_RGB: [u8; 3] = [255, 0, 255];
/// The hover paint of the SECOND button of a scene that has two.
const HOVER2_RGB: [u8; 3] = [0, 255, 255];
const BUTTON: usize = 200 * 80;

/// `useEnter`, `useExit`, `useHover`, `usePressed`, as the scenes read them from `FLAGS`.
const ENTER: u32 = 1;
const EXIT: u32 = 2;
const HOVER: u32 = 4;
const PRESSED: u32 = 8;
const ALL: u32 = ENTER | EXIT | HOVER | PRESSED;

/// One program, every state scene, chosen by `SCENE`. It presents once (the `moved`
/// scene twice), then sleeps `SLEEP` milliseconds and ends — it never polls the mouse
/// and never presents again, so every change of paint is the graphics thread's own.
const STATES: &str = r#"IMPORT app
IMPORT canvas
IMPORT color
IMPORT os
IMPORT vector

' A button with a `Text` drawn over it -- no `id`, not an `AutoPaint`. `STROKE` makes the
' text a stroked one, which is drawn as a polygon of its outlines. The font has to
' outlive every frame that draws the text, so the sleep is in here with it.
FUNC labelled() AS Nothing
  RES face AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e)
    RETURN
  END TRAP
  MUT ink AS canvas::Paint = canvas::fill(color::rgb(240, 240, 120))
  IF os::getEnvOr("STROKE", "") = "1" THEN
    ink = canvas::stroke(color::rgb(240, 240, 120), 2.0)
  END IF
  LET label AS canvas::DrawItem = canvas::Text[id := "", position := vector::Float2[x := 120.0, y := 165.0], text := "AA", font := face, fontSize := 40.0, paint := ink, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  canvas::present([button(100.0, 100.0, "btn", canvas::fill(color::rgb(255, 0, 0))), label])
  os::sleep(toInt(os::getEnvOr("SLEEP", "1000")))
END FUNC

FUNC flag(bit AS Integer) AS Boolean
  RETURN (toInt(os::getEnvOr("FLAGS", "15")) / bit) MOD 2 = 1
END FUNC

FUNC box(x AS Float, y AS Float, id AS String) AS canvas::Rectangle
  MUT r AS canvas::Rectangle
  r = WITH r { id := id, position := vector::Float2[x := x, y := y], size := canvas::Size[width := 200.0, height := 80.0], paint := canvas::fill(color::rgb(40, 90, 200)) }
  RETURN r
END FUNC

FUNC button(x AS Float, y AS Float, id AS String, hover AS canvas::Paint) AS canvas::DrawItem
  LET a AS canvas::AutoPaint = canvas::AutoPaint[shape := box(x, y, id), useEnter := flag(1), useExit := flag(2), useHover := flag(4), usePressed := flag(8), enterPaint := canvas::fill(color::rgb(0, 255, 0)), exitPaint := canvas::fill(color::rgb(255, 255, 0)), hoverPaint := hover, pressedPaint := canvas::fill(color::rgb(255, 0, 255))]
  LET item AS canvas::DrawItem = a
  RETURN item
END FUNC

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  LET scene AS String = os::getEnvOr("SCENE", "one")
  LET red AS canvas::Paint = canvas::fill(color::rgb(255, 0, 0))
  LET cyan AS canvas::Paint = canvas::fill(color::rgb(0, 255, 255))
  IF scene = "label" THEN
    labelled()
    RETURN
  END IF
  IF scene = "one" THEN
    canvas::present([button(100.0, 100.0, "btn", red)])
  ELSEIF scene = "overlap" THEN
    canvas::present([button(100.0, 100.0, "low", red), button(200.0, 140.0, "high", cyan)])
  ELSEIF scene = "blocked" THEN
    ' A plain circle with no `id`, drawn over part of the button.
    MUT c AS canvas::Circle
    c = WITH c { position := vector::Float2[x := 200.0, y := 140.0], radius := 60.0, paint := canvas::fill(color::rgb(128, 128, 128)) }
    LET cover AS canvas::DrawItem = c
    canvas::present([button(100.0, 100.0, "btn", red), cover])
  ELSEIF scene = "groups" THEN
    ' One group holding one NAMED `AutoPaint`, drawn twice.
    canvas::setGroup("face", [button(0.0, 0.0, "face", red)])
    MUT first AS canvas::Group
    first = WITH first { id := os::getEnvOr("G1", ""), position := vector::Float2[x := 100.0, y := 100.0], name := "face" }
    MUT second AS canvas::Group
    second = WITH second { id := os::getEnvOr("G2", ""), position := vector::Float2[x := 100.0, y := 300.0], name := "face" }
    LET one AS canvas::DrawItem = first
    LET two AS canvas::DrawItem = second
    canvas::present([one, two])
  ELSEIF scene = "stroke" THEN
    ' The hover paint has a 40-pixel white outline; the shape's own paint has none.
    canvas::present([button(100.0, 100.0, "btn", canvas::fillStroke(color::rgb(255, 0, 0), color::rgb(255, 255, 255), 40.0))])
  ELSEIF scene = "moved" THEN
    canvas::present([button(100.0, 100.0, "btn", red)])
    os::sleep(toInt(os::getEnvOr("MOVE_AFTER", "1000")))
    canvas::present([button(110.0, 100.0, "btn", red)])
  ELSE
    ' No `AutoPaint` at all: the bare rectangle.
    LET plain AS canvas::DrawItem = box(100.0, 100.0, "btn")
    canvas::present([plain])
  END IF
  os::sleep(toInt(os::getEnvOr("SLEEP", "1000")))
END FUNC
"#;

/// [`STATES`], built once for every test below.
fn states() -> &'static Built {
    static BUILT: OnceLock<Built> = OnceLock::new();
    BUILT.get_or_init(|| build("canvas_autopaint_states", STATES))
}

/// Run one scene of [`STATES`] with a pointer script, sleeping `sleep_ms` after its
/// last present.
fn scene(name: &str, flags: u32, pointer: &str, sleep_ms: u32, envs: &[(&str, &str)]) -> Ran {
    let flags = flags.to_string();
    let sleep = sleep_ms.to_string();
    let mut all = vec![
        ("SCENE", name),
        ("FLAGS", flags.as_str()),
        ("MFB_CANVAS_POINTER", pointer),
        ("SLEEP", sleep.as_str()),
    ];
    all.extend_from_slice(envs);
    run(states(), true, &all)
}

/// A window long enough that a flash is still showing when the program ends.
const LONG_WINDOW: (&str, &str) = ("MFB_CANVAS_AUTO_WINDOW_MS", "600000");

/// The pointer arrives on a shape and the shape shows its hover paint — presented
/// once, with no code of the program's running.
#[test]
fn the_hover_paint_shows_after_entry() {
    let ran = scene("one", HOVER, "0,150,120,0,1", 1500, &[]);
    assert_eq!(ran.autos(), vec!["3"]);
    assert_eq!(ran.count(HOVER_RGB), BUTTON);
    assert_eq!(ran.count(BASE), 0);
}

/// The enter paint shows first, and gives way to hover a tenth of a second later with the pointer
/// where it was: one script entry, two frames, the second one asked for by the clock.
#[test]
fn the_enter_paint_shows_first_and_gives_way_with_the_pointer_still() {
    let ran = scene("one", ALL, "0,150,120,0,1", 1500, &[]);
    assert_eq!(ran.autos(), vec!["1", "3"]);
    assert_eq!(ran.last("frames"), 2, "one frame per paint, and no more");
    assert_eq!(ran.count(HOVER_RGB), BUTTON);

    // And it is the enter paint that is drawn while the window runs.
    let flash = scene("one", ALL, "0,150,120,0,1", 1000, &[LONG_WINDOW]);
    assert_eq!(flash.autos(), vec!["1"]);
    assert_eq!(flash.count(ENTER_RGB), BUTTON);
}

/// The pointer leaves: the exit paint, then the shape's own paint a moment later, with
/// nothing further from the pointer.
#[test]
fn the_exit_paint_shows_then_the_default() {
    let script = "0,150,120,0,1;2000,10,10,0,1";
    let ran = scene("one", ALL, script, 4000, &[]);
    assert_eq!(ran.autos(), vec!["1", "3", "2", "0"]);
    assert_eq!(ran.count(BASE), BUTTON);

    // And it is the exit paint that is drawn while the window runs. `useEnter` is off
    // here, or the stretched ENTER window would still be running when the pointer left.
    let flash = scene("one", EXIT | HOVER, script, 4000, &[LONG_WINDOW]);
    assert_eq!(flash.autos(), vec!["3", "2"]);
    assert_eq!(flash.count(EXIT_RGB), BUTTON);
}

/// A button goes down on the shape: pressed, over hover. It comes up: hover again.
#[test]
fn pressed_wins_over_hover() {
    let ran = scene(
        "one",
        HOVER | PRESSED,
        "0,150,120,0,1;2000,150,120,1,1",
        4000,
        &[],
    );
    assert_eq!(ran.autos(), vec!["3", "4"]);
    assert_eq!(ran.count(PRESSED_RGB), BUTTON);

    let released = scene(
        "one",
        HOVER | PRESSED,
        "0,150,120,0,1;2000,150,120,1,1;4000,150,120,0,1",
        6000,
        &[],
    );
    assert_eq!(released.autos(), vec!["3", "4", "3"]);
    assert_eq!(released.count(HOVER_RGB), BUTTON);
}

/// Two `AutoPaint`s overlap and the pointer is where both are: only the upper one —
/// the later in the scene — changes. Where only the lower is, the lower does.
#[test]
fn only_the_upper_of_two_overlapping_autopaints_changes() {
    let both = scene("overlap", HOVER, "0,250,160,0,1", 1500, &[]);
    assert_eq!(both.autos(), vec!["03"]);
    assert_eq!(both.count(HOVER2_RGB), BUTTON);
    assert_eq!(
        both.count(HOVER_RGB),
        0,
        "the lower button took the pointer too"
    );

    let lower = scene("overlap", HOVER, "0,120,110,0,1", 1500, &[]);
    assert_eq!(lower.autos(), vec!["30"]);
    assert_eq!(lower.count(HOVER2_RGB), 0);
}

/// An item that is not an `AutoPaint` and has no `id` still takes the pointer from
/// what it covers. Beside it, on the uncovered part of the button, the button reacts.
#[test]
fn an_item_with_no_id_on_top_blocks_the_autopaint_beneath() {
    // (200, 140) is the circle's centre and inside the button.
    let covered = scene("blocked", HOVER, "0,200,140,0,1", 1500, &[]);
    assert_eq!(covered.autos(), vec!["0"]);
    assert_eq!(covered.count(HOVER_RGB), 0);

    // (110, 105) is inside the button and 96 pixels from the circle's centre.
    let beside = scene("blocked", HOVER, "0,110,105,0,1", 1500, &[]);
    assert_eq!(beside.autos(), vec!["3"]);
}

/// One group, drawn twice, holding a NAMED `AutoPaint`: the two uses have a state
/// each, whatever the group nodes are called — two different `id`s, the same `id`, or
/// none. An `id` is what `overID` reports; it plays no part here.
#[test]
fn two_uses_of_one_group_change_independently() {
    for (first, second) in [("a", "b"), ("same", "same"), ("", "")] {
        let ids = [("G1", first), ("G2", second)];
        // The first use is at (100, 100), the second at (100, 300).
        let upper = scene("groups", HOVER, "0,150,120,0,1", 1500, &ids);
        assert_eq!(upper.autos(), vec!["30"], "ids `{first}` / `{second}`");
        assert_eq!(upper.count(HOVER_RGB), BUTTON, "ids `{first}` / `{second}`");
        assert_eq!(upper.count(BASE), BUTTON, "ids `{first}` / `{second}`");
        assert_eq!(upper.pixel(150, 120), HOVER_RGB);
        assert_eq!(upper.pixel(150, 320), BASE);

        let lower = scene("groups", HOVER, "0,150,320,0,1", 1500, &ids);
        assert_eq!(lower.autos(), vec!["03"], "ids `{first}` / `{second}`");
        assert_eq!(lower.pixel(150, 120), BASE);
        assert_eq!(lower.pixel(150, 320), HOVER_RGB);
    }
}

/// The hover paint has a 40-pixel outline and the shape's own paint has none. The
/// outline is drawn — and a pointer on it, outside the shape, is NOT on the shape:
/// where a shape is hit is decided by its own paint.
#[test]
fn a_hover_paint_with_a_wider_stroke_does_not_widen_the_hit_region() {
    // The button is x 100..300. (310, 120) is 10 pixels outside it, and inside the
    // 20 pixels of outline the hover paint draws there.
    let hovered = scene("stroke", HOVER, "0,150,120,0,1", 1500, &[]);
    assert_eq!(hovered.autos(), vec!["3"]);
    assert_eq!(
        hovered.pixel(310, 120),
        [255, 255, 255],
        "the hover paint's outline is not where this test assumes it is"
    );

    let moved = scene("stroke", HOVER, "0,150,120,0,1;2000,310,120,0,1", 4000, &[]);
    assert_eq!(moved.autos(), vec!["3", "0"]);
    assert_eq!(moved.count(BASE), BUTTON);
}

/// The program presents the same button again, ten pixels to the right, with the
/// pointer still on it. It is the same instance — the same place in the scene — so it
/// stays on hover: the enter paint does not come round again.
#[test]
fn state_survives_a_re_present_of_a_moved_shape() {
    let ran = scene(
        "moved",
        ALL,
        "0,150,120,0,1",
        2000,
        &[("MOVE_AFTER", "3000")],
    );
    assert_eq!(ran.autos(), vec!["1", "3"]);
    assert_eq!(ran.count(HOVER_RGB), BUTTON);
    // The frame is the SECOND present's: the button is at x 110..310.
    assert_eq!(ran.pixel(105, 120), [0, 0, 0]);
    assert_eq!(ran.pixel(305, 120), HOVER_RGB);
}

/// A scene with no `AutoPaint` costs no frame however the pointer moves: one present,
/// three pointer changes — the last with a button down, on the shape — one frame, and
/// the graphics thread was never woken for the other three.
#[test]
fn a_scene_with_no_autopaint_draws_one_frame_however_the_pointer_moves() {
    let ran = scene(
        "plain",
        ALL,
        "0,10,10,0,1;2000,150,120,0,1;4000,150,120,1,1",
        6000,
        &[],
    );
    assert_eq!(
        ran.stats.len(),
        1,
        "the render loop woke for a pointer change with no AutoPaint in the scene:\n{}",
        ran.stats.join("\n")
    );
    assert_eq!(ran.last("frames"), 1);
    assert_eq!(ran.count(BASE), BUTTON);
}

/// With an `AutoPaint` in the scene the pointer does wake the render loop — and a wake
/// that changes no paint draws nothing. The pointer moves three times and never
/// reaches the button: one frame drawn, the first, and every later wake skipped.
#[test]
fn pointer_wakes_that_change_no_paint_draw_no_frame() {
    let ran = scene(
        "one",
        ALL,
        "0,500,500,0,1;2000,520,510,0,1;4000,540,520,1,1",
        6000,
        &[],
    );
    assert_eq!(ran.autos(), vec!["0"]);
    assert_eq!(ran.last("frames"), 1, "{}", ran.stats.join("\n"));
    assert!(
        ran.last("skipped") >= 2,
        "the two later pointer changes did not wake the render loop:\n{}",
        ran.stats.join("\n")
    );
    assert_eq!(ran.count(BASE), BUTTON);
}

/// A `Text` drawn over an `AutoPaint` takes the pointer over its whole LAYOUT BOX — the
/// advance by the ascent and descent — not just where its glyphs have ink, on the
/// graphics thread exactly as `overID` has it on the worker
/// (`rt_canvas_hit.rs:hit_region_text_is_its_layout_box`, whose metrics these are). So
/// a label on a button blocks the button across the label's box, and a pointer on the
/// bare part of the button still lights it.
///
/// `AA` at `fontSize` 40 with its baseline starting at (120, 165) is the box
/// (120, 133)-(142, 173): the fixture font advances 0.275 em a glyph, ascends 0.8 em
/// and descends 0.2. Its ink is two 12 px squares resting on the baseline, so
/// (122, 137) is inside the box and far above any ink — the frame confirms it shows
/// the button there — and (150, 137) is on the same row, 8 px past the advance.
///
/// Both kinds of text record: a filled run is a glyph run, and a stroked run is drawn
/// as a polygon of its outlines, which says it carries a box by a mark of its own
/// (`helper_hit.rs:__canvas_hitIsTextBox`).
#[test]
fn a_text_over_an_autopaint_blocks_it_across_its_layout_box() {
    for stroke in ["", "1"] {
        let kind = [("STROKE", stroke)];
        let on_box = scene("label", HOVER, "0,122,137,0,1", 1500, &kind);
        assert!(
            on_box.frame.chunks(4).any(|p| p[..3] == [240, 240, 120]),
            "the label drew no ink (stroke `{stroke}`)"
        );
        assert_eq!(
            on_box.pixel(122, 137),
            BASE,
            "(122, 137) is not bare button: ink, or a button that lit (stroke `{stroke}`)"
        );
        assert_eq!(on_box.autos(), vec!["0"], "stroke `{stroke}`");
        assert_eq!(on_box.count(HOVER_RGB), 0, "stroke `{stroke}`");

        let beside = scene("label", HOVER, "0,150,137,0,1", 1500, &kind);
        assert_eq!(beside.autos(), vec!["3"], "stroke `{stroke}`");
    }
}

/// The `--debug` geometry check (`MFB_CANVAS_GEO_VERIFY=1`) rebuilds the record every
/// resolved item should have and counts the ones that differ. An `AutoPaint`'s record is
/// its shape's, and in a program with an `AutoPaint` a text's record carries its layout
/// box — and the check's own builder has to know both, or every `AutoPaint` scene reads
/// as corrupt on the boxes whose rows set the variable. It did not, the first time:
/// `scripts/test-canvas-gpu-rows.sh` counted three mismatches in a five-item scene.
///
/// The scene is a button with a label on it, filled and stroked; `geoResolvedChecked`
/// must be the two items and `geoVerifyMismatches` zero.
#[test]
fn the_debug_geometry_check_agrees_with_an_autopaint_scene() {
    for stroke in ["", "1"] {
        let ran = scene(
            "label",
            HOVER,
            "",
            500,
            &[("STROKE", stroke), ("MFB_CANVAS_GEO_VERIFY", "1")],
        );
        assert_eq!(ran.last("geoResolvedChecked"), 2, "stroke `{stroke}`");
        assert_eq!(
            ran.last("geoVerifyMismatches"),
            0,
            "stroke `{stroke}`: {}",
            ran.stats.join("\n")
        );
    }
}

/// `options.canvasAutoPaintMS` in project.json is how long the enter and exit paints are
/// held, and it is baked into the program: the same source built with the option at ten
/// seconds is still showing its enter paint when it ends, where built without it — 100
/// ms — it has gone on to hover. Nothing in the environment says how long; that is the
/// difference between this and every test above that stretches the window.
#[test]
fn the_project_option_sets_how_long_a_flash_lasts() {
    let source = STATES;
    let stretched = {
        let project = common::temp_project("canvas_autopaint_option", source);
        let manifest = project.join("project.json");
        let text = std::fs::read_to_string(&manifest).expect("read project.json");
        let with_option = text.replacen(
            "\"targets\":[\"native\"]",
            "\"targets\":[\"native\"],\"options\":{\"canvasAutoPaintMS\":10000}",
            1,
        );
        assert_ne!(
            text, with_option,
            "the scaffold's project.json changed shape"
        );
        std::fs::write(&manifest, with_option).expect("write project.json");
        std::fs::write(project.join("fixture.ttf"), common::fixture_truetype())
            .expect("write the font fixture");
        let binary = common::build_app_debug(&project, "canvas_autopaint_option");
        Built {
            name: "canvas_autopaint_option",
            project,
            binary,
        }
    };
    let envs = [
        ("SCENE", "one"),
        ("FLAGS", "15"),
        ("MFB_CANVAS_POINTER", "0,150,120,0,1"),
        ("SLEEP", "1500"),
    ];
    let held = run(&stretched, true, &envs);
    assert_eq!(held.autos(), vec!["1"]);
    assert_eq!(held.count(ENTER_RGB), BUTTON);

    // The default: the flash is over well inside the same second and a half.
    let default = run(states(), true, &envs);
    assert_eq!(default.autos(), vec!["1", "3"]);
    assert_eq!(default.count(HOVER_RGB), BUTTON);
}

/// plan-172-L: a `Picture` whose shape is an `AutoPaint`. One in the scene, at
/// (200, 200), and one in a group drawn twice, at (500, 200) and (700, 200); every
/// image is one white texel, so a picture shows exactly its tint.
const AUTO_PICTURES: &str = r#"IMPORT app
IMPORT canvas
IMPORT color
IMPORT io
IMPORT os
IMPORT vector

FUNC auto(c AS canvas::Circle) AS canvas::AutoPaint
  LET off AS canvas::Paint = canvas::fill(color::rgb(0, 0, 0))
  RETURN canvas::AutoPaint[shape := c, useEnter := FALSE, useExit := FALSE, useHover := TRUE, usePressed := FALSE, enterPaint := off, exitPaint := off, hoverPaint := canvas::fillStroke(color::rgb(255, 0, 0), color::rgb(0, 0, 255), 6.0), pressedPaint := off]
END FUNC

FUNC node(x AS Float) AS canvas::DrawItem
  MUT g AS canvas::Group
  g = WITH g { position := vector::Float2[x := x, y := 200.0], name := "badge" }
  LET item AS canvas::DrawItem = g
  RETURN item
END FUNC

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES img AS canvas::Image = canvas::createImage(1, 1, [toByte(255), toByte(255), toByte(255), toByte(255)])
  RES inGroup AS canvas::Image = canvas::createImage(1, 1, [toByte(255), toByte(255), toByte(255), toByte(255)])
  MUT disc AS canvas::Circle
  disc = WITH disc { id := "disc", position := vector::Float2[x := 200.0, y := 200.0], radius := 50.0, paint := canvas::fill(color::rgb(255, 255, 255)) }
  LET top AS canvas::DrawItem = canvas::Picture[id := "p", shape := auto(disc), clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img]
  MUT dot AS canvas::Circle
  dot = WITH dot { radius := 40.0, paint := canvas::fill(color::rgb(255, 255, 255)) }
  canvas::setGroup("badge", [canvas::Picture[id := "", shape := auto(dot), clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := inGroup]])
  canvas::present([top, node(500.0), node(700.0)])
  os::sleep(1500)
END FUNC
"#;

/// The state paint of an `AutoPaint` that is a picture's shape replaces the picture's
/// tint and its outline — and only that instance's.
///
/// With no pointer the three discs are the white image untinted. With the pointer on
/// one, that one shows the hover paint: its opaque red fill is the image's tint, so
/// the white texel is red, and its 6 px blue stroke outlines the circle — (250, 200),
/// centre half a pixel outside the radius, is on it, and (256, 200), 6.5 px out, is
/// past it. The other two stay white, including the other use of the same group.
#[test]
fn a_picture_on_an_autopaint_shows_the_state_paint_as_its_tint_and_outline() {
    const WHITE: [u8; 3] = [255, 255, 255];
    const RED: [u8; 3] = [255, 0, 0];
    const BLUE: [u8; 3] = [0, 0, 255];
    let built = build("canvas_autopaint_pictures", AUTO_PICTURES);

    let idle = run(&built, true, &[]);
    assert_eq!(idle.autos(), vec!["000"]);
    for x in [200, 500, 700] {
        assert_eq!(idle.pixel(x, 200), WHITE, "idle, x = {x}");
    }
    assert_eq!(idle.count(RED), 0);
    assert_eq!(idle.count(BLUE), 0);

    let top = run(&built, true, &[("MFB_CANVAS_POINTER", "0,200,200,0,1")]);
    assert_eq!(top.autos(), vec!["300"]);
    assert_eq!(top.pixel(200, 200), RED);
    assert_eq!(top.pixel(250, 200), BLUE);
    assert_eq!(top.pixel(256, 200), [0, 0, 0]);
    assert_eq!(top.pixel(500, 200), WHITE);
    assert_eq!(top.pixel(700, 200), WHITE);

    // The group's two uses are two instances: each alone.
    let first = run(&built, true, &[("MFB_CANVAS_POINTER", "0,500,200,0,1")]);
    assert_eq!(first.autos(), vec!["030"]);
    assert_eq!(first.pixel(200, 200), WHITE);
    assert_eq!(first.pixel(500, 200), RED);
    assert_eq!(first.pixel(540, 200), BLUE);
    assert_eq!(first.pixel(700, 200), WHITE);

    let second = run(&built, true, &[("MFB_CANVAS_POINTER", "0,700,200,0,1")]);
    assert_eq!(second.autos(), vec!["003"]);
    assert_eq!(second.pixel(500, 200), WHITE);
    assert_eq!(second.pixel(700, 200), RED);

    // A picture is over wherever its SHAPE is: (234, 234), centre 48.8 px from the
    // circle's, is inside it; (237, 237), 53.0 px out, is inside its box and outside it.
    let edge = run(&built, true, &[("MFB_CANVAS_POINTER", "0,234,234,0,1")]);
    assert_eq!(edge.autos(), vec!["300"]);
    let outside = run(&built, true, &[("MFB_CANVAS_POINTER", "0,237,237,0,1")]);
    assert_eq!(outside.autos(), vec!["000"]);
}

// ---- plan-164-B: an `AutoPaint` over a curve shape -----------------------------------

/// Two `AutoPaint`s, each with only its hover paint in use: one over a `CurvePolygon`
/// circle of radius 60 about (200, 200), filled [`BASE`]; one over a `PolyCurve` arch
/// at (500, 300) — a quadratic from (0, 0) through a control point at (50, -60) to
/// (100, 0), whose highest point is (550, 270) — stroked 8 px wide in [`BASE`].
/// `WIDE` gives the circle's hover paint a 40 px white outline.
const CURVES: &str = r#"IMPORT app
IMPORT canvas
IMPORT color
IMPORT os
IMPORT vector

FUNC at(x AS Float, y AS Float) AS vector::Float2
  RETURN vector::Float2[x := x, y := y]
END FUNC

FUNC arc(x0 AS Float, y0 AS Float, x1 AS Float, y1 AS Float, x2 AS Float, y2 AS Float) AS canvas::BezierCurve
  RETURN canvas::BezierCurve[points := [at(x0, y0), at(x1, y1), at(x2, y2)], weights := [1.0, 0.7071067811865476, 1.0]]
END FUNC

FUNC hovering(s AS canvas::Shape, hover AS canvas::Paint) AS canvas::DrawItem
  LET off AS canvas::Paint = canvas::fill(color::rgb(0, 255, 0))
  LET a AS canvas::AutoPaint = canvas::AutoPaint[shape := s, useEnter := FALSE, useExit := FALSE, useHover := TRUE, usePressed := FALSE, enterPaint := off, exitPaint := off, hoverPaint := hover, pressedPaint := off]
  LET item AS canvas::DrawItem = a
  RETURN item
END FUNC

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  LET base AS color::Color = color::rgb(40, 90, 200)
  LET red AS color::Color = color::rgb(255, 0, 0)
  MUT disc AS canvas::CurvePolygon
  disc = WITH disc { id := "disc", position := at(200.0, 200.0), contours := [[arc(60.0, 0.0, 60.0, 60.0, 0.0, 60.0), arc(0.0, 60.0, 0.0 - 60.0, 60.0, 0.0 - 60.0, 0.0), arc(0.0 - 60.0, 0.0, 0.0 - 60.0, 0.0 - 60.0, 0.0, 0.0 - 60.0), arc(0.0, 0.0 - 60.0, 60.0, 0.0 - 60.0, 60.0, 0.0)]], paint := canvas::fill(base) }
  MUT arch AS canvas::PolyCurve
  arch = WITH arch { id := "arch", position := at(500.0, 300.0), curves := [canvas::BezierCurve[points := [at(0.0, 0.0), at(50.0, 0.0 - 60.0), at(100.0, 0.0)], weights := []]], paint := canvas::stroke(base, 8.0) }
  MUT discHover AS canvas::Paint = canvas::fill(red)
  IF os::getEnvOr("WIDE", "") = "1" THEN
    discHover = canvas::fillStroke(red, color::rgb(255, 255, 255), 40.0)
  END IF
  LET discShape AS canvas::Shape = disc
  LET archShape AS canvas::Shape = arch
  IF os::getEnvOr("PICTURE", "") = "1" THEN
    ' A white image cut to the disc, whose own fill (white) leaves it white and whose
    ' hover fill (red) tints it red.
    RES img AS canvas::Image = canvas::createImage(1, 1, [toByte(255), toByte(255), toByte(255), toByte(255)])
    LET off AS canvas::Paint = canvas::fill(color::rgb(0, 255, 0))
    LET lit AS canvas::AutoShape = canvas::AutoPaint[shape := WITH disc { paint := canvas::fill(color::rgb(255, 255, 255)) }, useEnter := FALSE, useExit := FALSE, useHover := TRUE, usePressed := FALSE, enterPaint := off, exitPaint := off, hoverPaint := canvas::fill(red), pressedPaint := off]
    LET pic AS canvas::DrawItem = canvas::Picture[id := "pic", shape := lit, clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img]
    canvas::present([pic])
    os::sleep(toInt(os::getEnvOr("SLEEP", "1000")))
    RETURN
  END IF
  canvas::present([hovering(discShape, discHover), hovering(archShape, canvas::stroke(red, 8.0))])
  os::sleep(toInt(os::getEnvOr("SLEEP", "1000")))
END FUNC
"#;

fn curves() -> &'static Built {
    static BUILT: OnceLock<Built> = OnceLock::new();
    BUILT.get_or_init(|| build("canvas_autopaint_curves", CURVES))
}

fn curve_scene(pointer: &str, sleep_ms: u32, envs: &[(&str, &str)]) -> Ran {
    let sleep = sleep_ms.to_string();
    let mut all = vec![("MFB_CANVAS_POINTER", pointer), ("SLEEP", sleep.as_str())];
    all.extend_from_slice(envs);
    run(curves(), true, &all)
}

/// An `AutoPaint` over a `CurvePolygon` is hovered inside the curved outline and not in
/// the corners of its bounding box. The circle is radius 60 about (200, 200): the
/// pointer at its centre shows the hover paint (`auto=30`: the disc on hover, the arch
/// on its own); at (250, 250) it is inside the box and 70.7 px from the centre —
/// outside the curve — and the disc shows its own paint.
#[test]
fn an_autopaint_over_a_curve_polygon_follows_the_curve_not_its_box() {
    let inside = curve_scene("0,200,200,0,1", 1500, &[]);
    assert_eq!(inside.autos(), vec!["30"]);
    assert_eq!(inside.pixel(200, 200), HOVER_RGB);
    assert_eq!(
        inside.pixel(230, 230),
        HOVER_RGB,
        "inside the curve, 42 px out"
    );

    let corner = curve_scene("0,250,250,0,1", 1500, &[]);
    assert_eq!(corner.autos(), vec!["00"]);
    assert_eq!(corner.pixel(200, 200), BASE);
}

/// An `AutoPaint` over a `PolyCurve` is hovered on its stroke and not in the area the
/// chain would enclose. The arch's highest point is (550, 270): the pointer there
/// shows the hover paint; at (550, 292), 22 px below it and 8 px above the chord, it
/// is between the curve and its ends and on neither, and the arch shows its own paint.
#[test]
fn an_autopaint_over_a_poly_curve_follows_its_stroke() {
    let on = curve_scene("0,550,270,0,1", 1500, &[]);
    assert_eq!(on.autos(), vec!["03"]);
    assert_eq!(on.pixel(550, 270), HOVER_RGB);

    let under = curve_scene("0,550,292,0,1", 1500, &[]);
    assert_eq!(under.autos(), vec!["00"]);
    assert_eq!(under.pixel(550, 270), BASE);
}

/// A hover paint with a wider stroke does not make a curve shape easier to hit: the
/// disc's hover paint has a 40 px outline, drawn 20 px either side of radius 60. The
/// pointer goes to the centre (hover) and then to (270, 200), 10 px outside the disc
/// and on that outline — and the disc goes back to its own paint.
#[test]
fn a_curve_shapes_hover_stroke_does_not_widen_its_hit_region() {
    let hovered = curve_scene("0,200,200,0,1", 1500, &[("WIDE", "1")]);
    assert_eq!(hovered.autos(), vec!["30"]);
    assert_eq!(
        hovered.pixel(270, 200),
        [255, 255, 255],
        "the hover paint's outline is not where this test assumes it is"
    );

    let moved = curve_scene("0,200,200,0,1;2000,270,200,0,1", 4000, &[("WIDE", "1")]);
    assert_eq!(moved.autos(), vec!["30", "00"]);
    assert_eq!(moved.pixel(200, 200), BASE);
    assert_eq!(moved.pixel(270, 200), [0, 0, 0]);
}

/// A `Picture` on an `AutoPaint` over a `CurvePolygon` changes its tint on hover: the
/// paint the pointer selects is the picture's. A white image cut to the disc, white
/// under the shape's own fill and red under its hover fill. The pointer on the disc
/// turns the picture red inside the curved outline; in the corner of its box, outside
/// the curve, it does not.
#[test]
fn a_picture_on_an_autopaint_over_a_curve_polygon_tints_on_hover() {
    let hovered = curve_scene("0,200,200,0,1", 1500, &[("PICTURE", "1")]);
    assert_eq!(hovered.autos(), vec!["3"]);
    assert_eq!(hovered.pixel(200, 200), HOVER_RGB);
    assert_eq!(
        hovered.pixel(240, 230),
        HOVER_RGB,
        "inside the curve, 50 px out"
    );
    assert_eq!(
        hovered.pixel(250, 250),
        [0, 0, 0],
        "the box's corner is outside it"
    );

    let corner = curve_scene("0,250,250,0,1", 1500, &[("PICTURE", "1")]);
    assert_eq!(corner.autos(), vec!["0"]);
    assert_eq!(corner.pixel(200, 200), [255, 255, 255]);
}
