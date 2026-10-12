//! The `canvas::AutoPaint` state rules (plan-172-K Phase 2), one test per rule in
//! `planning/new_canvas.md` "AutoPaint states".
//!
//! The rules are one pure MFBASIC function, `__canvas_autoStep`
//! (`src/codegen/builtins/canvas/helper_autopaint.rs:AUTO_STEP`). The compiler is a
//! binary crate, so a test cannot link that text; this file reads it out of the source
//! file it is declared in, compiles it as a program of its own behind a small driver,
//! and runs it. What is run is therefore the text the compiler injects, character for
//! character, with only the `__canvas_` prefix — which user source may not spell —
//! renamed.
//!
//! The driver holds exactly what the graphics thread holds for an instance between
//! frames (a phase, a deadline, whether it owns the press, the buttons last seen) and
//! steps it through a script of frames. Each frame is `ms,inside,buttons`; the answer
//! is the paint shown on each frame as one digit:
//!
//! | Digit | Paint |
//! |---|---|
//! | 0 | the shape's own |
//! | 1 | `enterPaint` |
//! | 2 | `exitPaint` |
//! | 3 | `hoverPaint` |
//! | 4 | `pressedPaint` |

#[path = "../common/mod.rs"]
mod common;

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

/// `useEnter`, `useExit`, `useHover`, `usePressed`, as `__canvas_autoStep` takes them.
const ENTER: u32 = 1;
const EXIT: u32 = 2;
const HOVER: u32 = 4;
const PRESSED: u32 = 8;
const ALL: u32 = ENTER | EXIT | HOVER | PRESSED;

/// The text of `AUTO_STEP`, as the compiler holds it.
fn auto_step_source() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/codegen/builtins/canvas/helper_autopaint.rs");
    let text = std::fs::read_to_string(&path).expect("read helper_autopaint.rs");
    let opens = "pub(super) const AUTO_STEP: &str =\nr#\"";
    let start = text
        .find(opens)
        .expect("helper_autopaint.rs declares AUTO_STEP as a raw string")
        + opens.len();
    let end = start
        + text[start..]
            .find("\"#;")
            .expect("AUTO_STEP's raw string closes");
    text[start..end]
        .replace("__canvas_", "k_")
        .replace("__CANVAS_", "K_")
}

const DRIVER: &str = r#"
FUNC numbers(text AS String) AS List OF Integer
  LET bytes AS List OF Byte = encoding::utf8Encode(text)
  MUT out AS List OF Integer = []
  MUT value AS Integer = 0
  MUT open AS Boolean = FALSE
  MUT i AS Integer = 0
  WHILE i < len(bytes)
    LET b AS Integer = toInt(collections::getOr(bytes, i, toByte(0)))
    IF b >= 48 AND b <= 57 THEN
      value = value * 10 + (b - 48)
      open = TRUE
    ELSEIF open THEN
      out = collections::append(out, value)
      value = 0
      open = FALSE
    END IF
    i = i + 1
  END WHILE
  IF open THEN
    out = collections::append(out, value)
  END IF
  RETURN out
END FUNC

FUNC main() AS Nothing
  LET flagsFirst AS Integer = toInt(os::getEnvOr("FLAGS", "15"))
  LET flagsLater AS Integer = toInt(os::getEnvOr("FLAGS_LATER", os::getEnvOr("FLAGS", "15")))
  LET steps AS List OF Integer = numbers(os::getEnvOr("STEPS", ""))
  MUT phase AS Integer = 0
  MUT deadline AS Integer = 0
  MUT owns AS Boolean = FALSE
  MUT was AS Integer = 0
  MUT shown AS String = ""
  MUT i AS Integer = 0
  WHILE i * 3 + 2 < len(steps)
    ' A clock that does not start at zero, in nanoseconds: the window is 50 ms.
    LET now AS Integer = 7000000000 + collections::getOr(steps, i * 3, 0) * 1000000
    LET inside AS Boolean = collections::getOr(steps, i * 3 + 1, 0) = 1
    LET buttons AS Integer = collections::getOr(steps, i * 3 + 2, 0)
    ' The first frame is stepped with `FLAGS`, every later one with `FLAGS_LATER`.
    MUT flags AS Integer = flagsFirst
    IF i > 0 THEN
      flags = flagsLater
    END IF
    LET packed AS Integer = k_autoStep(phase, deadline, owns, inside, buttons, was, flags, now)
    deadline = k_autoDeadline(packed, deadline, now, 50000000)
    phase = k_autoPhase(packed)
    owns = k_autoOwns(packed)
    was = buttons
    shown = shown & toString(k_autoShown(packed))
    i = i + 1
  END WHILE
  MUT pending AS String = "idle"
  IF deadline > 0 THEN
    pending = "pending"
  END IF
  io::print(shown & " " & pending)
END FUNC
"#;

/// The driver, built once for every test in this file.
fn driver() -> &'static PathBuf {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    BUILT.get_or_init(|| {
        let source = format!(
            "IMPORT collections\nIMPORT encoding\nIMPORT io\nIMPORT os\n\n{}\n{DRIVER}",
            auto_step_source()
        );
        let project = common::temp_project("canvas_auto_step", &source);
        common::build_project(&project)
    })
}

/// Step one instance through `steps` (`ms,inside,buttons;…`) with `flags` in use, and
/// answer the paint shown on each frame, then whether a window is still running.
fn shown(flags: u32, steps: &str) -> String {
    shown_changing(flags, flags, steps)
}

/// [`shown`], with the flags changing after the first frame — what a re-`present` of
/// the same instance with different `use…` flags does.
fn shown_changing(first: u32, later: u32, steps: &str) -> String {
    let output = Command::new(driver())
        .env("FLAGS", first.to_string())
        .env("FLAGS_LATER", later.to_string())
        .env("STEPS", steps)
        .output()
        .expect("run the auto-step driver");
    assert!(
        output.status.success(),
        "the driver {}:\n{}",
        common::exit_description(&output.status),
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// Priority, highest first: pressed, enter, hover, exit, the shape's own.
///
/// One instance, every state in use. The pointer arrives with a button already going
/// down on it (pressed beats enter), the button comes up inside the enter window (enter
/// beats hover), the window runs out (hover), the pointer leaves (exit beats the
/// shape's own), and that window runs out.
#[test]
fn canvas_auto_step_priority_is_pressed_enter_hover_exit_default() {
    assert_eq!(
        shown(ALL, "0,0,0; 10,1,1; 20,1,0; 70,1,0; 80,0,0; 140,0,0"),
        "041320 idle"
    );
}

/// `pressed` unused shows what hovering would: a press on the shape changes nothing.
#[test]
fn canvas_auto_step_pressed_unused_shows_hover() {
    assert_eq!(
        shown(ENTER | EXIT | HOVER, "0,1,0; 60,1,0; 70,1,1; 80,1,1"),
        "1333 idle"
    );
}

/// `enter` unused shows `hover` at once, and starts no window.
#[test]
fn canvas_auto_step_enter_unused_shows_hover_at_once() {
    assert_eq!(shown(EXIT | HOVER | PRESSED, "0,1,0"), "3 idle");
}

/// `hover` unused shows the shape's own paint while the pointer rests on it — after
/// the enter flash, which is still in use.
#[test]
fn canvas_auto_step_hover_unused_shows_the_default_paint() {
    assert_eq!(
        shown(ENTER | EXIT | PRESSED, "0,1,0; 60,1,0; 70,1,0"),
        "100 idle"
    );
}

/// `exit` unused shows the shape's own paint at once, and starts no window.
#[test]
fn canvas_auto_step_exit_unused_shows_the_default_paint_at_once() {
    assert_eq!(
        shown(ENTER | HOVER | PRESSED, "0,1,0; 60,1,0; 70,0,0"),
        "130 idle"
    );
}

/// `enter` lasts 50 ms and then gives way to `hover` — with the pointer where it was.
/// One millisecond short of the window it is still the enter paint.
#[test]
fn canvas_auto_step_enter_expires_to_hover_at_50ms() {
    assert_eq!(shown(ALL, "0,1,0; 49,1,0; 50,1,0"), "113 idle");
    assert_eq!(shown(ALL, "0,1,0; 49,1,0"), "11 pending");
}

/// `exit` lasts 50 ms and then gives way to the shape's own paint.
#[test]
fn canvas_auto_step_exit_expires_to_default_at_50ms() {
    assert_eq!(
        shown(ALL, "0,1,0; 60,1,0; 100,0,0; 149,0,0; 150,0,0"),
        "13220 idle"
    );
}

/// Re-entering during the exit window cancels it and starts `enter` — a whole new
/// window, measured from the re-entry.
#[test]
fn canvas_auto_step_reentry_during_exit_starts_enter() {
    assert_eq!(
        shown(ALL, "0,1,0; 60,1,0; 100,0,0; 120,1,0; 169,1,0; 170,1,0"),
        "132113 idle"
    );
}

/// Leaving during the enter window cancels it and starts `exit` — a whole new window,
/// measured from the departure.
#[test]
fn canvas_auto_step_leaving_during_enter_starts_exit() {
    assert_eq!(shown(ALL, "0,1,0; 20,0,0; 69,0,0; 70,0,0"), "1220 idle");
}

/// A press that began inside: pressed while the pointer is on the shape, out through
/// `exit` to the shape's own paint when dragged off, pressed again when dragged back,
/// and hover once the button is up.
#[test]
fn canvas_auto_step_a_press_begun_inside_is_pressed_again_when_dragged_back() {
    assert_eq!(
        shown(
            ALL,
            "0,1,0; 60,1,0; 70,1,1; 80,0,1; 140,0,1; 150,1,1; 160,1,0; 220,1,0"
        ),
        "13420413 idle"
    );
}

/// A press that began outside and is dragged in shows `hover` (after its enter flash),
/// never `pressed` — and a second button going down on the shape does not make it one:
/// the press is whichever began first.
#[test]
fn canvas_auto_step_a_press_begun_outside_and_dragged_in_shows_hover() {
    assert_eq!(
        shown(ALL, "0,0,1; 10,1,1; 70,1,1; 80,1,3; 90,1,0; 100,1,1"),
        "013334 idle"
    );
}

/// Any button is a press: the right button alone (bit 1) presses as the left does.
#[test]
fn canvas_auto_step_any_button_presses() {
    assert_eq!(shown(ALL, "0,1,0; 60,1,0; 70,1,2; 80,1,4"), "1344 idle");
}

/// A state whose flag is turned off while the instance is in it is left at once: a
/// re-`present` can change an item's flags under a window that is still running.
#[test]
fn canvas_auto_step_a_flag_turned_off_ends_its_window() {
    // The enter window is running after the first frame; the second steps the same
    // instance with `useEnter` off, 10 ms in, and it must show hover with no window
    // left pending. With the flag still on it would be the enter paint.
    assert_eq!(shown(ALL, "0,1,0; 10,1,0"), "11 pending");
    assert_eq!(
        shown_changing(ALL, EXIT | HOVER | PRESSED, "0,1,0; 10,1,0"),
        "13 idle"
    );
    // And the exit window: on the shape, then off it 10 ms later with `useExit` off.
    assert_eq!(shown(ALL, "0,1,0; 10,0,0"), "12 pending");
    assert_eq!(
        shown_changing(ALL, ENTER | HOVER | PRESSED, "0,1,0; 10,0,0"),
        "10 idle"
    );
}
