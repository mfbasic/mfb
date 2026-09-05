//! Regression tests for bug-536: codegen shapes that allocate an arena block with
//! exactly one owner and then never free it, so an ordinary loop grows without
//! bound.
//!
//! Each case builds the same program twice — once with the iteration count `N`,
//! once with `2N` — and asserts the child's **peak RSS** does not grow with the
//! count. A leak-free loop reads the same at both counts; a per-iteration leak
//! reads roughly double. The count is baked in as a literal rather than read from
//! `os::args()` so the measurement needs nothing from the harness but
//! `common::run_bounded_with_rss`, which reports `ru_maxrss` for the one child it
//! reaps (`getrusage(RUSAGE_CHILDREN)` would fold in the `mfb build` child).
//!
//! The threshold is a growth *delta*, not an absolute: the arena's geometric
//! chunk growth makes the absolute floor machine- and allocator-dependent, but a
//! leak-free program's floor does not move with the iteration count at all. 8 MB
//! of slack over a 4x count spread is far below every measured leak (25–100 MB)
//! and far above the noise (measured: identical to the byte at both counts).
//!
//! Each contrast program is the bug report's own "does not leak" line, kept as a
//! POSITIVE pin: it must stay flat, so a fix that made these tests pass by
//! disabling an allocation would be caught by the behaviour assertions instead.

mod common;

use std::time::Duration;

/// Peak RSS of `source` with `{N}` replaced by `count`, in bytes.
/// The RSS half is Unix-only: peak RSS comes from `wait4`'s `ru_maxrss`
/// (`common::run_bounded_with_rss`), which has no Windows equivalent here — the
/// same split `rt_json_bounds` and `rt_regex_bounds` already use. Gating these
/// three rather than the FILE keeps `every_return_shape_still_produces_the_right_value`
/// running on Windows, where it passes and is the half that checks the VALUES.
/// Ungated, they aborted the Windows row with `unix reports ru_maxrss`.
#[cfg(unix)]
fn peak_rss(name: &str, source: &str, count: u64) -> u64 {
    let program = source.replace("{N}", &count.to_string());
    let project = common::temp_project(&format!("{name}_{count}"), &program);
    let exe = common::build_project(&project);
    let (status, stdout, rss) = common::run_bounded_with_rss(
        &exe,
        Duration::from_secs(300),
        "the scope-drop leak probe did not finish",
    );
    assert!(
        status.success(),
        "{} exited non-zero:\n{stdout}",
        common::exit_description(&status)
    );
    let rss = rss.expect("unix reports ru_maxrss");
    let _ = std::fs::remove_dir_all(&project);
    rss
}

/// Assert the loop's peak RSS does not grow with its iteration count.
#[cfg(unix)]
fn assert_flat(name: &str, source: &str, small: u64, large: u64) {
    let a = peak_rss(name, source, small);
    let b = peak_rss(name, source, large);
    let grew = b.saturating_sub(a);
    assert!(
        grew < 8 * 1024 * 1024,
        "{name}: peak RSS grew {} MB between {small} and {large} iterations \
         ({} MB -> {} MB) — the loop leaks one block per iteration",
        grew / (1024 * 1024),
        a / (1024 * 1024),
        b / (1024 * 1024),
    );
}

// ---------------------------------------------------------------- shape A

/// `RETURN <RecordConstructor>` — the fresh block was re-materialised into a
/// second block by `store_pending_success_result` and the first was dropped from
/// the pending-temp list unfreed. 25 MB at 400k calls, 50 MB at 800k.
const SHAPE_A_CONSTRUCTOR: &str = "IMPORT io\n\
TYPE Plain\n  value AS Integer\n  index AS Integer\nEND TYPE\n\
FUNC mkLit(i AS Integer) AS Plain\n  RETURN Plain[i, i]\nEND FUNC\n\
SUB main()\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    LET p AS Plain = mkLit(i)\n\
    acc = acc + p.value\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// The same shape through a fresh **call** result rather than a constructor —
/// the bug report measured this as leaking identically, so it is its own case.
const SHAPE_A_CALL: &str = "IMPORT io\n\
TYPE Plain\n  value AS Integer\n  index AS Integer\nEND TYPE\n\
FUNC mkLocal(i AS Integer) AS Plain\n  LET r AS Plain = Plain[i, i]\n  RETURN r\nEND FUNC\n\
FUNC passThrough(i AS Integer) AS Plain\n  RETURN mkLocal(i)\nEND FUNC\n\
SUB main()\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    LET p AS Plain = passThrough(i)\n\
    acc = acc + p.index\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// The bug report's contrast case: `RETURN <owned local>` moves the block
/// (plan-25-C C1) and was always flat. A POSITIVE pin — the shape-A fix reaches
/// the same `lower_returned_value` and must not give this one a second free.
const SHAPE_A_CONTRAST_LOCAL: &str = "IMPORT io\n\
TYPE Plain\n  value AS Integer\n  index AS Integer\nEND TYPE\n\
FUNC mkLocal(i AS Integer) AS Plain\n  LET r AS Plain = Plain[i, i]\n  RETURN r\nEND FUNC\n\
SUB main()\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    LET p AS Plain = mkLocal(i)\n\
    acc = acc + p.value\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

#[cfg(unix)]
#[test]
fn returning_a_record_constructor_runs_at_constant_rss() {
    assert_flat("b536_shape_a_ctor", SHAPE_A_CONSTRUCTOR, 400_000, 800_000);
}

#[cfg(unix)]
#[test]
fn returning_a_fresh_call_result_runs_at_constant_rss() {
    assert_flat("b536_shape_a_call", SHAPE_A_CALL, 400_000, 800_000);
}

#[cfg(unix)]
#[test]
fn returning_an_owned_local_still_runs_at_constant_rss() {
    assert_flat(
        "b536_shape_a_local",
        SHAPE_A_CONTRAST_LOCAL,
        400_000,
        800_000,
    );
}

/// The positive behaviour pin for shape A: returning a constructor, a call
/// result, a local, a list literal and a nested constructor must all still
/// produce exactly the right values. A leak fix that changed which block the
/// caller receives would show up here, not in an RSS number.
const SHAPE_A_BEHAVIOUR: &str = "IMPORT io\n\
IMPORT collections\n\
TYPE Plain\n  value AS Integer\n  index AS Integer\nEND TYPE\n\
TYPE Duo\n  a AS Plain\n  n AS Integer\nEND TYPE\n\
TYPE Named\n  tag AS String\n  n AS Integer\nEND TYPE\n\
FUNC mkLit(i AS Integer) AS Plain\n  RETURN Plain[i, i * 2]\nEND FUNC\n\
FUNC mkLocal(i AS Integer) AS Plain\n  LET r AS Plain = Plain[i + 1, i + 2]\n  RETURN r\nEND FUNC\n\
FUNC mkThrough(i AS Integer) AS Plain\n  RETURN mkLocal(i)\nEND FUNC\n\
FUNC mkDuo(i AS Integer) AS Duo\n  LET q AS Plain = mkLocal(i)\n  RETURN Duo[q, i]\nEND FUNC\n\
FUNC mkNamed(i AS Integer) AS Named\n  RETURN Named[\"t\" & toString(i), i]\nEND FUNC\n\
FUNC mkList(i AS Integer) AS List OF Integer\n  RETURN [i, i + 1, i + 2]\nEND FUNC\n\
SUB main()\n\
  MUT out AS String = \"\"\n\
  MUT i AS Integer = 0\n\
  WHILE i < 4\n\
    LET a AS Plain = mkLit(i)\n\
    LET b AS Plain = mkLocal(i)\n\
    LET c AS Plain = mkThrough(i)\n\
    LET d AS Duo = mkDuo(i)\n\
    LET e AS Named = mkNamed(i)\n\
    LET f AS List OF Integer = mkList(i)\n\
    out = out & toString(a.value) & \",\" & toString(a.index) & \"|\"\n\
    out = out & toString(b.value) & \",\" & toString(b.index) & \"|\"\n\
    out = out & toString(c.value) & \",\" & toString(c.index) & \"|\"\n\
    out = out & toString(d.a.value) & \",\" & toString(d.n) & \"|\"\n\
    out = out & e.tag & \",\" & toString(e.n) & \"|\"\n\
    out = out & toString(collections::get(f, 2)) & \"//\"\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(out)\n\
END SUB\n";

#[test]
fn every_return_shape_still_produces_the_right_value() {
    let project = common::temp_project("b536_shape_a_behaviour", SHAPE_A_BEHAVIOUR);
    let exe = common::build_project(&project);
    let output = std::process::Command::new(&exe)
        .output()
        .expect("run the behaviour probe");
    assert!(
        output.status.success(),
        "{}",
        common::exit_description(&output.status)
    );
    let out = String::from_utf8(output.stdout).expect("utf8 stdout");
    let mut expected = String::new();
    for i in 0..4 {
        expected.push_str(&format!("{},{}|", i, i * 2));
        expected.push_str(&format!("{},{}|", i + 1, i + 2));
        expected.push_str(&format!("{},{}|", i + 1, i + 2));
        expected.push_str(&format!("{},{}|", i + 1, i));
        expected.push_str(&format!("t{},{}|", i, i));
        expected.push_str(&format!("{}//", i + 2));
    }
    assert_eq!(out.trim(), expected, "a RETURN shape changed its value");
    let _ = std::fs::remove_dir_all(&project);
}
