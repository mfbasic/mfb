//! The console I/O members in `-app` mode, on every backend that has one.
//!
//! In an `--app` build there is no terminal: standard input is a window pipe, output goes
//! to the transcript, and `term::` draws on a character grid that exists only in
//! `app::Mode.Console`. The members therefore branch on the build mode, and the branches
//! are platform hooks — `emit_app_io_input`, `emit_app_raw_input_mode`,
//! `emit_app_term_helper` — that only an `-app` lowering reaches. The corpus lowers in
//! console mode, so each member's app arm (and the wrong-mode gate `term::` prepends
//! outside `Console`) had no lowering.
//!
//! One program calls them all (each reader with and without a timeout) in `Mode.Console` and is lowered for macOS, Linux (both
//! ISAs) and Windows; what is asserted is what differs from the console build: no stdout
//! buffer to drain (`io::setBuffered` is a no-op in app mode), a raw-input switch before
//! `readKey`, and every member lowering at all.

use crate::arch::ops::CodeOp;
use crate::codegen::engine::tests::test_support::Stream;
use crate::codegen::engine::types::NativeCodePlan;
use crate::target::NativeBuildMode::Console;
use crate::testutil::{app_code_cached, code_for_src_cached, CodeTarget};

const APP_IO: &str = "\
IMPORT app
IMPORT io
IMPORT term

FUNC main() AS Integer
  app::setMode(app::Mode.Console)
  io::setBuffered(TRUE)
  term::enableMouse(TRUE)
  LET ev = term::pollMouse()
  LET name = io::input(\"name? \")
  LET line = io::readLine()
  LET key = io::readKey()
  LET scalar = io::readScalar()
  LET grapheme = io::readGrapheme()
  LET raw = io::readRaw()
  LET timedLine = io::readLine(50)
  LET timedKey = io::readKey(50)
  LET timedScalar = io::readScalar(50)
  LET timedGrapheme = io::readGrapheme(50)
  LET timedRaw = io::readRaw(50)
  LET interactive = io::isInputTerminal()
  io::print(name & line)
  RETURN 0
END FUNC
";

/// The same members with no `app` import: a console build.
const CONSOLE_IO: &str = "\
IMPORT io

FUNC main() AS Integer
  io::setBuffered(TRUE)
  LET name = io::input(\"name? \")
  LET line = io::readLine()
  LET key = io::readKey()
  io::print(name & line)
  RETURN 0
END FUNC
";

fn drains_in(plan: &NativeCodePlan, member: &str) -> bool {
    plan.functions
        .iter()
        .filter(|function| function.symbol.contains(member))
        .any(|function| {
            Stream::of(function)
                .calls_between(0, function.instructions.len())
                .iter()
                .any(|call| call == crate::codegen::error::constants::STDOUT_DRAIN_SYMBOL)
        })
}

fn has_op(plan: &NativeCodePlan, member: &str, op: CodeOp) -> bool {
    plan.functions
        .iter()
        .filter(|function| function.symbol.contains(member))
        .any(|function| function.instructions.iter().any(|i| i.op == op))
}

/// The members lower in app mode on every backend that has one.
#[test]
fn the_console_io_members_lower_in_app_mode() {
    for target in CodeTarget::ALL {
        if target.app_mode().is_none() {
            continue;
        }
        let plan = app_code_cached(APP_IO, target);
        assert!(
            plan.functions
                .iter()
                .any(|function| function.symbol.contains("setBuffered")),
            "{}: io::setBuffered must lower in app mode",
            target.name()
        );
    }
}

/// App mode has no stdout buffer: `io::setBuffered` drains nothing there, while the
/// console build's drains when buffering is switched off.
#[test]
fn set_buffered_has_nothing_to_drain_in_app_mode() {
    for target in CodeTarget::ALL {
        if target.app_mode().is_none() {
            continue;
        }
        let app = app_code_cached(APP_IO, target);
        assert!(
            !drains_in(app, "setBuffered"),
            "{}: app mode has no stdout buffer to drain",
            target.name()
        );
        let console = code_for_src_cached(CONSOLE_IO, target, Console);
        assert!(
            drains_in(console, "setBuffered"),
            "{}: the console build drains the buffer when it is switched off",
            target.name()
        );
    }
}

/// `readKey` in app mode switches the window to raw (one key at a time) input first,
/// which a console build does not.
#[test]
fn read_key_switches_an_app_window_to_raw_input() {
    for target in [CodeTarget::MacosAarch64, CodeTarget::LinuxX86_64] {
        let app = app_code_cached(APP_IO, target);
        let console = code_for_src_cached(CONSOLE_IO, target, Console);
        let app_calls: usize = app
            .functions
            .iter()
            .filter(|f| f.symbol.contains("readKey"))
            .map(|f| Stream::of(f).calls_between(0, f.instructions.len()).len())
            .sum();
        let console_calls: usize = console
            .functions
            .iter()
            .filter(|f| f.symbol.contains("readKey"))
            .map(|f| Stream::of(f).calls_between(0, f.instructions.len()).len())
            .sum();
        assert_ne!(
            app_calls,
            console_calls,
            "{}: the app and console readKey are different lowerings",
            target.name()
        );
        // Both lower a key read; neither is empty.
        assert!(has_op(app, "readKey", CodeOp::Label) && has_op(console, "readKey", CodeOp::Label));
    }
}
