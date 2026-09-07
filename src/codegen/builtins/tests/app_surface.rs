//! The `canvas::` and `term::` surfaces, in `-app` mode, on every backend.
//!
//! Both packages branch on the platform family and on the build mode. The
//! corpus lowers its 421 fixtures in CONSOLE mode for one backend, and
//! `canvas.rs` lowers one `-app` program to inspect the scene publish — so the
//! members neither of those touches (`canvas::getSize`, `setBytes`, `getBytes`,
//! `didResize`, `createImage`, and the `term::` shadow-grid helpers) had only
//! their console/Linux arm measured, or none at all.
//!
//! What is asserted is the same property the acceptance matrix spends five
//! runners on: the program lowers, completely, everywhere it is supposed to. An
//! `-app` member that lowers on Linux and not on macOS is a build failure nobody
//! sees until a release runner reaches it.

use crate::codegen::engine::types::NativeCodePlan;
use crate::testutil::{app_code_cached, code_for_src_cached, CodeTarget};

/// `os::resourcePath` in APP mode, where the base has a suffix to append.
///
/// `resource_base_offset` gives every build shape a `(strip, suffix)` pair, and
/// the suffix is empty for exactly two of the four: console and Windows-app,
/// where the executable sits beside its resources. macOS-app appends
/// `Resources` and Linux-app appends `share/<module>` — and the emitter has a
/// whole branch for that, one store per suffix byte plus the separator, plus a
/// different length calculation (`suffix.len() + 2` rather than `1`).
///
/// The corpus lowers in console mode, so only the empty-suffix arm had run: the
/// bytes that actually distinguish an app build's resource lookup were emitted
/// by nothing. A wrong length there is a heap write past the string it just
/// sized, and a missing separator is a path that silently resolves to a
/// directory that does not exist -- which reads as "the resource was not
/// installed" rather than as a compiler bug.
///
/// Windows-app is the fourth shape and it is NOT here: `os.resourcePath` is
/// unimplemented on `windows-x86_64` (bug-454). Its suffix would be empty in any
/// case -- the `.exe` sits beside its resources -- so nothing about this arm is
/// lost by its absence.
const OS_RESOURCE_PATH: &str = "\
IMPORT app
IMPORT io
IMPORT os

FUNC main() AS Integer
  app::setMode(app::Mode.Canvas)
  io::print(os::resourcePath(\"music/song.ogg\"))
  RETURN 0
END FUNC
";

/// The suffix-appending arm lowers on the two backends that HAVE a suffix.
///
/// Three of the four app-capable backends are excluded, and for two different
/// reasons that are both worth naming.
///
/// `windows-x86_64` does not implement `os.resourcePath` at all -- it is absent
/// from that backend's `SUPPORTED_RUNTIME_CALLS`, and the shared exe-path
/// acquisition answers "not implemented for Windows" rather than emitting, which
/// `gen_paths.rs` documents as deliberate: a diagnostic instead of an ICE for
/// whoever opens the gate first. That is bug-454, still Open, and this test is
/// not the place to work around it -- when the gate opens, adding the target
/// here is one line.
///
/// `linux-aarch64` and `linux-riscv64` share `LinuxApp` with `linux-x86_64`, so
/// they compute the same `share/<module>` suffix through the same emitter; one
/// Linux backend is what makes the arm run, and driving three costs three
/// lowerings for one code path. macOS is separate because its suffix is a
/// different string (`Resources`) through the same branch.
#[test]
fn the_resource_path_base_suffix_lowers_where_the_base_has_one() {
    let source = OS_RESOURCE_PATH.to_string();
    for target in [CodeTarget::MacosAarch64, CodeTarget::LinuxX86_64] {
        assert!(
            target.app_mode().is_some(),
            "{}: this test is about the APP-mode base suffix, so the target must \
             have an app mode",
            target.name()
        );
        let plan = app_code_cached(&source, target);
        assert!(
            plan.functions
                .iter()
                .any(|function| function.name.contains("main")),
            "{}: an -app program calling os::resourcePath must lower",
            target.name()
        );
    }
}

/// A scene GROUP, which is the only call in the language that consumes a
/// resource held inside its argument.
///
/// `canvas::setGroup` is the sole `add_consuming_parameter` row in the whole
/// registry (`func_set_group.rs`), so it is the only way to reach
/// `deactivate_consumed_cleanups` — 40 lines of `builder_resource_cleanup.rs`
/// that drop the caller's close obligation for every resource reachable from
/// the items list, transitively.
///
/// The obligation has to move, not be shared. A group takes over closing the
/// images its items name, which is what lets `img` go out of scope here without
/// the group losing its picture; if the scope ALSO closed it, the group would be
/// drawing a closed image, and `canvas::imageHandle` answers 0 for one — the
/// same 0 that means "no such object". So the failure mode is a silently blank
/// picture with no error anywhere, which is why
/// `tests/syntax/resources/canvas-setgroup-consumes-items` refuses the sharing
/// case at compile time. That fixture stops at the diagnostic, so nothing had
/// ever lowered the ACCEPTED half.
///
/// The image is reached through `Picture.image`, not named in the argument at
/// all: the list holds `a`, whose type is a `DrawItem` union. That is the
/// transitive step, and a walk that only looked at the names written in the
/// argument would find `a` and deactivate nothing.
const CANVAS_GROUP: &str = "\
IMPORT app
IMPORT canvas
IMPORT color
IMPORT io

FUNC main() AS Integer
  app::setMode(app::Mode.Canvas)
  LET px AS List OF Byte = [toByte(1), toByte(2), toByte(3), toByte(4)]
  RES img AS canvas::Image = canvas::createImage(1, 1, px) TRAP(e)
    RETURN 1
  END TRAP
  LET a AS canvas::DrawItem = canvas::Picture[x := 0.0, y := 0.0, w := 8.0, h := 8.0, image := img, paint := canvas::fill(color::rgb(255, 255, 255))]
  canvas::setGroup(\"one\", [a])
  io::print(\"grouped\")
  RETURN 0
END FUNC
";

/// The image surface: create, size, read back, write, and the resize flag.
const CANVAS_SURFACE: &str = "\
IMPORT app
IMPORT canvas
IMPORT collections
IMPORT io

FUNC main() AS Integer
  app::setMode(app::Mode.Canvas)
  MUT px AS List OF Byte = []
  FOR i = 0 TO 15
    px = collections::append(px, toByte(i))
  NEXT
  RES img AS canvas::Image = canvas::createImage(2, 2, px) TRAP(e)
    RETURN 1
  END TRAP
  LET size AS canvas::Size = canvas::getSize(img)
  LET back AS List OF Byte = canvas::getBytes(img)
  canvas::setBytes(img, back)
  io::print(toString(len(back)) & toString(canvas::didResize()))
  RETURN 0
END FUNC
";

/// Every `-app`-capable backend lowers the canvas surface completely.
#[test]
fn the_canvas_surface_lowers_on_every_app_capable_backend() {
    let mut agreed: Option<Vec<String>> = None;
    let mut lowered = 0;
    for target in CodeTarget::ALL {
        if target.app_mode().is_none() {
            continue;
        }
        let plan = app_code_cached(CANVAS_SURFACE, target);
        lowered += 1;
        let surface = runtime_members(plan, "runtime.canvas.");
        for member in ["createImage", "imageHandle", "destroyImage"] {
            assert!(
                surface.contains(&format!("runtime.canvas.{member}")),
                "{}: a program that creates an image must emit \
                 `runtime.canvas.{member}`; it emitted {surface:?}",
                target.name()
            );
        }
        match &agreed {
            None => agreed = Some(surface),
            Some(first) => assert_eq!(
                first,
                &surface,
                "{}: the canvas runtime surface differs from the first backend's",
                target.name()
            ),
        }
    }
    assert_eq!(lowered, 4, "four backends have an -app mode");
}

/// The whole `term::` drawing surface in one program.
///
/// The corpus reaches `term::` through `func_term_drawText_valid`, which calls
/// six members — `on`, `setForeground`, `drawText`, `drawGlyph`, `sync`, `off`.
/// In an `-app` build each member has a SECOND emitter that writes into the
/// toolkit's shadow grid instead of to a tty, and the ones that fixture does not
/// call had never been lowered in either backend that has them: `emit_app_draw_line`
/// (111 lines), `emit_app_draw_box` (107), `emit_app_fill_rect` (74),
/// `emit_app_move_to` (62), `emit_app_terminal_size` (58) and `emit_app_clear`
/// (42) in `target/macos_aarch64/app/app_io.rs`, and their `target/linux_gtk`
/// counterparts.
///
/// A drawing member is exactly the kind that a console run cannot vouch for: the
/// tty arm writes escape sequences a golden can compare, and the app arm writes
/// cells into a grid nothing in a headless test ever reads back.
const TERM_SURFACE: &str = "\
IMPORT color
IMPORT term

FUNC main() AS Integer
  term::on()
  LET size AS term::TermSize = term::terminalSize()
  term::clear()
  term::hideCursor()
  term::moveTo(1, 1)
  term::setForeground(color::rgb(255, 255, 0))
  term::setBackground(color::rgb(0, 0, 40))
  term::setBold(true)
  term::setUnderline(true)
  term::drawText(2, 3, \"Hello, TUI!\")
  term::drawGlyph(0, 0, 9731)
  term::drawHLine(term::LineStyle.Light, 4, 0, 20)
  term::drawVLine(term::LineStyle.Double, 5, 2, 10)
  term::drawBox(term::LineStyle.Heavy, 6, 4, 9, 24)
  term::fillRect(term::FillStyle.Light, 10, 4, 12, 24)
  term::showCursor()
  term::sync()
  term::off()
  RETURN size.rows
END FUNC
";

/// A `term::` program lowers in BOTH build modes, on every backend.
///
/// `term::` is where the two modes genuinely differ: in console mode the helpers
/// write escape sequences to a tty, and in `-app` mode they write into the
/// toolkit's shadow grid instead. Only one of those two arms was ever lowered,
/// because every fixture the corpus runs is a console build.
#[test]
fn a_term_program_lowers_in_both_build_modes_on_every_backend() {
    let source = TERM_SURFACE.to_string();
    let mut lowered = 0;
    for target in CodeTarget::ALL {
        let console = code_for_src_cached(&source, target, crate::target::NativeBuildMode::Console);
        assert!(
            !runtime_members(console, "runtime.term.").is_empty(),
            "{}: a term program must emit term runtime members in console mode",
            target.name()
        );
        lowered += 1;
        let Some(_) = target.app_mode() else {
            continue;
        };
        let app = app_code_cached(&source, target);
        assert!(
            app.functions.len() > console.functions.len(),
            "{}: the -app build must emit the toolkit bootstrap on top of the \
             console surface ({} functions against {})",
            target.name(),
            app.functions.len(),
            console.functions.len()
        );
        lowered += 1;
    }
    assert_eq!(
        lowered, 9,
        "five console builds plus four -app builds (rv64 is console-only)"
    );
}

/// The `io::` members that route to the app transcript window instead of a tty.
///
/// `mfb man io`: "in app mode all of them are routed to the application
/// transcript window, which is treated as an interactive terminal". That
/// routing is a whole second arm of `gen_is_terminal` and `func_flush` -- in
/// console mode they call `isatty(fd)` and drain the stdout buffer through
/// `write()`; in `-app` mode they ask the platform to append a hook into the
/// body instead. Every fixture the corpus runs is a console build, so the app
/// arm of both had never lowered on any backend.
const APP_IO_SURFACE: &str = "\
IMPORT app
IMPORT io

FUNC main() AS Integer
  app::setMode(app::Mode.Canvas)
  io::flush()
  MUT n AS Integer = 0
  IF io::isInputTerminal() THEN
    n = n + 1
  END IF
  IF io::isOutputTerminal() THEN
    n = n + 2
  END IF
  IF io::isErrorTerminal() THEN
    n = n + 4
  END IF
  io::print(\"io=\" & toString(n))
  RETURN 0
END FUNC
";

/// The app-mode `io::` arm lowers on every backend that has an app mode.
///
/// A backend whose app surface does not implement one of these hooks refuses by
/// NAME (`native target '<t>' does not support app-mode io helpers`) rather than
/// emitting a body that silently does nothing -- which is the failure this asserts
/// against, because a `flush` that lowered to no instructions in app mode would
/// lose buffered output with nothing to read in the build log.
#[test]
fn the_app_mode_io_helpers_lower_on_every_app_capable_backend() {
    let source = APP_IO_SURFACE.to_string();
    let mut lowered = 0;
    for target in CodeTarget::ALL {
        let Some(_) = target.app_mode() else {
            continue;
        };
        let app = app_code_cached(&source, target);
        assert!(
            !runtime_members(app, "runtime.io.").is_empty(),
            "{}: an -app io program must still emit io runtime members",
            target.name()
        );
        lowered += 1;
    }
    assert_eq!(lowered, 4, "four -app builds (riscv64 is console-only)");
}

fn runtime_members(plan: &NativeCodePlan, prefix: &str) -> Vec<String> {
    let mut names: Vec<String> = plan
        .functions
        .iter()
        .map(|f| f.name.clone())
        .filter(|name| name.starts_with(prefix))
        .collect();
    names.sort();
    names
}

/// The consuming call lowers on every app-capable backend.
///
/// One assertion beyond "it lowered": `main` must still be a real body. A
/// cleanup deactivation that removed the wrong obligation would not fail to
/// build -- it would emit a function whose scope exit closes nothing, or closes
/// something twice -- so what this pins is that the four backends agree the
/// program is lowerable at all, which is what the acceptance matrix cannot check
/// without four runners.
#[test]
fn the_scene_group_lowers_on_every_app_capable_backend() {
    let source = CANVAS_GROUP.to_string();
    let mut lowered = 0;
    for target in CodeTarget::ALL {
        let Some(_) = target.app_mode() else {
            continue;
        };
        let plan = app_code_cached(&source, target);
        assert!(
            plan.functions
                .iter()
                .any(|function| function.name.contains("main")),
            "{}: the group program must lower to a real program",
            target.name()
        );
        lowered += 1;
    }
    assert_eq!(
        lowered, 4,
        "four of the five backends have an app mode; a count that fell would          mean this ran on fewer than it reads as covering"
    );
}
