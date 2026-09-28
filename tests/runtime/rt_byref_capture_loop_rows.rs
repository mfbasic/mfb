//! bug-763: the `-O3` NIR loop rows must treat a `MUT` captured by reference
//! into `collections::forEach`'s lambda as redefined by that call.
//!
//! A by-ref capture lowers to `NirValue::LocalRef` in the closure's env; the
//! lifted lambda stores through the slot pointer, so the parent body never
//! spells the write as an `Assign`. The loop rows counted only `Bind`/`Assign`
//! targets as definitions, so at `-O3`:
//!
//! - LICM (`optimizer::opt1::licm::hoistable_bind`) saw `MUT total = 0` as a
//!   once-defined invariant bind and hoisted it out of the loop — the
//!   accumulator carried over (`6 12 18` instead of `6 6 6`);
//! - LICM and loop unswitching read the `plans::loops::loop_body_defined` set,
//!   so a comparison of an outer by-ref-captured `MUT` looked invariant — a
//!   `LET small = acc < 5` was hoisted, and an `IF acc < 5` was unswitched on
//!   its first-iteration value.
//!
//! Every program is built at `-O1` and `-O3`; both must print the literal
//! expected output, so the two levels cannot agree on a wrong answer.

#[path = "../common/mod.rs"]
mod common;

use std::path::Path;
use std::process::Command;

fn build_and_run(project: &Path, level: u8) -> String {
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg(format!("-O{level}"))
        .arg(project)
        .output()
        .expect("run mfb build");
    assert!(
        output.status.success(),
        "-O{level} build failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8 build output");
    let executable = stdout
        .lines()
        .find_map(|line| line.strip_prefix("Wrote executable to "))
        .expect("build output names the executable")
        .to_string();
    let run = Command::new(&executable)
        .current_dir(project)
        .output()
        .expect("run program");
    assert!(
        run.status.success(),
        "-O{level} program failed:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    String::from_utf8(run.stdout).expect("utf8 program output")
}

/// Build `body` at `-O1` and `-O3`; both must print `expected`.
fn assert_levels_agree(name: &str, body: &str, expected: &str) {
    let source = format!("IMPORT io\nIMPORT collections\n{body}");
    let project = common::temp_project(name, &source);
    let o1 = build_and_run(&project, 1);
    let o3 = build_and_run(&project, 3);
    let _ = std::fs::remove_dir_all(&project);
    assert_eq!(o1, expected, "-O1 output");
    assert_eq!(o3, expected, "-O3 output must match -O1");
}

/// The reported shape: a per-iteration accumulator bind in a FOR body.
#[test]
fn a_for_body_accumulator_is_reinitialized() {
    assert_levels_agree(
        "byref_loop_for",
        "FUNC main AS Integer
  LET xs AS List OF Integer = [1, 2, 3]
  FOR r = 0 TO 2
    MUT total AS Integer = 0
    collections::forEach(xs, LAMBDA(x AS Integer) -> total = total + x)
    io::print(toString(total))
  NEXT
  RETURN 0
END FUNC
",
        "6\n6\n6\n",
    );
}

/// The same bind in WHILE, DO .. LOOP UNTIL and FOR EACH bodies.
#[test]
fn every_loop_form_reinitializes_the_accumulator() {
    assert_levels_agree(
        "byref_loop_forms",
        "FUNC main AS Integer
  LET xs AS List OF Integer = [1, 2, 3]
  MUT r AS Integer = 0
  WHILE r < 2
    MUT a AS Integer = 0
    collections::forEach(xs, LAMBDA(x AS Integer) -> a = a + x)
    io::print(\"w\" & toString(a))
    r = r + 1
  END WHILE
  MUT k AS Integer = 0
  DO
    MUT b AS Integer = 0
    collections::forEach(xs, LAMBDA(x AS Integer) -> b = b + x)
    io::print(\"d\" & toString(b))
    k = k + 1
  LOOP UNTIL k >= 2
  FOR EACH y IN [7, 8]
    MUT c AS Integer = 0
    collections::forEach(xs, LAMBDA(x AS Integer) -> c = c + x)
    io::print(\"e\" & toString(c))
  NEXT
  RETURN 0
END FUNC
",
        "w6\nw6\nd6\nd6\ne6\ne6\n",
    );
}

/// Boolean and Float scalar accumulators (the LICM row's scalar class).
#[test]
fn boolean_and_float_accumulators_are_reinitialized() {
    assert_levels_agree(
        "byref_loop_scalars",
        "FUNC main AS Integer
  LET xs AS List OF Integer = [1, 2, 3]
  FOR r = 0 TO 2
    MUT seen AS Boolean = FALSE
    collections::forEach(xs, LAMBDA(x AS Integer) -> seen = seen OR x = r)
    MUT f AS Float = 0.0
    collections::forEach(xs, LAMBDA(x AS Integer) -> f = f + 0.5)
    io::print(toString(seen) & \" \" & toString(f))
  NEXT
  RETURN 0
END FUNC
",
        "FALSE 1.50\nTRUE 1.50\nTRUE 1.50\n",
    );
}

/// A read of an outer by-ref-captured `MUT` is not loop-invariant: neither
/// the `LET` (LICM) nor the `IF` condition (unswitching) may be evaluated once.
#[test]
fn a_read_of_a_byref_captured_outer_mut_is_not_invariant() {
    assert_levels_agree(
        "byref_loop_invariance",
        "FUNC main AS Integer
  LET xs AS List OF Integer = [1, 2, 3]
  MUT acc AS Integer = 0
  FOR r = 0 TO 2
    LET small AS Boolean = acc < 5
    io::print(toString(small))
    collections::forEach(xs, LAMBDA(x AS Integer) -> acc = acc + x)
  NEXT
  MUT acc2 AS Integer = 0
  FOR r = 0 TO 2
    IF acc2 < 5 THEN
      io::print(\"small\")
    ELSE
      io::print(\"big\")
    END IF
    collections::forEach(xs, LAMBDA(x AS Integer) -> acc2 = acc2 + x)
  NEXT
  RETURN 0
END FUNC
",
        "TRUE\nFALSE\nFALSE\nsmall\nbig\nbig\n",
    );
}
