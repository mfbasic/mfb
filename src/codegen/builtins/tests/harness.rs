//! The harness reports the CAUSE of a lowering failure, not its consequence.
//!
//! `testutil`'s lowering entry points run the concrete HIR straight into codegen
//! and never run the build's source checkers — that is deliberate, because the
//! corpus lowers hundreds of programs across five backends per run and
//! `ir::shape::collect_diagnostics` on every one of them is not free.
//!
//! The cost of skipping them is that a program naming something the language
//! does not have gets all the way to codegen, where it surfaces as an undefined
//! relocation: `internal relocation target 'canvas.rgb' is not defined`. That
//! reads as a codegen bug in a file nobody touched. The actual cause — a member
//! that moved to another package — is not in the message at all.
//!
//! So the checkers run on the FAILURE path only, and this pins that they do. It
//! is a test about a test harness, which is worth having exactly once: every
//! suite under this directory depends on a failure message that points at the
//! program rather than at the compiler.

use crate::target::NativeBuildMode::Console;
use crate::testutil::{try_code_for_src, CodeTarget};

/// A program naming a member no package exports.
///
/// `canvas::rgb` is a real historical example: plan-122-D moved the colour model
/// out of `canvas` into `color`, and every in-process canvas test failed with
/// the undefined-relocation message above until someone read the fixture that
/// documents the rename.
const UNRESOLVED_MEMBER: &str = "\
IMPORT canvas
IMPORT io

FUNC main() AS Integer
  LET c AS canvas::Color = canvas::rgb(10, 20, 30)
  io::print(\"built\")
  RETURN 0
END FUNC
";

/// A program that lowers cleanly, so the explanatory pass must stay silent.
const FINE: &str = "\
IMPORT io

FUNC main() AS Integer
  io::print(\"ok\")
  RETURN 0
END FUNC
";

/// A failure caused by an unresolved name says so.
#[test]
fn a_lowering_failure_with_a_source_error_reports_the_source_error() {
    // `NativeCodePlan` is not `Debug`, so `expect_err` is unavailable.
    let Err(err) = try_code_for_src(UNRESOLVED_MEMBER, CodeTarget::LinuxX86_64, Console) else {
        panic!("a program naming a member no package exports must not lower");
    };
    assert!(
        err.contains("source checkers"),
        "the harness must say the lowering failure is a consequence of a source \
         error, not report the codegen symptom alone; it said: {err}"
    );
    // `TYPE_UNKNOWN_VALUE`, not `SYMBOL_UNKNOWN_IDENTIFIER`: `check_src` is
    // `ir::shape` + `ir::verify`, and the resolver's own diagnostic is upstream
    // of both. What survives is that the binding's value has no type — which is
    // still a statement about the PROGRAM, which is the whole point.
    assert!(
        err.contains("TYPE_UNKNOWN_VALUE"),
        "the explanation must name the rule that fired, which is the thing that \
         points at the program rather than at the compiler; it said: {err}"
    );
}

/// A program with no source errors gets no explanation appended.
///
/// Without this, the assertion above would pass just as well if the harness
/// appended the note unconditionally — and every genuine codegen failure would
/// then carry a misleading "this is a source error" line.
#[test]
fn a_clean_program_lowers_with_no_explanation_attached() {
    try_code_for_src(FINE, CodeTarget::LinuxX86_64, Console)
        .expect("a program with no source errors must lower");
}
