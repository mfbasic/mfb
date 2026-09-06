//! bug-149: a console read inside `term::` raw mode restores the line discipline.
//!
//! `term::on()` puts stdin in single-key raw mode — no echo, no waiting for
//! Return, which is what a TUI needs. `io::readLine` needs the opposite. So when
//! a program uses BOTH, the read helper brackets itself: restore the saved
//! cooked discipline before reading, put raw mode back after.
//!
//! `emit_console_raw_line_mode` is that bracket, and the whole function was
//! never lowered. Its guard is `console_term_state`, which is `Some` only for a
//! program that allocates term state AND calls a console read, and no committed
//! fixture does both — `tests/rt-behavior/term/**` never reads, and the io read
//! fixtures never call `term::on`. The two examples that do (`hangman`, `life`)
//! are examples, which no unit test lowers. That left
//! `codegen/io/terminal/io_terminal.rs` at 69.10%, with the whole of
//! `emit_console_raw_line_mode` uncovered.
//!
//! Without the bracket the failure is not a crash. `io::readLine` inside a TUI
//! returns after ONE keystroke, un-echoed — the program reads a one-character
//! name and carries on, which looks like a game that ignored your input rather
//! than like a compiler bug.

use crate::codegen::engine::types::NativeCodePlan;
use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_src_cached, CodeTarget};

/// `term::on()` and `io::readLine()` in one program: the bug-149 shape.
const TERM_AND_READ: &str = "\
IMPORT io
IMPORT term

FUNC main() AS Integer
  term::on()
  term::moveTo(1, 1)
  LET name AS String = io::readLine()
  term::off()
  io::print(name)
  RETURN 0
END FUNC
";

/// The same read with no `term::` anywhere: nothing to restore.
const READ_ONLY: &str = "\
IMPORT io

FUNC main() AS Integer
  LET name AS String = io::readLine()
  io::print(name)
  RETURN 0
END FUNC
";

fn plan(source: &str, target: CodeTarget) -> &'static NativeCodePlan {
    code_for_src_cached(source, target, Console)
}

/// The labels `runtime.io.readLine` defines, for `source` on `target`.
fn read_line_labels(source: &str, target: CodeTarget) -> Vec<String> {
    plan(source, target)
        .functions
        .iter()
        .filter(|f| f.name == "runtime.io.readLine")
        .flat_map(|f| f.instructions.iter())
        .filter(|i| i.op == crate::arch::ops::CodeOp::Label)
        .filter_map(|i| i.get("name"))
        .collect()
}

/// A program that uses `term::` and reads gets BOTH halves of the bracket.
///
/// Both, not one: restoring the cooked discipline and never putting raw mode
/// back would leave the TUI echoing every subsequent keystroke into the drawn
/// screen, which is the same bug from the other end.
#[test]
fn a_console_read_under_term_brackets_itself_with_the_line_discipline() {
    for target in [
        CodeTarget::MacosAarch64,
        CodeTarget::LinuxAarch64,
        CodeTarget::LinuxX86_64,
        CodeTarget::LinuxRiscv64,
        CodeTarget::WindowsX86_64,
    ] {
        let labels = read_line_labels(TERM_AND_READ, target);
        assert!(
            !labels.is_empty(),
            "{}: the program calls `io::readLine`, so the plan must define \
             `runtime.io.readLine`",
            target.name()
        );
        for half in ["_console_line_mode_skip", "_console_raw_mode_skip"] {
            assert!(
                labels.iter().any(|name| name.ends_with(half)),
                "{}: a program using `term::` and `io::readLine` must restore \
                 the cooked line discipline around the read and put raw mode \
                 back after (bug-149); no label ending `{half}` was emitted. \
                 The labels are {labels:?}",
                target.name()
            );
        }
    }
}

/// A program with no `term::` gets neither half.
///
/// The bracket reads and writes the term state block, which a program without
/// `term::` never allocates — so emitting it there would load through an offset
/// into whatever the arena happens to hold. It is also what makes the test
/// above mean something: without this, a bracket emitted unconditionally would
/// pass it.
#[test]
fn a_console_read_with_no_term_state_emits_no_bracket() {
    let labels = read_line_labels(READ_ONLY, CodeTarget::LinuxX86_64);
    assert!(
        !labels.is_empty(),
        "the program calls `io::readLine`, so the plan must define \
         `runtime.io.readLine`"
    );
    for half in ["_console_line_mode_skip", "_console_raw_mode_skip"] {
        assert!(
            !labels.iter().any(|name| name.ends_with(half)),
            "a program with no `term::` allocates no term state, so the bracket \
             must not be emitted -- it would read through an offset into \
             whatever the arena happens to hold. Found a label ending `{half}` \
             in {labels:?}"
        );
    }
}
