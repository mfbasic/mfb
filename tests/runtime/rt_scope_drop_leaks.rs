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

#[path = "../common/mod.rs"]
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

// ---------------------------------------------------------------- shape B

/// `acc = acc + len(toString(i))` — a `String` produced by a call and consumed
/// by an operator, never bound. `register_pending_temp` returned early for every
/// `String`, so nothing ever freed the block: 25 MB at 400k evaluations, 50 MB at
/// 800k (measured on the pre-fix compiler). The fix gives the shared String
/// producers fail-closed provenance (`mark_fresh_string`) so this one is freed at
/// statement end.
const SHAPE_B_CALL_RESULT: &str = "IMPORT io\n\
SUB main()\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    acc = acc + len(toString(i))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// The same shape through the concatenation operator: `"x" & toString(i) & "y"`
/// allocates the fused chain's block and hands it straight to `len`.
const SHAPE_B_CONCAT: &str = "IMPORT io\n\
SUB main()\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    acc = acc + len(\"x\" & toString(i) & \"y\")\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// The native `strings::` producers, unbound. `strings::padLeft(s, n)` also
/// leaked an INTERIOR block — the one-byte default pad String it materializes and
/// copies into its result — which no result-shaped fix can reach; it is freed by
/// `register_fresh_string_temp`.
const SHAPE_B_NATIVE_PRODUCERS: &str = "IMPORT io\n\
IMPORT collections\n\
IMPORT strings\n\
SUB main()\n\
  MUT names AS List OF String = []\n\
  MUT k AS Integer = 0\n\
  WHILE k < 40\n\
    names = collections::append(names, \"name\" & toString(k))\n\
    k = k + 1\n\
  END WHILE\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  MUT sink AS String = \"seed\"\n\
  WHILE i < {N}\n\
    acc = acc + len(collections::get(names, i MOD 40))\n\
    acc = acc + len(strings::trim(sink & \"  \"))\n\
    acc = acc + len(strings::left(collections::get(names, i MOD 40), 2))\n\
    acc = acc + len(strings::padLeft(toString(i), 12))\n\
    acc = acc + len(strings::upper(sink & \"z\"))\n\
    acc = acc + len(strings::mid(sink & \"abcdef\", 1, 3))\n\
    acc = acc + len(strings::replace(sink & \"abc\", \"a\", \"zz\"))\n\
    acc = acc + len(strings::join(names, \",\"))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// The bug report's contrast line — binding the call result already freed it —
/// wrapped around the two shapes that did NOT: the `toString(i)` operand inside
/// `toString(i) & "-"`, and `strings::padLeft`'s interior pad String. It is
/// therefore RED pre-fix too (128 B per iteration: 50 MB at 400k, 99 MB at 800k).
///
/// What it pins is the other half: the fix now ALSO registers the *bound*
/// results as temps, and `lower_value_owned`'s `claim_pending_temp` is the only
/// thing keeping that from becoming a second free. If the claim ever stopped
/// matching, this program would abort on a double free rather than leak.
const SHAPE_B_CONTRAST_BOUND: &str = "IMPORT io\n\
IMPORT strings\n\
SUB main()\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  MUT s AS String = \"\"\n\
  WHILE i < {N}\n\
    LET t AS String = toString(i)\n\
    LET u AS String = strings::padLeft(t, 9)\n\
    s = toString(i) & \"-\"\n\
    acc = acc + len(t) + len(u) + len(s)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

#[cfg(unix)]
#[test]
fn an_unbound_string_call_result_runs_at_constant_rss() {
    assert_flat("b536_shape_b_call", SHAPE_B_CALL_RESULT, 400_000, 800_000);
}

#[cfg(unix)]
#[test]
fn an_unbound_string_concat_runs_at_constant_rss() {
    assert_flat("b536_shape_b_concat", SHAPE_B_CONCAT, 400_000, 800_000);
}

#[cfg(unix)]
#[test]
fn unbound_native_string_producers_run_at_constant_rss() {
    assert_flat(
        "b536_shape_b_native",
        SHAPE_B_NATIVE_PRODUCERS,
        100_000,
        200_000,
    );
}

#[cfg(unix)]
#[test]
fn a_bound_string_call_result_still_runs_at_constant_rss() {
    assert_flat(
        "b536_shape_b_bound",
        SHAPE_B_CONTRAST_BOUND,
        400_000,
        800_000,
    );
}

/// The positive behaviour pin for shape B, and the one that matters most: the
/// fix ADDS `arena_free`s, so the failure mode it risks is a **wild free**, not a
/// leak. Every String kind the guards deliberately exclude is exercised here
/// unbound and in a churning loop, so a block freed by mistake is either read
/// back wrong or corrupts the free list and faults a later allocation:
///
/// * a rodata literal returned from a callee (`lit`) — `arena_free` on rodata is
///   SIGBUS;
/// * `toString(String)`, whose `String` arm hands back its own ARGUMENT (`ident`);
/// * a callee that returns one of its parameters (`pick`) — the plan-86 K1
///   param-borrow;
/// * a `String` element read out of a `List OF String`, whose container must
///   survive the read;
/// * `strings::*` results both bound and unbound, mixed;
/// * a live `MUT String` reassigned every iteration, and a fresh list allocated
///   every iteration so a corrupted free list surfaces as a later fault.
const SHAPE_B_BEHAVIOUR: &str = "IMPORT io\n\
IMPORT collections\n\
IMPORT strings\n\
FUNC pick(a AS String, b AS String, useA AS Boolean) AS String\n\
  IF useA THEN\n    RETURN a\n  END IF\n  RETURN b\n\
END FUNC\n\
FUNC lit(i AS Integer) AS String\n\
  IF i MOD 2 = 0 THEN\n    RETURN \"even-literal\"\n  END IF\n  RETURN \"odd-literal\"\n\
END FUNC\n\
FUNC ident(s AS String) AS String\n  RETURN toString(s)\nEND FUNC\n\
SUB main()\n\
  MUT names AS List OF String = []\n\
  MUT k AS Integer = 0\n\
  WHILE k < 8\n\
    names = collections::append(names, \"name\" & toString(k))\n\
    k = k + 1\n\
  END WHILE\n\
  MUT total AS Integer = 0\n\
  MUT sink AS String = \"seed\"\n\
  MUT i AS Integer = 0\n\
  WHILE i < 2000\n\
    total = total + len(toString(i))\n\
    total = total + len(\"x\" & toString(i) & \"y\")\n\
    total = total + len(lit(i))\n\
    total = total + len(ident(sink))\n\
    total = total + len(pick(sink, \"fallback\", i MOD 3 = 0))\n\
    total = total + len(collections::get(names, i MOD 8))\n\
    total = total + len(\"plain\")\n\
    IF collections::get(names, i MOD 8) = \"name7\" THEN\n\
      total = total + 1\n\
    END IF\n\
    total = total + len(strings::trim(\"  pad  \"))\n\
    total = total + len(strings::left(collections::get(names, i MOD 8), 2))\n\
    total = total + len(strings::padLeft(toString(i), 12))\n\
    LET bound AS String = strings::upper(sink)\n\
    total = total + len(bound)\n\
    sink = \"S\" & toString(i MOD 7)\n\
    LET churn AS List OF Integer = [i, i + 1, i + 2]\n\
    total = total + collections::get(churn, 2)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"total=\" & toString(total))\n\
  io::print(\"sink=\" & sink)\n\
  io::print(\"names=\" & collections::get(names, 0) & \",\" & collections::get(names, 7))\n\
  io::print(\"lit=\" & lit(2) & \",\" & lit(3))\n\
  io::print(\"pick=\" & pick(\"A\", \"B\", true) & pick(\"A\", \"B\", false))\n\
  io::print(\"pad=[\" & strings::padLeft(\"7\", 4) & \"]\")\n\
END SUB\n";

#[test]
fn every_string_producer_still_produces_the_right_value() {
    let project = common::temp_project("b536_shape_b_behaviour", SHAPE_B_BEHAVIOUR);
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
    let mut total: i64 = 0;
    let mut sink = String::from("seed");
    for i in 0..2000i64 {
        total += i.to_string().len() as i64;
        total += format!("x{i}y").len() as i64;
        total += if i % 2 == 0 { 12 } else { 11 };
        total += sink.len() as i64;
        total += if i % 3 == 0 {
            sink.len() as i64
        } else {
            "fallback".len() as i64
        };
        total += format!("name{}", i % 8).len() as i64;
        total += "plain".len() as i64;
        if i % 8 == 7 {
            total += 1;
        }
        total += "pad".len() as i64;
        total += 2;
        total += 12;
        total += sink.to_uppercase().len() as i64;
        sink = format!("S{}", i % 7);
        total += i + 2;
    }
    let expected = format!(
        "total={total}\nsink={sink}\nnames=name0,name7\nlit=even-literal,odd-literal\npick=AB\npad=[   7]"
    );
    assert_eq!(
        out.trim(),
        expected,
        "a String producer changed its value — a freed block was read back"
    );
    let _ = std::fs::remove_dir_all(&project);
}
