//! bug-744: no `-O` level may fold a module binding's initializer into a read
//! that can run before that binding is initialized.
//!
//! Top-level bindings initialize one at a time, in declaration order, and a
//! binding whose store has not run yet reads zero. An early initializer that
//! calls a function reading a LATER binding therefore observes `0`, and one
//! that reads the later binding directly does too. The Level-2 constification
//! row (`optimizer::opt1::globals`) replaced every read of a never-written
//! private global with its literal — including those two — so `-O2` printed
//! `10 / 6` where `-O0` and `-O1` printed `0 / 1`: the dial changed a result,
//! which its contract forbids. The fix keeps every read in a function the
//! global initializer can reach (and in the initializer itself).
//!
//! Each case builds the same program at every level and requires the `-O0`
//! answer everywhere. The last line of each program reads the binding from
//! `main`, after initialization, so the controls also pin that a correct read
//! still sees the initializer's value once it has run.

#[path = "../common/mod.rs"]
mod common;
use common::{mfb_exe, temp_project};
use std::process::Command;

/// `A` and `C` initialize before `B`: `A` through a call, `C` directly.
fn program(binding: &str) -> String {
    format!(
        "\
IMPORT io\n\
LET A AS Integer = f()\n\
LET C AS Integer = B + 1\n\
{binding} B AS Integer = 5\n\
\n\
FUNC f() AS Integer\n\
  RETURN B * 2\n\
END FUNC\n\
\n\
SUB main()\n\
  io::print(toString(A))\n\
  io::print(toString(C))\n\
  io::print(toString(B))\n\
END SUB\n"
    )
}

/// Build `source` at `-O<level>` and return what the program prints.
fn run_at(name: &str, source: &str, level: u8) -> String {
    let project = temp_project(name, source);
    let output = Command::new(mfb_exe())
        .arg("build")
        .arg(format!("-O{level}"))
        .arg(&project)
        .output()
        .expect("run mfb build");
    assert!(
        output.status.success(),
        "-O{level} build failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8 build output");
    let executable = stdout
        .lines()
        .find_map(|line| line.strip_prefix("Wrote executable to "))
        .expect("build output names the executable");
    let run = Command::new(executable).output().expect("run program");
    assert!(run.status.success(), "-O{level} program failed");
    String::from_utf8(run.stdout).expect("utf8 program output")
}

fn same_answer_at_every_level(name: &str, binding: &str) {
    let source = program(binding);
    let unoptimized = run_at(name, &source, 0);
    assert_eq!(
        unoptimized, "0\n1\n5\n",
        "the reference: both early reads see the uninitialized slot, main sees 5"
    );
    for level in 1..=3 {
        assert_eq!(
            run_at(name, &source, level),
            unoptimized,
            "-O{level} changed what `{binding} B` reads during initialization"
        );
    }
}

/// The Level-2 constification row's own target: a private, never-written
/// `LET`.
#[test]
fn private_let_read_during_initialization() {
    same_answer_at_every_level("init_order_private_let", "PRIVATE LET");
}

/// A private `MUT` nothing writes is constified the same way.
#[test]
fn private_mut_read_during_initialization() {
    same_answer_at_every_level("init_order_private_mut", "PRIVATE MUT");
}

/// A public `LET`: out of the Level-2 row's reach (not private), pinned here so
/// any row that folds `LET`s regardless of visibility inherits the same rule.
#[test]
fn public_let_read_during_initialization() {
    same_answer_at_every_level("init_order_public_let", "LET");
}
