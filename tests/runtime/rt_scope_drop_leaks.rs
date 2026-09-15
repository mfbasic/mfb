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

// -------------------------------------------------------------- shape B-2

/// The bug document's own standalone repro of the CALLEE half: a `String`
/// returned by a user / `.mfb`-bodied function, left unbound as an `append`
/// argument. This is the shape `csv::parse` is built out of —
/// `row = collections::append(row, __csv_fieldValue(...))`.
///
/// It needs BOTH halves of the fix, which is why it is the first case:
///
/// * the callee's `RETURN out` was move-elided, but `out`'s value is a constant
///   `String` so every read of it constant-folds back to rodata — the caller got
///   a READ-ONLY pointer and the arena copy `out` actually owns was orphaned
///   (that orphan IS the pre-fix 64 B per call). `plan_returned_move` now
///   declines, so the return is a real block;
/// * `register_pending_temp` then took the plan-25 bare-`String` early return at
///   the call site, because native provenance (`mark_fresh_string`) cannot see
///   through a call. `function_returns_fresh_string` is the callee's own promise,
///   and it is what lets the `append` argument be freed at statement end.
///
/// Either fix alone is worse than neither: the decline without the caller free
/// just moves the leak, and the caller free without the decline `arena_free`s
/// rodata — observed as an immediate SIGBUS. Measured 400k/800k: 25 → 50 MB
/// before, 0 → 0 MB after.
const SHAPE_B2_APPEND_ARGUMENT: &str = "IMPORT io\n\
IMPORT collections\n\
FUNC mkEmpty(i AS Integer) AS String\n\
  MUT out AS String = \"\"\n\
  RETURN out\n\
END FUNC\n\
SUB main()\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    MUT xs AS List OF String = []\n\
    xs = collections::append(xs, mkEmpty(i))\n\
    acc = acc + len(collections::get(xs, 0))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// The TRANSITIVE form, which is what the decoders actually need: `top` returns
/// `middle`'s result, `middle` returns `leaf`'s, and only `leaf` reaches a native
/// producer. A per-return-site classification that stopped at the first call would
/// leak every level; the guarantee is inductive, so each level's result is a
/// claimed pending temp and no copy is inserted anywhere.
/// Pre-fix 13 MB -> 25 MB at 200k/400k; after: 0 MB -> 0 MB.
const SHAPE_B2_TRANSITIVE: &str = "IMPORT io\n\
FUNC leaf(i AS Integer) AS String\n  RETURN toString(i)\nEND FUNC\n\
FUNC middle(i AS Integer) AS String\n  RETURN leaf(i)\nEND FUNC\n\
FUNC top(i AS Integer) AS String\n  RETURN middle(i)\nEND FUNC\n\
SUB main()\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    acc = acc + len(top(i))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// `RETURN "literal"`. The bug document listed this as the reason a caller could
/// NOT free a user callee's `String` — "a rodata pointer (`arena_free` on it is
/// SIGBUS)". It is not: `static_string_value` classifies the literal as needing an
/// owning copy, so `lower_returned_value` already `copy_flat_block`s it and the
/// callee returns a fresh arena block. The measurement is the proof — pre-fix this
/// leaked 64 B per call (13 MB -> 25 MB at 200k/400k), which a rodata pointer
/// cannot do because nothing would have been allocated.
const SHAPE_B2_RETURNED_LITERAL: &str = "IMPORT io\n\
FUNC lit(i AS Integer) AS String\n\
  IF i MOD 2 = 0 THEN\n    RETURN \"even\"\n  END IF\n  RETURN \"odd\"\n\
END FUNC\n\
SUB main()\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    acc = acc + len(lit(i))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// `toString` of a fresh `String` — a pre-existing USE-AFTER-FREE this change
/// also fixes, because shape B-2 would otherwise widen its reach from native
/// producers to every user callee.
///
/// `toString`'s `String` arm is the IDENTITY: it hands back its argument's block.
/// It also spills the argument and reloads it into a fresh register, so the block
/// leaves under a DIFFERENT operand than it arrived, and that operand is the
/// pending temp's identity token. The owning binding's `claim_pending_temp` no
/// longer matched, so the statement-scope `arena_free` ran anyway and the binding
/// was left holding freed memory. `retarget_pending_temp` moves the token forward
/// instead: one block, one owner.
///
/// On the pre-fix compiler this program does not merely leak — it **SIGSEGVs**
/// (`[exit 139]`, measured at both counts). It is RED as a crash, not as a number.
const SHAPE_B2_TOSTRING_IDENTITY: &str = "IMPORT io\n\
FUNC mk(i AS Integer) AS String\n  RETURN toString(i)\nEND FUNC\n\
SUB main()\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    LET a AS String = toString(\"x\" & toString(i))\n\
    LET b AS String = toString(mk(i))\n\
    LET c AS String = toString(toString(mk(i)))\n\
    acc = acc + len(a) + len(b) + len(c)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// The POSITIVE pin for B-2, and the one that decides the design: a callee whose
/// every value-return is a bare parameter is a plan-86 K1 BORROW of the caller's
/// argument block. Freeing that result frees a `String` the caller still owns, so
/// `function_returns_fresh_string` must keep excluding it. The loop keeps the
/// borrowed source live and re-reads it after every call, and allocates a fresh
/// list each iteration so a corrupted free list faults a later allocation rather
/// than passing silently.
const SHAPE_B2_PARAM_BORROW_CONTRAST: &str = "IMPORT io\n\
IMPORT collections\n\
FUNC pick(a AS String, b AS String, useA AS Boolean) AS String\n\
  IF useA THEN\n    RETURN a\n  END IF\n  RETURN b\n\
END FUNC\n\
SUB main()\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  MUT held AS String = \"held-value\"\n\
  WHILE i < {N}\n\
    acc = acc + len(pick(held, \"fallback\", i MOD 2 = 0))\n\
    acc = acc + len(held)\n\
    LET churn AS List OF Integer = [i, i + 1]\n\
    acc = acc + collections::get(churn, 1)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"held=\" & held)\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

#[cfg(unix)]
#[test]
fn an_unbound_user_string_call_result_runs_at_constant_rss() {
    assert_flat(
        "b536_b2_append_arg",
        SHAPE_B2_APPEND_ARGUMENT,
        400_000,
        800_000,
    );
}

#[cfg(unix)]
#[test]
fn a_transitive_user_string_call_chain_runs_at_constant_rss() {
    assert_flat("b536_b2_transitive", SHAPE_B2_TRANSITIVE, 400_000, 800_000);
}

#[cfg(unix)]
#[test]
fn a_returned_string_literal_runs_at_constant_rss() {
    assert_flat(
        "b536_b2_literal",
        SHAPE_B2_RETURNED_LITERAL,
        400_000,
        800_000,
    );
}

#[cfg(unix)]
#[test]
fn tostring_of_a_fresh_string_runs_at_constant_rss() {
    assert_flat(
        "b536_b2_tostring_identity",
        SHAPE_B2_TOSTRING_IDENTITY,
        400_000,
        800_000,
    );
}

// -------------------------------------------------------------- bug-562

/// bug-562's caller-side half, and the one shape of it that reads as a NUMBER
/// rather than a crash.
///
/// `function_returns_fresh_string` is consulted from both ends — the callee takes
/// on "return a solely-owned block on every path", the call site takes the licence
/// to free the result at statement end. Excluding callback-referenced functions
/// switched BOTH off, so a `String`-returning function that happened to be passed
/// to a HOF *anywhere in the module* lost its callers' statement-scope free too:
/// `acc = acc + len(deco(arg))` leaked 64 B per call, purely because `deco` was
/// also used as a callback. The `transform` call here is what makes `deco`
/// callback-referenced; deleting that ONE line made the same loop flat.
///
/// Measured 400k/800k: 25.6 MB -> 50.2 MB before, 1.0 MB -> 1.0 MB after.
const SHAPE_562_CALLBACK_REFERENCED_DIRECT_CALL: &str = "IMPORT io\n\
IMPORT collections\n\
FUNC deco(s AS String) AS String\n  RETURN s & \">\"\nEND FUNC\n\
SUB main()\n\
  LET seed AS List OF String = [\"a\"]\n\
  LET once AS List OF String = collections::transform(seed, deco)\n\
  LET arg AS String = \"x\"\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    acc = acc + len(deco(arg))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc) & \" once=\" & collections::get(once, 0))\n\
END SUB\n";

/// The POSITIVE pin for it: the identical program with the `transform` line
/// removed, so `deco` is NOT callback-referenced. It was already flat before
/// bug-562 and must stay flat — the fix widens which functions carry the
/// freshness obligation, and a widening that also gave THIS one a second owner
/// would be a double free rather than a leak fix.
const SHAPE_562_PLAIN_DIRECT_CALL_CONTRAST: &str = "IMPORT io\n\
FUNC deco(s AS String) AS String\n  RETURN s & \">\"\nEND FUNC\n\
SUB main()\n\
  LET arg AS String = \"x\"\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    acc = acc + len(deco(arg))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// bug-562 is not confined to the built-in HOFs: the `FunctionRef` ABI is reached
/// by any user function that takes a callable and invokes it, and the callee's
/// obligation is the same there. This is the shape with no `collections` import
/// at all.
///
/// `apply(other, held)` passes `other` as a `FunctionRef`, which is all it takes
/// to make it callback-referenced. `other`'s `RETURN toString(s)` then handed
/// back the caller's own `held` block; `apply` moved it on as its own result, and
/// the statement-scope free at the `&` operand released a block `held` still
/// owned. Pre-fix this printed exactly one line and `[exit 139]`, on every run.
///
/// Two details are load-bearing and were each found by having them wrong:
///
/// * the result must be consumed UNBOUND (as a `&` operand). Bound to a `LET`,
///   the double free lands on two scope-drops in the same iteration and the arena
///   happens to survive it — the program then returns the right answer with the
///   defect intact.
/// * the top-level `g` must NOT itself be passed as a callback anywhere. Adding
///   `apply(g, "z")` makes `g` callback-referenced too, which on the PRE-FIX
///   compiler switched off the very freshness licence that produced the extra
///   free — the bug masked itself.
///
/// The callable parameter is deliberately named `g`, shadowing a top-level `g`
/// that also returns `String`, because the caller-side freshness classification
/// keys off the call TARGET name: an indirect call through a shadowing parameter
/// is the one place that lookup could resolve to the wrong function. The final
/// line pins that the parameter wins (`apply(other, …)` yields `"y"`, not
/// `"TOP"`).
const SHAPE_562_INDIRECT_CALLABLE: &str = "IMPORT io\n\
FUNC g(s AS String) AS String\n  RETURN \"TOP\"\nEND FUNC\n\
FUNC other(s AS String) AS String\n  RETURN toString(s)\nEND FUNC\n\
FUNC apply(g AS FUNC(String) AS String, s AS String) AS String\n  RETURN g(s)\nEND FUNC\n\
SUB main()\n\
  MUT rep AS Integer = 0\n\
  WHILE rep < 200\n\
    LET held AS String = \"held-\" & toString(rep)\n\
    io::print(\"a=\" & apply(other, held))\n\
    io::print(\"held=\" & held)\n\
    rep = rep + 1\n\
  END WHILE\n\
  io::print(\"shadow=\" & g(\"x\") & \",\" & apply(other, \"y\"))\n\
END SUB\n";

/// The VALUES, not the RSS: a callback reached through a user function rather
/// than a built-in HOF must return a block the caller owns, and must leave the
/// caller's argument intact. Every one of the 401 lines is asserted, because the
/// pre-fix failure mode is a use-after-free that reads back empty as often as it
/// faults.
#[test]
fn a_callback_invoked_through_a_user_function_returns_an_owned_block() {
    let project = common::temp_project("b562_indirect_callable", SHAPE_562_INDIRECT_CALLABLE);
    let exe = common::build_project(&project);
    let output = std::process::Command::new(&exe)
        .output()
        .expect("run the indirect-callable probe");
    assert!(
        output.status.success(),
        "{}",
        common::exit_description(&output.status)
    );
    let out = String::from_utf8(output.stdout).expect("utf8 stdout");
    let mut expected = String::new();
    for rep in 0..200 {
        expected.push_str(&format!("a=held-{rep}\nheld=held-{rep}\n"));
    }
    expected.push_str("shadow=TOP,y\n");
    assert_eq!(
        out, expected,
        "the callback's result and the caller's live argument must both read back \
         intact on every iteration, and the callable PARAMETER `g` must shadow the \
         top-level `g` at the indirect call"
    );
    let _ = std::fs::remove_dir_all(&project);
}

#[cfg(unix)]
#[test]
fn a_direct_call_to_a_callback_referenced_string_callee_runs_at_constant_rss() {
    assert_flat(
        "b562_callback_referenced_direct",
        SHAPE_562_CALLBACK_REFERENCED_DIRECT_CALL,
        400_000,
        800_000,
    );
}

#[cfg(unix)]
#[test]
fn a_direct_call_to_a_plain_string_callee_still_runs_at_constant_rss() {
    assert_flat(
        "b562_plain_direct",
        SHAPE_562_PLAIN_DIRECT_CALL_CONTRAST,
        400_000,
        800_000,
    );
}

#[cfg(unix)]
#[test]
fn a_param_borrow_string_callee_still_runs_at_constant_rss() {
    assert_flat(
        "b536_b2_param_borrow",
        SHAPE_B2_PARAM_BORROW_CONTRAST,
        400_000,
        800_000,
    );
}

/// The B-2 behaviour pin: every shape a `String`-returning user function can
/// take, each consumed in every position a `String` call result can appear in
/// (bare bind, `&` operand, `append` argument, another call's argument, `RETURN`
/// operand, comparison operand, reassignment source), run in a churning loop with
/// exact expected values.
///
/// It is the counterpart to the RSS cases above, and the more important half: the
/// fix ADDS `arena_free`s at every user-`String`-call site, so its failure mode is
/// a wild free — a value read back wrong, or a corrupted free list that faults a
/// later allocation. Every shape the predicate must NOT admit is in here on
/// purpose: a param-borrow callee (`pick`), the `toString` identity (`ident`), a
/// rodata literal return (`litOnly`), a `collections::get` borrow re-returned
/// (`viaGet`), and a fallible callee reached through `TRAP` (`failable`).
///
/// Byte-identical to the pre-fix compiler's output on this program, which is what
/// makes it a pin on the fix rather than on the bug.
const SHAPE_B2_BEHAVIOUR: &str = "IMPORT io\n\
IMPORT collections\n\
IMPORT strings\n\
FUNC mk(i AS Integer) AS String\n\
  MUT out AS String = \"v\"\n\
  RETURN out & toString(i)\n\
END FUNC\n\
FUNC pick(a AS String, b AS String, useA AS Boolean) AS String\n\
  IF useA THEN\n    RETURN a\n  END IF\n  RETURN b\n\
END FUNC\n\
FUNC ident(s AS String) AS String\n  RETURN toString(s)\nEND FUNC\n\
FUNC litOnly(i AS Integer) AS String\n\
  IF i MOD 2 = 0 THEN\n    RETURN \"even\"\n  END IF\n  RETURN \"odd\"\n\
END FUNC\n\
FUNC viaGet(i AS Integer) AS String\n\
  LET xs AS List OF String = [\"g0\", \"g1\", \"g2\"]\n\
  RETURN collections::get(xs, i MOD 3)\n\
END FUNC\n\
FUNC recurse(i AS Integer) AS String\n\
  IF i <= 0 THEN\n    RETURN \"z\"\n  END IF\n  RETURN \"r\" & recurse(i - 1)\n\
END FUNC\n\
FUNC failable(i AS Integer) AS String\n\
  IF i < 0 THEN\n    FAIL error(77050003, \"neg\")\n  END IF\n  RETURN \"ok\" & toString(i)\n\
END FUNC\n\
SUB main()\n\
  MUT i AS Integer = 0\n\
  MUT total AS Integer = 0\n\
  MUT names AS List OF String = []\n\
  MUT sink AS String = \"seed\"\n\
  WHILE i < 2000\n\
    total = total + len(toString(mk(i)))\n\
    total = total + len(toString(toString(mk(i))))\n\
    total = total + len(ident(mk(i)))\n\
    total = total + len(strings::upper(mk(i)))\n\
    total = total + len(mk(i) & \"-\" & mk(i))\n\
    total = total + len(pick(mk(i), \"fallback\", i MOD 2 = 0))\n\
    total = total + len(litOnly(i))\n\
    total = total + len(viaGet(i))\n\
    total = total + len(recurse(3))\n\
    names = [ ]\n\
    names = collections::append(names, mk(i))\n\
    names = collections::append(names, ident(mk(i)))\n\
    total = total + len(collections::get(names, 0) & collections::get(names, 1))\n\
    IF mk(i) = \"v3\" THEN\n      total = total + 1\n    END IF\n\
    sink = ident(mk(i))\n\
    total = total + len(sink)\n\
    LET fv AS String = failable(i) TRAP\n      total = total - 1\n      EXIT SUB\n    END TRAP\n\
    total = total + len(fv)\n\
    LET churn AS List OF Integer = [i, i + 1, i + 2]\n\
    total = total + collections::get(churn, 2)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"total=\" & toString(total))\n\
  io::print(\"sink=\" & sink)\n\
  io::print(\"mk=\" & mk(7) & \",\" & toString(mk(7)) & \",\" & ident(mk(7)))\n\
  io::print(\"pick=\" & pick(mk(7), \"fb\", TRUE) & pick(mk(7), \"fb\", FALSE))\n\
  io::print(\"lit=\" & litOnly(2) & litOnly(3) & \",\" & viaGet(1) & \",\" & recurse(2))\n\
END SUB\n";

#[test]
fn every_string_return_shape_still_produces_the_right_value() {
    let project = common::temp_project("b536_b2_behaviour", SHAPE_B2_BEHAVIOUR);
    let exe = common::build_project(&project);
    let output = std::process::Command::new(&exe)
        .output()
        .expect("run the B-2 behaviour probe");
    assert!(
        output.status.success(),
        "{}",
        common::exit_description(&output.status)
    );
    let out = String::from_utf8(output.stdout).expect("utf8 stdout");
    let mut total: i64 = 0;
    let mut sink = String::new();
    for i in 0..2000i64 {
        let mk = format!("v{i}");
        let lit = if i % 2 == 0 { "even" } else { "odd" };
        total += mk.len() as i64; // toString(mk(i))
        total += mk.len() as i64; // toString(toString(mk(i)))
        total += mk.len() as i64; // ident(mk(i))
        total += mk.len() as i64; // strings::upper(mk(i)) — same length
        total += (mk.len() * 2 + 1) as i64; // mk & "-" & mk
        total += if i % 2 == 0 {
            mk.len() as i64
        } else {
            "fallback".len() as i64
        };
        total += lit.len() as i64;
        total += 2; // viaGet — "g0"/"g1"/"g2"
        total += 4; // recurse(3) — "rrrz"
        total += (mk.len() * 2) as i64; // names[0] & names[1]
        if mk == "v3" {
            total += 1;
        }
        sink = mk.clone();
        total += sink.len() as i64;
        total += format!("ok{i}").len() as i64;
        total += i + 2;
    }
    let expected =
        format!("total={total}\nsink={sink}\nmk=v7,v7,v7\npick=v7fb\nlit=evenodd,g1,rrz");
    assert_eq!(
        out.trim(),
        expected,
        "a String return shape changed its value — a freed block was read back"
    );
    let _ = std::fs::remove_dir_all(&project);
}

// ---------------------------------------------------------------- bug-560

/// bug-560: `out = out & <expr>` on a `MUT String` leaked ~190 B per evaluation
/// once the binding was also *reassigned* in the loop. The in-place self-append
/// grows the block with geometric capacity headroom recorded only in a frame
/// shadow slot, and the reassignment's drop
/// (`_mfb_rt_drop_owned_string`) sized the free from the `byteLength` header
/// alone — freeing `len + 9` of a `len + spare + 9` block and orphaning `spare`
/// on every iteration. Measured on the pre-fix compiler: 38 MB at 200k, 75 MB at
/// 400k.
const B560_REASSIGNED_SELF_APPEND: &str = "IMPORT io\n\
SUB main()\n\
  MUT out AS String = \"\"\n\
  MUT acc AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    out = \"\"\n\
    out = out & \"a\"\n\
    acc = acc + len(out)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// The same defect through the RETURN seam, and the shape that owns the size of
/// the bug: `plan_returned_move` moved the headroom-carrying block to the caller,
/// where nothing knows the shadow, so the caller under-freed by `spare` on every
/// call. This is `__encoding_utf32Decode` / `__csv_decodeRange` exactly — build a
/// String with `out = out & …` in a loop and return it. Measured on the pre-fix
/// compiler: 63 MB at 2 000 calls, 126 MB at 4 000 (a 9 000-byte string in a
/// 16 384-byte buffer, so ~7 KB orphaned per call).
const B560_RETURNED_SELF_APPEND: &str = "IMPORT io\n\
FUNC build(n AS Integer) AS String\n\
  MUT out AS String = \"\"\n\
  MUT k AS Integer = 0\n\
  WHILE k < n\n\
    out = out & \"abc\"\n\
    k = k + 1\n\
  END WHILE\n\
  RETURN out\n\
END FUNC\n\
SUB main()\n\
  MUT acc AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET s AS String = build(3000)\n\
    acc = acc + len(s)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// The same defect reached through the shipped `.mfb` decoder rather than a
/// hand-written loop: `encoding::utf32Decode` is `out = out & fromCodepoint(cp)`
/// once per scalar and returns `out`, so it inherited the RETURN-seam leak
/// without a single line of user code being at fault.
const B560_DECODER: &str = "IMPORT io\n\
IMPORT collections\n\
IMPORT encoding\n\
SUB main()\n\
  MUT units AS List OF Integer = []\n\
  MUT k AS Integer = 0\n\
  WHILE k < 400\n\
    units = collections::append(units, 65 + (k MOD 26))\n\
    k = k + 1\n\
  END WHILE\n\
  MUT acc AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    acc = acc + len(encoding::utf32Decode(units))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// The bug report's contrast line, kept as a POSITIVE pin: a self-append with no
/// intervening reassignment and no return was ALREADY flat — bug-77 gave the
/// regrow its own exactly-sized free — and must stay flat. It is the case that
/// would go RED if the fix's extra `spare` were ever double-counted (the regrow
/// frees the old block itself; the capacity-aware drop must not also fire on it).
const B560_CONTRAST_PLAIN: &str = "IMPORT io\n\
SUB main()\n\
  MUT out AS String = \"\"\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    out = out & \"a\"\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"len=\" & toString(len(out)))\n\
END SUB\n";

#[cfg(unix)]
#[test]
fn a_reassigned_string_self_append_runs_at_constant_rss() {
    assert_flat(
        "b560_reassigned",
        B560_REASSIGNED_SELF_APPEND,
        200_000,
        400_000,
    );
}

#[cfg(unix)]
#[test]
fn a_returned_string_self_append_runs_at_constant_rss() {
    assert_flat("b560_returned", B560_RETURNED_SELF_APPEND, 2_000, 4_000);
}

#[cfg(unix)]
#[test]
fn the_utf32_decoder_runs_at_constant_rss() {
    assert_flat("b560_decoder", B560_DECODER, 20_000, 40_000);
}

#[cfg(unix)]
#[test]
fn a_plain_string_self_append_still_runs_at_constant_rss() {
    assert_flat("b560_plain", B560_CONTRAST_PLAIN, 200_000, 400_000);
}

/// The positive behaviour pin for bug-560, and the one that matters: the fix
/// makes an existing `arena_free` free MORE bytes, so its failure mode is an
/// OVER-free — arena free-list corruption, which surfaces as a fault or a wrong
/// value at some later allocation, never as a red assertion. Every shape that
/// reaches the capacity-aware drop is exercised here in a churning loop with
/// unrelated allocations interleaved, so a corrupted free list is very likely to
/// be handed back to a later `String`/`List` and read wrong:
///
/// * a self-append target reassigned from a literal (the shadow describes the
///   OLD block at the drop, and the new block is tight);
/// * a self-append target reassigned from a call result;
/// * a self-append target that is only ever appended (bug-77's exact-size regrow
///   free must remain the only free on that path);
/// * a self-append target RETURNED — now copied rather than moved, so the caller
///   must still see exactly the built bytes;
/// * a conditional append, so a drop can be reached on a path where the shadow
///   is 0 and on one where it is not;
/// * a self-append inside a nested loop, appended after the inner loop too;
/// * the shipped `encoding::utf32Decode`, which is this shape in `.mfb`.
const B560_BEHAVIOUR: &str = "IMPORT io\n\
IMPORT collections\n\
IMPORT encoding\n\
FUNC build(n AS Integer) AS String\n\
  MUT out AS String = \"\"\n\
  MUT k AS Integer = 0\n\
  WHILE k < n\n\
    out = out & \"ab\"\n\
    k = k + 1\n\
  END WHILE\n\
  RETURN out\n\
END FUNC\n\
FUNC nested(n AS Integer) AS String\n\
  MUT out AS String = \"<\"\n\
  MUT k AS Integer = 0\n\
  WHILE k < n\n\
    MUT inner AS String = \"\"\n\
    MUT j AS Integer = 0\n\
    WHILE j < 3\n\
      inner = inner & toString(j)\n\
      j = j + 1\n\
    END WHILE\n\
    out = out & inner\n\
    k = k + 1\n\
  END WHILE\n\
  out = out & \">\"\n\
  RETURN out\n\
END FUNC\n\
SUB main()\n\
  MUT total AS Integer = 0\n\
  MUT reset AS String = \"\"\n\
  MUT dyn AS String = \"\"\n\
  MUT grow AS String = \"\"\n\
  MUT cond AS String = \"\"\n\
  MUT units AS List OF Integer = []\n\
  MUT k AS Integer = 0\n\
  WHILE k < 12\n\
    units = collections::append(units, 97 + k)\n\
    k = k + 1\n\
  END WHILE\n\
  MUT last AS String = \"\"\n\
  MUT i AS Integer = 0\n\
  WHILE i < 400\n\
    reset = \"\"\n\
    reset = reset & \"a\" & toString(i MOD 10)\n\
    total = total + len(reset)\n\
    dyn = toString(i MOD 100)\n\
    dyn = dyn & \"-\"\n\
    total = total + len(dyn)\n\
    grow = grow & \"g\"\n\
    total = total + len(grow)\n\
    IF i MOD 3 = 0 THEN\n\
      cond = \"\"\n\
      cond = cond & \"c\"\n\
    END IF\n\
    total = total + len(cond)\n\
    LET b AS String = build(i MOD 40)\n\
    total = total + len(b)\n\
    LET d AS String = encoding::utf32Decode(units)\n\
    total = total + len(d)\n\
    LET churn AS List OF Integer = [i, i + 1, i + 2]\n\
    total = total + collections::get(churn, 2)\n\
    last = nested(i MOD 5)\n\
    total = total + len(last)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"total=\" & toString(total))\n\
  io::print(\"grow=\" & toString(len(grow)))\n\
  io::print(\"last=\" & last)\n\
  io::print(\"build=\" & build(4))\n\
  io::print(\"nested=\" & nested(2))\n\
  io::print(\"decode=\" & encoding::utf32Decode(units))\n\
END SUB\n";

#[test]
fn every_string_self_append_shape_still_produces_the_right_value() {
    let project = common::temp_project("b560_behaviour", B560_BEHAVIOUR);
    let exe = common::build_project(&project);
    let output = std::process::Command::new(&exe)
        .output()
        .expect("run the bug-560 behaviour probe");
    assert!(
        output.status.success(),
        "{}",
        common::exit_description(&output.status)
    );
    let out = String::from_utf8(output.stdout).expect("utf8 stdout");
    let decoded: String = (0..12u32)
        .map(|k| char::from_u32(97 + k).expect("ascii"))
        .collect();
    let nested = |n: i64| {
        let mut s = String::from("<");
        for _ in 0..n {
            s.push_str("012");
        }
        s.push('>');
        s
    };
    let mut total: i64 = 0;
    let mut grow = 0i64;
    let mut cond = 0i64;
    let mut last = String::new();
    for i in 0..400i64 {
        total += format!("a{}", i % 10).len() as i64;
        total += format!("{}-", i % 100).len() as i64;
        grow += 1;
        total += grow;
        if i % 3 == 0 {
            cond = 1;
        }
        total += cond;
        total += (i % 40) * 2;
        total += decoded.len() as i64;
        total += i + 2;
        last = nested(i % 5);
        total += last.len() as i64;
    }
    let expected = format!(
        "total={total}\ngrow={grow}\nlast={last}\nbuild=abababab\nnested={}\ndecode={decoded}",
        nested(2)
    );
    assert_eq!(
        out.trim(),
        expected,
        "a String self-append shape changed its value — a block was freed twice or too far"
    );
    let _ = std::fs::remove_dir_all(&project);
}

// ---------------------------------------------------------------- bug-561

/// bug-561: `LET s AS String = fallible(i) TRAP … END TRAP` leaked the block the
/// CALLEE returned. The inline-`TRAP` desugar binds
/// `$trap_resN : Result OF T = CallResult(f(..))`, and the lowering builds that
/// `Result` by COPYING the callee's block into a fresh `{tag, size, payload}`
/// block — after which the callee's own block has no owner at all. Measured on
/// the pre-fix compiler: 13.3 MB at 200k, 25.6 MB at 400k.
///
/// The callee deliberately returns `toString(...)` and not a `&` concat: a user
/// function that returns a concatenation leaks 64 B per call *without any*
/// `TRAP`, which is a separate defect (bug-567) and would mask this one.
const B561_TRAP_STRING: &str = "IMPORT io\n\
FUNC fname(n AS Integer) AS String\n\
  IF n < 0 THEN\n    FAIL error(7, \"negative\")\n  END IF\n\
  RETURN toString(n MOD 10)\n\
END FUNC\n\
SUB main()\n\
  MUT acc AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET s AS String = fname(i) TRAP(e)\n\
      RECOVER \"x\"\n\
    END TRAP\n\
    acc = acc + len(s)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// The same defect with a collection payload — 256 B per call, 50.6 MB at 200k
/// and 100.2 MB at 400k. It is in its own case because the leak is the PAYLOAD,
/// not a fixed-size wrapper: the bug report's "type-independent, it is the
/// `Result` wrapper" reading is wrong, and `Result OF Integer` (a scalar payload,
/// stored inline in the `Result`'s own block) never leaked at all.
const B561_TRAP_LIST: &str = "IMPORT io\n\
IMPORT collections\n\
FUNC trio(n AS Integer) AS List OF Integer\n\
  IF n < 0 THEN\n    FAIL error(7, \"negative\")\n  END IF\n\
  RETURN [n, n + 1, n + 2]\n\
END FUNC\n\
SUB main()\n\
  MUT acc AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET xs AS List OF Integer = trio(i) TRAP(e)\n\
      RECOVER []\n\
    END TRAP\n\
    acc = acc + collections::get(xs, 2)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// The second lowering path with the same defect: an inline builtin under a
/// `TRAP` (`lower_inline_builtin_raw`) runs the member's ordinary lowering
/// DIRECTLY, bypassing `lower_value` — the only place `register_pending_temp` is
/// called — so `strings::mid`'s and `collections::get`'s fresh blocks were
/// copied into the `Result` and abandoned. 13.3 MB at 200k, 25.6 MB at 400k
/// each. Outside a `TRAP` the identical calls are flat, which is the contrast
/// that identifies the bypass.
const B561_TRAP_INLINE_BUILTIN: &str = "IMPORT io\n\
IMPORT collections\n\
IMPORT strings\n\
SUB main()\n\
  LET base AS String = \"abcdefghijklmnop\"\n\
  LET xs AS List OF String = [\"aa\", \"bb\", \"cc\"]\n\
  MUT acc AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET a AS String = strings::mid(base, i MOD 4, 5) TRAP(e)\n\
      RECOVER \"x\"\n\
    END TRAP\n\
    LET b AS String = collections::get(xs, i MOD 3) TRAP(e)\n\
      RECOVER \"y\"\n\
    END TRAP\n\
    acc = acc + len(a) + len(b)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// The ERROR path, which is the one the fix could get catastrophically wrong:
/// the payload temp's slot is written on the Ok branch ONLY, because on the
/// error branch the raw success register holds an error code and not a block.
/// Every call here fails, so the slot is never written and the statement-end
/// drop must see the prologue zero (or the previous iteration's free-and-null)
/// and skip. A fix that spilled before the branch would free an error code —
/// a wild `arena_free` — rather than leak, so this is asserted on the VALUE and
/// the exit status.
///
/// It is deliberately NOT an RSS case: the inline-`TRAP` error path leaks
/// ~780 B per trapped error on its own (149 MB at 200k, 298 MB at 400k), a
/// SEPARATE pre-existing defect this change does not touch — measured
/// byte-identical on the base compiler and after (bug-565).
const B561_TRAP_ALWAYS_FAILS: &str = "IMPORT io\n\
FUNC always(n AS Integer) AS String\n\
  IF n >= 0 THEN\n    FAIL error(7, \"always\")\n  END IF\n\
  RETURN toString(n)\n\
END FUNC\n\
SUB main()\n\
  MUT acc AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET s AS String = always(i) TRAP(e)\n\
      RECOVER \"fallback\"\n\
    END TRAP\n\
    acc = acc + len(s)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// The bug report's own claim, kept as a POSITIVE pin: it recorded
/// `Result OF Integer` as the WORST case at 128 B per call. It never leaked —
/// a scalar payload lives in the `Result`'s own block, which was always freed —
/// and it must stay flat, because the fix adds a free next to it.
const B561_CONTRAST_SCALAR: &str = "IMPORT io\n\
FUNC half(n AS Integer) AS Integer\n\
  IF n < 0 THEN\n    FAIL error(7, \"negative\")\n  END IF\n\
  RETURN n / 2\n\
END FUNC\n\
SUB main()\n\
  MUT acc AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET n AS Integer = half(i) TRAP(e)\n\
      RECOVER 0\n\
    END TRAP\n\
    acc = acc + n\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// The other POSITIVE pin: the same producers with NO `TRAP` were already flat
/// (their block is bound and freed by the binding's own scope drop). The fix
/// must not give them a second free.
const B561_CONTRAST_NO_TRAP: &str = "IMPORT io\n\
IMPORT collections\n\
IMPORT strings\n\
FUNC fname(n AS Integer) AS String\n  RETURN toString(n MOD 10)\nEND FUNC\n\
FUNC trio(n AS Integer) AS List OF Integer\n  RETURN [n, n + 1, n + 2]\nEND FUNC\n\
SUB main()\n\
  LET base AS String = \"abcdefghijklmnop\"\n\
  LET xs AS List OF String = [\"aa\", \"bb\", \"cc\"]\n\
  MUT acc AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET s AS String = fname(i)\n\
    LET ys AS List OF Integer = trio(i)\n\
    LET a AS String = strings::mid(base, i MOD 4, 5)\n\
    LET b AS String = collections::get(xs, i MOD 3)\n\
    acc = acc + len(s) + collections::get(ys, 2) + len(a) + len(b)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

#[cfg(unix)]
#[test]
fn a_trap_bound_string_result_runs_at_constant_rss() {
    assert_flat("b561_trap_string", B561_TRAP_STRING, 200_000, 400_000);
}

#[cfg(unix)]
#[test]
fn a_trap_bound_collection_result_runs_at_constant_rss() {
    assert_flat("b561_trap_list", B561_TRAP_LIST, 200_000, 400_000);
}

#[cfg(unix)]
#[test]
fn a_trap_bound_inline_builtin_result_runs_at_constant_rss() {
    assert_flat(
        "b561_trap_builtin",
        B561_TRAP_INLINE_BUILTIN,
        200_000,
        400_000,
    );
}

#[test]
fn a_trap_whose_call_always_fails_still_produces_the_right_value() {
    let program = B561_TRAP_ALWAYS_FAILS.replace("{N}", "20000");
    let project = common::temp_project("b561_trap_fails", &program);
    let exe = common::build_project(&project);
    let output = std::process::Command::new(&exe)
        .output()
        .expect("run the always-failing TRAP probe");
    assert!(
        output.status.success(),
        "the always-failing TRAP faulted: {}",
        common::exit_description(&output.status)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        format!("acc={}", 20_000 * "fallback".len()),
        "the error path's recovery value changed — the Ok-path payload free ran \
         on a branch where the success register holds an error code"
    );
    let _ = std::fs::remove_dir_all(&project);
}

#[cfg(unix)]
#[test]
fn a_trap_bound_scalar_result_still_runs_at_constant_rss() {
    assert_flat("b561_trap_scalar", B561_CONTRAST_SCALAR, 200_000, 400_000);
}

#[cfg(unix)]
#[test]
fn the_same_producers_without_a_trap_still_run_at_constant_rss() {
    assert_flat("b561_no_trap", B561_CONTRAST_NO_TRAP, 200_000, 400_000);
}

/// The positive behaviour pin for bug-561. The fix ADDS an `arena_free` on the
/// success path of every inline `TRAP`, so its failure mode is a double free or
/// a use-after-free — a wrong value or a fault at some later allocation, never a
/// red assertion. Every shape whose block the guard must NOT free is exercised
/// here alongside the ones it must:
///
/// * a fallible user callee returning a fresh `String`, and one returning a
///   `List` and a record;
/// * the ERROR path of each, taken on a third of the iterations, so the
///   conditionally-written temp slot is re-reached unwritten;
/// * a param-borrow callee (`pick`) — its result is the caller's OWN argument
///   block, so freeing it would be a use-after-free;
/// * a callee that returns a rodata literal;
/// * the `toString` identity, whose `String` arm returns its own argument;
/// * inline builtins under `TRAP` (`strings::mid`, `collections::get`) whose
///   containers must survive the read;
/// * a nested `TRAP` inside a `TRAP` handler;
/// * a scalar payload, which has no block at all;
/// * a fresh list every iteration, so a corrupted free list surfaces as a
///   later fault or a wrong element.
const B561_BEHAVIOUR: &str = "IMPORT io\n\
IMPORT collections\n\
IMPORT strings\n\
FUNC fstr(n AS Integer) AS String\n\
  IF n MOD 3 = 0 THEN\n    FAIL error(7, \"three\")\n  END IF\n\
  RETURN toString(n MOD 10)\n\
END FUNC\n\
FUNC flist(n AS Integer) AS List OF Integer\n\
  IF n MOD 3 = 1 THEN\n    FAIL error(8, \"one\")\n  END IF\n\
  RETURN [n, n + 1, n + 2]\n\
END FUNC\n\
FUNC pick(a AS String, b AS String, useA AS Boolean) AS String\n\
  IF useA THEN\n    RETURN a\n  END IF\n  RETURN b\n\
END FUNC\n\
FUNC fpick(a AS String, n AS Integer) AS String\n\
  IF n MOD 5 = 0 THEN\n    FAIL error(9, \"five\")\n  END IF\n\
  RETURN pick(a, \"fb\", n MOD 2 = 0)\n\
END FUNC\n\
FUNC flit(n AS Integer) AS String\n\
  IF n MOD 7 = 0 THEN\n    FAIL error(10, \"seven\")\n  END IF\n\
  IF n MOD 2 = 0 THEN\n    RETURN \"even\"\n  END IF\n  RETURN \"odd\"\n\
END FUNC\n\
FUNC fident(s AS String, n AS Integer) AS String\n\
  IF n MOD 11 = 0 THEN\n    FAIL error(11, \"eleven\")\n  END IF\n\
  RETURN toString(s)\n\
END FUNC\n\
FUNC fnum(n AS Integer) AS Integer\n\
  IF n MOD 3 = 2 THEN\n    FAIL error(12, \"two\")\n  END IF\n\
  RETURN n MOD 100\n\
END FUNC\n\
SUB main()\n\
  LET base AS String = \"abcdefghijklmnop\"\n\
  LET names AS List OF String = [\"aa\", \"bb\", \"cc\"]\n\
  MUT total AS Integer = 0\n\
  MUT sink AS String = \"seed\"\n\
  MUT i AS Integer = 0\n\
  WHILE i < 3000\n\
    LET s AS String = fstr(i) TRAP(e)\n      RECOVER \"S\"\n    END TRAP\n\
    total = total + len(s)\n\
    LET xs AS List OF Integer = flist(i) TRAP(e)\n      RECOVER [9]\n    END TRAP\n\
    total = total + collections::get(xs, 0)\n\
    LET p AS String = fpick(sink, i) TRAP(e)\n      RECOVER \"P\"\n    END TRAP\n\
    total = total + len(p)\n\
    LET l AS String = flit(i) TRAP(e)\n      RECOVER \"L\"\n    END TRAP\n\
    total = total + len(l)\n\
    LET d AS String = fident(sink, i) TRAP(e)\n      RECOVER \"D\"\n    END TRAP\n\
    total = total + len(d)\n\
    LET n AS Integer = fnum(i) TRAP(e)\n      RECOVER -1\n    END TRAP\n\
    total = total + n\n\
    LET m AS String = strings::mid(base, i MOD 4, 5) TRAP(e)\n      RECOVER \"M\"\n    END TRAP\n\
    total = total + len(m)\n\
    LET g AS String = collections::get(names, i MOD 3) TRAP(e)\n      RECOVER \"G\"\n    END TRAP\n\
    total = total + len(g)\n\
    LET nest AS String = fstr(i) TRAP(e)\n\
      LET inner AS String = flit(i) TRAP(e2)\n        RECOVER \"NI\"\n      END TRAP\n\
      RECOVER \"N\" & inner\n\
    END TRAP\n\
    total = total + len(nest)\n\
    LET churn AS List OF Integer = [i, i + 1, i + 2]\n\
    total = total + collections::get(churn, 2)\n\
    sink = \"s\" & toString(i MOD 13)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"total=\" & toString(total))\n\
  io::print(\"sink=\" & sink)\n\
  io::print(\"names=\" & collections::get(names, 0) & collections::get(names, 2))\n\
  io::print(\"base=\" & base)\n\
END SUB\n";

#[test]
fn every_trap_bound_result_still_produces_the_right_value() {
    let project = common::temp_project("b561_behaviour", B561_BEHAVIOUR);
    let exe = common::build_project(&project);
    let output = std::process::Command::new(&exe)
        .output()
        .expect("run the bug-561 behaviour probe");
    assert!(
        output.status.success(),
        "{}",
        common::exit_description(&output.status)
    );
    let out = String::from_utf8(output.stdout).expect("utf8 stdout");
    let mut total: i64 = 0;
    let mut sink = String::from("seed");
    for i in 0..3000i64 {
        // fstr
        let s = if i % 3 == 0 {
            "S".to_string()
        } else {
            (i % 10).to_string()
        };
        total += s.len() as i64;
        // flist
        total += if i % 3 == 1 { 9 } else { i };
        // fpick — reads `sink` as it stood at the top of this iteration
        let p = if i % 5 == 0 {
            "P".to_string()
        } else if i % 2 == 0 {
            sink.clone()
        } else {
            "fb".to_string()
        };
        total += p.len() as i64;
        // flit
        let l = if i % 7 == 0 {
            "L"
        } else if i % 2 == 0 {
            "even"
        } else {
            "odd"
        };
        total += l.len() as i64;
        // fident
        let d = if i % 11 == 0 {
            "D".to_string()
        } else {
            sink.clone()
        };
        total += d.len() as i64;
        // fnum
        total += if i % 3 == 2 { -1 } else { i % 100 };
        // strings::mid — always in range
        total += 5;
        // collections::get
        total += 2;
        // nested TRAP
        let nest = if i % 3 == 0 {
            // the INNER trap's own recovery text, not the outer one's
            let inner = if i % 7 == 0 {
                "NI"
            } else if i % 2 == 0 {
                "even"
            } else {
                "odd"
            };
            format!("N{inner}")
        } else {
            (i % 10).to_string()
        };
        total += nest.len() as i64;
        total += i + 2;
        sink = format!("s{}", i % 13);
    }
    let expected = format!("total={total}\nsink={sink}\nnames=aacc\nbase=abcdefghijklmnop");
    assert_eq!(
        out.trim(),
        expected,
        "a TRAP-bound value changed — a block was freed twice or while still live"
    );
    let _ = std::fs::remove_dir_all(&project);
}

// -------------------------------------------------------------- bug-569

/// bug-569, per HOF. Each of these is the bug report's own reproduction: a
/// `String`-returning callback whose block the HOF collected and never freed —
/// 64 B per element per call, on programs that were otherwise entirely correct.
///
/// The callback is `RETURN toString(s)` rather than a concat on purpose: a concat
/// return leaks a SECOND block (bug-567, untouched here), so it would read as a
/// leak at both counts and say nothing about this one. The identity leaks exactly
/// the block this bug is about.
const SHAPE_569_TRANSFORM: &str = "IMPORT io\n\
IMPORT collections\n\
FUNC pick(s AS String) AS String\n  RETURN toString(s)\nEND FUNC\n\
SUB main()\n\
  LET xs AS List OF String = [\"n0\", \"n1\", \"n2\", \"n3\", \"n4\", \"n5\", \"n6\", \"n7\"]\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    LET c AS List OF String = collections::transform(xs, pick)\n\
    acc = acc + len(collections::get(c, 0))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// `sortBy`'s key projection. String items + a String key declines the native
/// fast path, so this is the `.mfb` body's route to the callback.
const SHAPE_569_SORT_BY: &str = "IMPORT io\n\
IMPORT collections\n\
FUNC key(s AS String) AS String\n  RETURN toString(s)\nEND FUNC\n\
SUB main()\n\
  LET xs AS List OF String = [\"n3\", \"n1\", \"n7\", \"n0\", \"n5\", \"n2\", \"n6\", \"n4\"]\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    LET c AS List OF String = collections::sortBy(xs, key)\n\
    acc = acc + len(collections::get(c, 0))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// `groupBy`'s value projection, on the native fast path (Integer key).
const SHAPE_569_GROUP_BY: &str = "IMPORT io\n\
IMPORT collections\n\
FUNC kf(s AS String) AS Integer\n  RETURN len(s)\nEND FUNC\n\
FUNC vf(s AS String) AS String\n  RETURN toString(s)\nEND FUNC\n\
SUB main()\n\
  LET xs AS List OF String = [\"n0\", \"n1\", \"n2\", \"n3\", \"n4\", \"n5\", \"n6\", \"n7\"]\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    LET c AS Map OF Integer TO List OF String = collections::groupBy(xs, kf, vf)\n\
    acc = acc + len(collections::get(c, 2))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// `mapValues`, the one HOF that invokes its callback directly rather than
/// through `transform`. The source map is `Map OF Integer TO Integer` on purpose:
/// a `String` key or value would materialise its own per-entry block in the
/// `FOR EACH`, and that is a DIFFERENT leak (a bare `FOR EACH` over a
/// `Map OF String TO String` reading `e.key`/`e.value` grows 50 -> 99 MB at these
/// counts with no callback in the program at all). With fixed-width keys and
/// values, the callback's result is the only block either loop can own.
const SHAPE_569_MAP_VALUES: &str = "IMPORT io\n\
IMPORT collections\n\
FUNC deco(v AS Integer) AS String\n  RETURN toString(v)\nEND FUNC\n\
SUB main()\n\
  MUT xs AS Map OF Integer TO Integer = Map OF Integer TO Integer {}\n\
  MUT j AS Integer = 0\n\
  WHILE j < 8\n\
    xs = collections::set(xs, j, j)\n\
    j = j + 1\n\
  END WHILE\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    LET c AS Map OF Integer TO String = collections::mapValues(xs, deco)\n\
    acc = acc + len(collections::get(c, 1))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// The POSITIVE pin, and the one shape where the HOF must NOT free: a callback
/// that returns its own bare parameter. The block it hands back is the very one
/// `free_collection_loop_item` released on the way in, so a second free is a
/// double free — which the arena reports as "Allocation failed" at some later,
/// unrelated allocation, or as a wrong value read back from reused memory, not as
/// a crash at the site.
///
/// It is measured as RSS *and* as a value (`acc` counts the characters actually
/// read back out of the result list), because the two failure directions are
/// invisible to each other: a missing free shows only in the RSS, a double free
/// only in the bytes.
const SHAPE_569_BARE_PARAM_CALLBACK: &str = "IMPORT io\n\
IMPORT collections\n\
FUNC pick(s AS String) AS String\n  RETURN s\nEND FUNC\n\
SUB main()\n\
  LET xs AS List OF String = [\"n0\", \"n1\", \"n2\", \"n3\", \"n4\", \"n5\", \"n6\", \"n7\"]\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    LET c AS List OF String = collections::transform(xs, pick)\n\
    acc = acc + len(collections::get(c, 0)) + len(collections::get(c, 7))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc) & \" src=\" & collections::get(xs, 0))\n\
END SUB\n";

/// A fixed-width callback over the same list. It never allocated and was always
/// flat, and it must stay flat: a fix that freed something here would be freeing
/// a scalar.
const SHAPE_569_FIXED_WIDTH_CONTRAST: &str = "IMPORT io\n\
IMPORT collections\n\
FUNC pick(s AS String) AS Integer\n  RETURN len(s)\nEND FUNC\n\
SUB main()\n\
  LET xs AS List OF String = [\"n0\", \"n1\", \"n2\", \"n3\", \"n4\", \"n5\", \"n6\", \"n7\"]\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    LET c AS List OF Integer = collections::transform(xs, pick)\n\
    acc = acc + collections::get(c, 0)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// 56.8 MB -> 112.6 MB at 50k/100k passes over an 8-element list, before.
#[cfg(unix)]
#[test]
fn transform_runs_at_constant_rss_with_a_string_callback() {
    assert_flat("b569_transform", SHAPE_569_TRANSFORM, 50_000, 100_000);
}

/// Each HOF frees on its OWN path, so a pin on `transform` says nothing about
/// this one even though both reach the callback through the same lowering today.
#[cfg(unix)]
#[test]
fn sort_by_runs_at_constant_rss_with_a_string_key() {
    assert_flat("b569_sort_by", SHAPE_569_SORT_BY, 50_000, 100_000);
}

#[cfg(unix)]
#[test]
fn group_by_runs_at_constant_rss_with_a_string_value() {
    assert_flat("b569_group_by", SHAPE_569_GROUP_BY, 50_000, 100_000);
}

/// 33 MB -> 66 MB before. `mapValues` reaches its callback by an entirely
/// different route (a direct indirect invocation in its `.mfb` body, not
/// `transform`), which is why it needs its own case and its own fix.
#[cfg(unix)]
#[test]
fn map_values_runs_at_constant_rss_with_a_string_callback() {
    assert_flat("b569_map_values", SHAPE_569_MAP_VALUES, 50_000, 100_000);
}

/// The POSITIVE pin, as RSS: freeing nothing here is a leak, freeing twice is
/// heap corruption.
#[cfg(unix)]
#[test]
fn a_bare_parameter_callback_runs_at_constant_rss() {
    assert_flat(
        "b569_bare_param",
        SHAPE_569_BARE_PARAM_CALLBACK,
        50_000,
        100_000,
    );
}

#[cfg(unix)]
#[test]
fn a_fixed_width_callback_stays_flat() {
    assert_flat(
        "b569_fixed_width",
        SHAPE_569_FIXED_WIDTH_CONTRAST,
        50_000,
        100_000,
    );
}

/// The VALUES half of bug-569, run REPEATEDLY, because the failure mode a free
/// introduces is not a red assertion: a double free corrupts the arena's free
/// list and surfaces later — as "Allocation failed" on an unrelated allocation, as
/// a block read back empty, or as a fault — and none of those are deterministic
/// from one run.
///
/// One program exercises all four HOFs plus a callback reached through a
/// user-written function, over five callback shapes including the two that bracket
/// the danger: `RETURN s` (the block the loop already freed) and
/// `RETURN toString(s)` (the identity, bug-562's crash). Every pass re-reads the
/// SOURCE list as well as the results, so a free that reached into the collection
/// shows up as a changed source.
#[test]
fn every_hof_frees_its_callback_result_exactly_once() {
    const SOURCE: &str = "IMPORT io\n\
IMPORT collections\n\
IMPORT strings\n\
FUNC bare(s AS String) AS String\n  RETURN s\nEND FUNC\n\
FUNC iden(s AS String) AS String\n  RETURN toString(s)\nEND FUNC\n\
FUNC deco(s AS String) AS String\n  RETURN \"<\" & s & \">\"\nEND FUNC\n\
FUNC up(s AS String) AS String\n  RETURN strings::upper(s)\nEND FUNC\n\
FUNC lit(s AS String) AS String\n  RETURN \"K\"\nEND FUNC\n\
FUNC klen(s AS String) AS Integer\n  RETURN len(s)\nEND FUNC\n\
FUNC apply(via AS FUNC(String) AS String, s AS String) AS String\n  RETURN via(s)\nEND FUNC\n\
FUNC digest(xs AS List OF String) AS String\n\
  MUT d AS String = \"\"\n\
  FOR EACH e IN xs\n\
    d = d & e & \";\"\n\
  NEXT\n\
  RETURN d\n\
END FUNC\n\
SUB main()\n\
  LET xs AS List OF String = [\"ab\", \"cd\", \"ef\", \"gh\", \"ij\", \"kl\", \"mn\", \"op\"]\n\
  MUT ms AS Map OF Integer TO String = Map OF Integer TO String {}\n\
  MUT j AS Integer = 0\n\
  WHILE j < 8\n\
    ms = collections::set(ms, j, \"v\" & toString(j))\n\
    j = j + 1\n\
  END WHILE\n\
  MUT rep AS Integer = 0\n\
  MUT sig AS String = \"\"\n\
  WHILE rep < 400\n\
    LET a AS List OF String = collections::transform(xs, bare)\n\
    LET b AS List OF String = collections::transform(xs, iden)\n\
    LET c AS List OF String = collections::transform(xs, deco)\n\
    LET d AS List OF String = collections::transform(xs, up)\n\
    LET e AS List OF String = collections::transform(xs, lit)\n\
    LET f AS List OF String = collections::transform(xs, LAMBDA(s AS String) -> toString(s))\n\
    LET g AS List OF String = collections::sortBy(xs, deco)\n\
    LET h AS Map OF Integer TO List OF String = collections::groupBy(xs, klen, bare)\n\
    LET k AS Map OF Integer TO String = collections::mapValues(ms, deco)\n\
    LET m AS Map OF Integer TO String = collections::mapValues(ms, bare)\n\
    LET n AS String = apply(iden, \"held-\" & toString(rep))\n\
    LET cur AS String = digest(a) & digest(b) & digest(c) & digest(d) & digest(e) & digest(f) & digest(g) & digest(collections::get(h, 2)) & collections::get(k, 3) & collections::get(m, 4)\n\
    IF rep = 0 THEN\n\
      sig = cur\n\
    END IF\n\
    IF cur <> sig THEN\n\
      io::print(\"DRIFT at rep=\" & toString(rep) & \" now=\" & cur)\n\
      EXIT SUB\n\
    END IF\n\
    IF n <> \"held-\" & toString(rep) THEN\n\
      io::print(\"INDIRECT WRONG at rep=\" & toString(rep) & \" n=\" & n)\n\
      EXIT SUB\n\
    END IF\n\
    IF collections::get(xs, 0) <> \"ab\" OR collections::get(xs, 7) <> \"op\" THEN\n\
      io::print(\"SOURCE CLOBBERED at rep=\" & toString(rep))\n\
      EXIT SUB\n\
    END IF\n\
    rep = rep + 1\n\
  END WHILE\n\
  io::print(sig)\n\
  io::print(\"ok\")\n\
END SUB\n";

    let expected_digest = {
        let xs = ["ab", "cd", "ef", "gh", "ij", "kl", "mn", "op"];
        let join = |v: Vec<String>| -> String {
            v.into_iter().map(|s| format!("{s};")).collect::<String>()
        };
        let identity = join(xs.iter().map(|s| s.to_string()).collect());
        let decorated: Vec<String> = xs.iter().map(|s| format!("<{s}>")).collect();
        let mut sorted_by_deco = xs.to_vec();
        sorted_by_deco.sort_by_key(|s| format!("<{s}>"));
        format!(
            "{identity}{identity}{}{}{}{identity}{}{identity}{}{}",
            join(decorated),
            join(xs.iter().map(|s| s.to_uppercase()).collect()),
            join(xs.iter().map(|_| "K".to_string()).collect()),
            join(sorted_by_deco.iter().map(|s| s.to_string()).collect()),
            "<v3>",
            "v4",
        )
    };

    let project = common::temp_project("b569_hof_values", SOURCE);
    let exe = common::build_project(&project);
    // A double free is not deterministic: it corrupts the free list and surfaces
    // on some later allocation, which may or may not happen in a given run.
    for run in 1..=25 {
        let output = std::process::Command::new(&exe)
            .output()
            .expect("run the HOF callback-ownership probe");
        assert!(
            output.status.success(),
            "run {run}: {}\n{}",
            common::exit_description(&output.status),
            String::from_utf8_lossy(&output.stdout)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(
            stdout.trim(),
            format!("{expected_digest}\nok"),
            "run {run}: a HOF callback result changed — a block was freed twice, \
             or freed while the collection still owned it"
        );
    }
    let _ = std::fs::remove_dir_all(&project);
}

// -------------------------------------------------------------- bug-571

/// bug-571: `FOR EACH e IN <List OF String>` reading only `len(e)` — the bug
/// report's own twelve-line reproduction, with no callback and no HOF anywhere in
/// the program. A packed `String` element has no standalone header to point at,
/// so the loop materialises a fresh arena block per element per pass; nothing
/// freed it. 25 MB at 50 000 passes over an 8-element list, 50 MB at 100 000.
const SHAPE_571_LIST_OF_STRING: &str = "IMPORT io\n\
SUB main()\n\
  LET xs AS List OF String = [\"n0\", \"n1\", \"n2\", \"n3\", \"n4\", \"n5\", \"n6\", \"n7\"]\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    FOR EACH e IN xs\n\
      acc = acc + len(e)\n\
    NEXT\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// A `Map OF String TO String` materialises BOTH sides per entry — the key and
/// the value are separate loads — so it leaked twice as fast: 50 MB at 50 000,
/// 99 MB at 100 000.
const SHAPE_571_MAP_OF_STRING: &str = "IMPORT io\n\
IMPORT collections\n\
SUB main()\n\
  MUT xs AS Map OF String TO String = Map OF String TO String {}\n\
  MUT j AS Integer = 0\n\
  WHILE j < 8\n\
    xs = collections::set(xs, \"k\" & toString(j), \"v\" & toString(j))\n\
    j = j + 1\n\
  END WHILE\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    FOR EACH e IN xs\n\
      acc = acc + len(e.key) + len(e.value)\n\
    NEXT\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// A `Set OF String` reaches the same materialising arm on its own code path in
/// `lower_for_each`; it leaked identically (25 -> 50 MB) and is the arm a
/// `List`/`Map` enumeration silently omits.
const SHAPE_571_SET_OF_STRING: &str = "IMPORT io\n\
IMPORT collections\n\
SUB main()\n\
  MUT xs AS Set OF String = Set OF String {}\n\
  MUT j AS Integer = 0\n\
  WHILE j < 8\n\
    xs = collections::add(xs, \"k\" & toString(j))\n\
    j = j + 1\n\
  END WHILE\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    FOR EACH e IN xs\n\
      acc = acc + len(e)\n\
    NEXT\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// The bug report's contrast, and the case that says it is the `String` element
/// and not the loop: `FOR EACH` over a `Map OF Integer TO Integer` reading
/// `e.key`/`e.value` was 1.0 MB flat at both counts before the fix, and must stay
/// flat after it — its payload arms materialise nothing, so a fix that freed
/// anything here would be freeing a scalar.
const SHAPE_571_CONTRAST_MAP_OF_INTEGER: &str = "IMPORT io\n\
IMPORT collections\n\
SUB main()\n\
  MUT xs AS Map OF Integer TO Integer = Map OF Integer TO Integer {}\n\
  MUT j AS Integer = 0\n\
  WHILE j < 8\n\
    xs = collections::set(xs, j, j)\n\
    j = j + 1\n\
  END WHILE\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    FOR EACH e IN xs\n\
      acc = acc + e.key + e.value\n\
    NEXT\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// The `List` half of the same contrast.
const SHAPE_571_CONTRAST_LIST_OF_INTEGER: &str = "IMPORT io\n\
SUB main()\n\
  LET xs AS List OF Integer = [0, 1, 2, 3, 4, 5, 6, 7]\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    FOR EACH e IN xs\n\
      acc = acc + e\n\
    NEXT\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// `EXIT FOR` leaves the loop from the MIDDLE of an iteration, branching straight
/// to the end label past the fall-through drop. 16 -> 31 MB before.
const SHAPE_571_EXIT_FOR: &str = "IMPORT io\n\
SUB main()\n\
  LET xs AS List OF String = [\"n0\", \"n1\", \"n2\", \"n3\", \"n4\", \"n5\", \"n6\", \"n7\"]\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    FOR EACH e IN xs\n\
      IF e = \"n4\" THEN\n\
        EXIT FOR\n\
      END IF\n\
      acc = acc + len(e)\n\
    NEXT\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// `CONTINUE FOR` branches to the TOP of the loop, also past the fall-through
/// drop. 25 -> 50 MB before.
const SHAPE_571_CONTINUE_FOR: &str = "IMPORT io\n\
SUB main()\n\
  LET xs AS List OF String = [\"n0\", \"n1\", \"n2\", \"n3\", \"n4\", \"n5\", \"n6\", \"n7\"]\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    FOR EACH e IN xs\n\
      IF e = \"n4\" THEN\n\
        CONTINUE FOR\n\
      END IF\n\
      acc = acc + len(e)\n\
    NEXT\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// `RETURN e` from inside the loop — the escape direction, and the one shape
/// where freeing the item would be a use-after-free in the CALLER. It stays
/// correct because the item is registered as an ordinary `OwnedValue` cleanup, so
/// `plan_returned_move` finds it by stack offset and moves the block out instead
/// of freeing it on that path. 16 -> 31 MB before (the returned block's ORIGINAL
/// was leaked); the printed `src=` proves the caller still reads it.
const SHAPE_571_RETURN_ITEM: &str = "IMPORT io\n\
FUNC pick(xs AS List OF String) AS String\n\
  FOR EACH e IN xs\n\
    IF e = \"n4\" THEN\n\
      RETURN e\n\
    END IF\n\
  NEXT\n\
  RETURN \"none\"\n\
END FUNC\n\
SUB main()\n\
  LET xs AS List OF String = [\"n0\", \"n1\", \"n2\", \"n3\", \"n4\", \"n5\", \"n6\", \"n7\"]\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    LET got AS String = pick(xs)\n\
    acc = acc + len(got)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc) & \" src=\" & pick(xs))\n\
END SUB\n";

/// A body that USES the element beyond `len` — appends it to a collection and
/// concatenates it into a string. Both are owning consumers that COPY (§14.6:
/// "inserting into a container copies or moves the inserted value into the
/// container; it never stores a non-owning alias"), so the block stays the
/// loop's to drop — but that has to be measured, not assumed. 27 -> 54 MB before;
/// the printed `src=` proves the source list is intact afterward.
const SHAPE_571_USES_THE_ITEM: &str = "IMPORT io\n\
IMPORT collections\n\
SUB main()\n\
  LET xs AS List OF String = [\"n0\", \"n1\", \"n2\", \"n3\", \"n4\", \"n5\", \"n6\", \"n7\"]\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    MUT out AS List OF String = []\n\
    MUT s AS String = \"\"\n\
    FOR EACH e IN xs\n\
      out = collections::append(out, e)\n\
      s = s & e\n\
    NEXT\n\
    acc = acc + len(collections::get(out, 7)) + len(s)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc) & \" src=\" & collections::get(xs, 0))\n\
END SUB\n";

/// 25 MB at 50 000 passes over an 8-element list, 50 MB at 100 000, before.
#[cfg(unix)]
#[test]
fn a_for_each_over_a_list_of_string_does_not_leak_its_element() {
    assert_flat(
        "b571_list_of_string",
        SHAPE_571_LIST_OF_STRING,
        50_000,
        100_000,
    );
}

/// 50 -> 99 MB before: both sides of every entry.
#[cfg(unix)]
#[test]
fn a_for_each_over_a_map_of_string_does_not_leak_either_side() {
    assert_flat(
        "b571_map_of_string",
        SHAPE_571_MAP_OF_STRING,
        50_000,
        100_000,
    );
}

/// 25 -> 50 MB before, on `lower_for_each`'s own Set arm.
#[cfg(unix)]
#[test]
fn a_for_each_over_a_set_of_string_does_not_leak_its_element() {
    assert_flat(
        "b571_set_of_string",
        SHAPE_571_SET_OF_STRING,
        50_000,
        100_000,
    );
}

/// The contrast that attributes the leak to the `String` element rather than to
/// the loop: flat before, and it must stay flat.
#[cfg(unix)]
#[test]
fn a_for_each_over_fixed_width_elements_stays_flat() {
    assert_flat(
        "b571_contrast_map_int",
        SHAPE_571_CONTRAST_MAP_OF_INTEGER,
        50_000,
        100_000,
    );
    assert_flat(
        "b571_contrast_list_int",
        SHAPE_571_CONTRAST_LIST_OF_INTEGER,
        50_000,
        100_000,
    );
}

/// `EXIT FOR` (16 -> 31 MB) and `CONTINUE FOR` (25 -> 50 MB) both jump around the
/// fall-through drop, so each is its own case.
#[cfg(unix)]
#[test]
fn an_early_exit_from_a_for_each_does_not_leak_the_item() {
    assert_flat("b571_exit_for", SHAPE_571_EXIT_FOR, 50_000, 100_000);
    assert_flat("b571_continue_for", SHAPE_571_CONTINUE_FOR, 50_000, 100_000);
}

/// The escape direction: `RETURN e` hands the block to the caller. 16 -> 31 MB
/// before, and the value half is checked by
/// `every_for_each_body_shape_still_produces_the_right_value` below — a fix that
/// freed the returned block would read as a wrong value or a later "Allocation
/// failed", never as this assertion.
#[cfg(unix)]
#[test]
fn returning_the_loop_item_neither_leaks_nor_double_frees() {
    assert_flat("b571_return_item", SHAPE_571_RETURN_ITEM, 50_000, 100_000);
}

/// A body that stores and concatenates the element. 27 -> 54 MB before.
#[cfg(unix)]
#[test]
fn a_for_each_body_that_uses_the_item_does_not_leak_it() {
    assert_flat("b571_uses_item", SHAPE_571_USES_THE_ITEM, 50_000, 100_000);
}

/// The VALUE half of bug-571, and the half a leak test cannot see.
///
/// Adding a free is the double-free direction, and the arena reports a double
/// free as "Allocation failed" at some later, unrelated allocation — or as a
/// wrong value read back out of reused memory — not as a crash at the site. So
/// every shape whose block might have another owner is exercised for its VALUE,
/// 25 times, against an expectation computed here rather than by the program:
///
/// * `RETURN e` — the block leaves the loop (moved, not freed).
/// * `append(out, e)` / `s = s & e` — owning consumers that copy (§14.6).
/// * a record element and a nested-collection element — these ALIAS the
///   container's own block (`emit_load_payload_with_stride` hands back its `data`
///   pointer for both arms), so freeing one corrupts the collection. Reading the
///   container again afterward is what catches it.
/// * a `Set OF String` and both one-sided `Map`s — the arms an enumeration built
///   from `List` alone would omit.
#[test]
fn every_for_each_body_shape_still_produces_the_right_value() {
    const SOURCE: &str = "IMPORT io\n\
IMPORT collections\n\
TYPE Row\n  name AS String\n  n AS Integer\nEND TYPE\n\
FUNC firstLong(xs AS List OF String) AS String\n\
  FOR EACH e IN xs\n\
    IF len(e) > 2 THEN\n\
      RETURN e\n\
    END IF\n\
  NEXT\n\
  RETURN \"none\"\n\
END FUNC\n\
FUNC joined(xs AS List OF String) AS String\n\
  MUT out AS String = \"\"\n\
  FOR EACH e IN xs\n\
    out = out & e & \"|\"\n\
  NEXT\n\
  RETURN out\n\
END FUNC\n\
FUNC copied(xs AS List OF String) AS List OF String\n\
  MUT out AS List OF String = []\n\
  FOR EACH e IN xs\n\
    out = collections::append(out, e)\n\
  NEXT\n\
  RETURN out\n\
END FUNC\n\
FUNC exitAt(xs AS List OF String, stop AS String) AS Integer\n\
  MUT n AS Integer = 0\n\
  FOR EACH e IN xs\n\
    IF e = stop THEN\n\
      EXIT FOR\n\
    END IF\n\
    n = n + len(e)\n\
  NEXT\n\
  RETURN n\n\
END FUNC\n\
FUNC skipping(xs AS List OF String, skip AS String) AS Integer\n\
  MUT n AS Integer = 0\n\
  FOR EACH e IN xs\n\
    IF e = skip THEN\n\
      CONTINUE FOR\n\
    END IF\n\
    n = n + len(e)\n\
  NEXT\n\
  RETURN n\n\
END FUNC\n\
FUNC nested(xs AS List OF String, ys AS List OF String) AS Integer\n\
  MUT n AS Integer = 0\n\
  FOR EACH a IN xs\n\
    FOR EACH b IN ys\n\
      n = n + len(a) + len(b)\n\
    NEXT\n\
  NEXT\n\
  RETURN n\n\
END FUNC\n\
SUB main()\n\
  LET xs AS List OF String = [\"aa\", \"bbb\", \"cccc\", \"d\"]\n\
  io::print(firstLong(xs))\n\
  io::print(joined(xs))\n\
  LET c AS List OF String = copied(xs)\n\
  io::print(collections::get(c, 0) & collections::get(c, 3) & toString(len(c)))\n\
  io::print(collections::get(xs, 0) & collections::get(xs, 2))\n\
  io::print(toString(exitAt(xs, \"cccc\")))\n\
  io::print(toString(skipping(xs, \"bbb\")))\n\
  io::print(toString(nested(xs, xs)))\n\
  LET ps AS List OF Row = [Row[\"one\", 1], Row[\"two\", 2], Row[\"three\", 3]]\n\
  MUT pacc AS String = \"\"\n\
  MUT pn AS Integer = 0\n\
  FOR EACH p IN ps\n\
    pacc = pacc & p.name & \";\"\n\
    pn = pn + p.n\n\
  NEXT\n\
  io::print(pacc & toString(pn))\n\
  LET p0 AS Row = collections::get(ps, 0)\n\
  LET p2 AS Row = collections::get(ps, 2)\n\
  io::print(p0.name & p2.name)\n\
  LET ls AS List OF List OF Integer = [[1, 2], [3, 4, 5], [6]]\n\
  MUT lacc AS Integer = 0\n\
  FOR EACH inner IN ls\n\
    lacc = lacc + len(inner)\n\
  NEXT\n\
  io::print(toString(lacc) & toString(len(collections::get(ls, 1))))\n\
  MUT st AS Set OF String = Set OF String {}\n\
  st = collections::add(st, \"pp\")\n\
  st = collections::add(st, \"qqq\")\n\
  MUT sacc AS Integer = 0\n\
  FOR EACH s IN st\n\
    sacc = sacc + len(s)\n\
  NEXT\n\
  io::print(toString(sacc) & toString(len(st)))\n\
  MUT msi AS Map OF String TO Integer = Map OF String TO Integer {}\n\
  msi = collections::set(msi, \"kk\", 7)\n\
  msi = collections::set(msi, \"lll\", 9)\n\
  MUT m1 AS Integer = 0\n\
  FOR EACH e IN msi\n\
    m1 = m1 + len(e.key) + e.value\n\
  NEXT\n\
  io::print(toString(m1) & toString(collections::get(msi, \"lll\")))\n\
  MUT mis AS Map OF Integer TO String = Map OF Integer TO String {}\n\
  mis = collections::set(mis, 1, \"xx\")\n\
  mis = collections::set(mis, 2, \"yyy\")\n\
  MUT m2 AS Integer = 0\n\
  FOR EACH e IN mis\n\
    m2 = m2 + e.key + len(e.value)\n\
  NEXT\n\
  io::print(toString(m2) & collections::get(mis, 2))\n\
END SUB\n";

    // Computed here, not read off the program: a value derived from the producer
    // is true by construction.
    let xs = ["aa", "bbb", "cccc", "d"];
    let expected = [
        "bbb".to_string(),
        format!("{}|{}|{}|{}|", xs[0], xs[1], xs[2], xs[3]),
        format!("{}{}{}", xs[0], xs[3], xs.len()),
        format!("{}{}", xs[0], xs[2]),
        (xs[0].len() + xs[1].len()).to_string(),
        (xs[0].len() + xs[2].len() + xs[3].len()).to_string(),
        (xs.len() * xs.iter().map(|s| s.len()).sum::<usize>() * 2).to_string(),
        "one;two;three;6".to_string(),
        "onethree".to_string(),
        "63".to_string(),
        "52".to_string(),
        "219".to_string(),
        "8yyy".to_string(),
    ]
    .join("\n");

    let project = common::temp_project("b571_for_each_values", SOURCE);
    let exe = common::build_project(&project);
    // A double free is not deterministic: it corrupts the free list and surfaces
    // on some later allocation, which may or may not happen in a given run.
    for run in 1..=25 {
        let output = std::process::Command::new(&exe)
            .output()
            .expect("run the FOR EACH item-ownership probe");
        assert!(
            output.status.success(),
            "run {run}: {}\n{}",
            common::exit_description(&output.status),
            String::from_utf8_lossy(&output.stdout)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            expected,
            "run {run}: a `FOR EACH` body read a different value — the loop freed \
             a block the container or the caller still owned"
        );
    }
    let _ = std::fs::remove_dir_all(&project);
}

// -------------------------------------------------------------- bug-572

/// bug-572, the bug report's own reproduction: a capturing `LAMBDA` passed to a
/// HOF leaked its whole environment on every call — the env block, the deep copy
/// of each captured value, and the 16-byte closure object. It is independent of
/// the callback's RESULT type, which is why the predicate is `Boolean`: that
/// allocates no result block at all, so nothing here is bug-562/569's leak.
/// 13 MB at 50 000 calls, 25 MB at 100 000.
const SHAPE_572_CAPTURING_FILTER: &str = "IMPORT io\n\
IMPORT collections\n\
SUB main()\n\
  LET cap AS String = \"CAPTURED\"\n\
  LET xs AS List OF String = [\"n0\", \"n1\", \"n2\", \"n3\", \"n4\", \"n5\", \"n6\", \"n7\"]\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    LET c AS List OF String = collections::filter(xs, LAMBDA(s AS String) -> len(s) < len(cap))\n\
    acc = acc + len(collections::get(c, 0))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// The contrast, one token wide: dropping the capture makes the lambda lower to
/// a `FunctionRef` over a static BSS descriptor rather than a `Closure`, so it
/// allocates nothing. 1.0 MB flat at both counts before AND after — a fix that
/// made the case above pass by suppressing an allocation would show up here.
const SHAPE_572_CONTRAST_CAPTURELESS: &str = "IMPORT io\n\
IMPORT collections\n\
SUB main()\n\
  LET xs AS List OF String = [\"n0\", \"n1\", \"n2\", \"n3\", \"n4\", \"n5\", \"n6\", \"n7\"]\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    LET c AS List OF String = collections::filter(xs, LAMBDA(s AS String) -> len(s) < 8)\n\
    acc = acc + len(collections::get(c, 0))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// `collections::reduce`'s BINARY callback (`FUNC(U, T) AS U`, parameter index 2)
/// — a different position on the allow-list, and the one `callback_member`'s
/// unary rule deliberately excludes. 13 -> 25 MB before.
const SHAPE_572_REDUCE: &str = "IMPORT io\n\
IMPORT collections\n\
SUB main()\n\
  LET cap AS String = \"CAPTURED\"\n\
  LET xs AS List OF Integer = [1, 2, 3, 4, 5, 6, 7, 8]\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    LET n AS Integer = collections::reduce(xs, 0, LAMBDA(a AS Integer, e AS Integer) -> a + e + len(cap))\n\
    acc = acc + n\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// `collections::forEach`, the one position that admits a BY-REF capture of a
/// `MUT` binding (`is_nonescaping_callback_arg`). A by-ref env slot holds a
/// pointer to the parent's stack slot, not an owned block, and
/// `capture_free_type` answers the empty type for it so the drop SKIPS it — only
/// the env array and the object are reclaimed. 7 -> 13 MB before; a wild free of
/// the by-ref slot would corrupt the caller's frame instead.
const SHAPE_572_FOREACH_BY_REF: &str = "IMPORT io\n\
IMPORT collections\n\
SUB main()\n\
  LET xs AS List OF String = [\"n0\", \"n1\", \"n2\", \"n3\", \"n4\", \"n5\", \"n6\", \"n7\"]\n\
  MUT total AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    collections::forEach(xs, LAMBDA(s AS String) -> total = total + len(s))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"total=\" & toString(total))\n\
END SUB\n";

/// `collections::mapValues`, whose callback is invoked from its own `.mfb` body
/// rather than a native loop — so it takes the OTHER arm of the gate, the one
/// that reads the callee's NIR and asks `collect_value_used_locals` whether the
/// parameter is invoke-only. 15 -> 29 MB before.
const SHAPE_572_MAP_VALUES: &str = "IMPORT io\n\
IMPORT collections\n\
SUB main()\n\
  LET cap AS String = \"CAPTURED\"\n\
  MUT m AS Map OF Integer TO Integer = Map OF Integer TO Integer {}\n\
  MUT j AS Integer = 0\n\
  WHILE j < 8\n\
    m = collections::set(m, j, j)\n\
    j = j + 1\n\
  END WHILE\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    LET c AS Map OF Integer TO Integer = collections::mapValues(m, LAMBDA(v AS Integer) -> v + len(cap))\n\
    acc = acc + collections::get(c, 1)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// The POSITIVE pin, as an RSS EQUALITY rather than flatness: `http::route`'s
/// handler is also a non-isolated `FUNC` parameter, but the returned
/// `http::Route` KEEPS it and the server invokes it per request. It must keep
/// leaking exactly as much as it did before — 6 -> 11 MB at 20k/40k on both
/// compilers, byte-identical — because the alternative is a use-after-free the
/// next time the route is served.
///
/// Asserted as "still grows", which is the only assertion that distinguishes
/// "declined" from "freed" here; the VALUE half is
/// `every_closure_argument_shape_still_produces_the_right_value` below.
const SHAPE_572_RETAINED_ROUTE: &str = "IMPORT io\n\
IMPORT http\n\
SUB main()\n\
  LET cap AS String = \"CAPTURED\"\n\
  MUT last AS http::Route = http::route(\"/seed\", LAMBDA(req AS http::Request) -> http::ok(\"seed\"))\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    LET r AS http::Route = http::route(\"/x/:id\", LAMBDA(req AS http::Request) -> http::ok(cap))\n\
    last = r\n\
    acc = acc + len(r.pattern)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc) & \" pattern=\" & last.pattern)\n\
END SUB\n";

/// 13 MB at 50 000 calls, 25 MB at 100 000, before — with a `Boolean` predicate.
#[cfg(unix)]
#[test]
fn a_capturing_lambda_argument_does_not_leak_its_environment() {
    assert_flat(
        "b572_capturing_filter",
        SHAPE_572_CAPTURING_FILTER,
        50_000,
        100_000,
    );
}

/// The one-token contrast that attributes the leak to the CAPTURE.
#[cfg(unix)]
#[test]
fn a_captureless_lambda_argument_stays_flat() {
    assert_flat(
        "b572_contrast_captureless",
        SHAPE_572_CONTRAST_CAPTURELESS,
        50_000,
        100_000,
    );
}

/// The other three admitted positions, each on its own path to the callback:
/// `reduce`'s binary combiner (13 -> 25 MB), `forEach`'s by-ref capture
/// (7 -> 13 MB), and `mapValues`' `.mfb` body (15 -> 29 MB).
#[cfg(unix)]
#[test]
fn every_admitted_callback_position_frees_its_closure() {
    assert_flat("b572_reduce", SHAPE_572_REDUCE, 50_000, 100_000);
    assert_flat(
        "b572_foreach_by_ref",
        SHAPE_572_FOREACH_BY_REF,
        50_000,
        100_000,
    );
    assert_flat("b572_map_values", SHAPE_572_MAP_VALUES, 50_000, 100_000);
}

/// `http::route` KEEPS its handler, so it must NOT have been freed — and the
/// evidence that it was not is that its (pre-existing, unrelated) growth is
/// unchanged.
///
/// The counts are 200k/400k, not the 20k/40k this was written at, because the
/// per-iteration retention is FOUR TIMES smaller off macOS and 20k/40k left it
/// under the arena's chunk floor everywhere else. Peak RSS, HEAD compiler, one
/// program per cell:
///
/// | N    | macOS aarch64 | linux aarch64 glibc | linux x86-64 glibc | linux riscv64 musl |
/// |------|---------------|---------------------|--------------------|--------------------|
/// | 20k  |  6 MB         |  6 MB               | —                  | —                  |
/// | 40k  | 11 MB         |  6 MB               | —                  | —                  |
/// | 200k | 51 MB         | 13 MB               | 13 MB              | 13 MB              |
/// | 400k | 101 MB        | 26 MB               | 26 MB              | 25 MB              |
///
/// At 20k/40k the growth is +5 MB on macOS and +0 MB on Linux — which is what
/// reddened both Linux rows with "`http::route`'s handler stopped leaking", a
/// verdict the 200k/400k and 2M (127 MB on linux-aarch64) rows refute: the
/// handler is retained on every platform, and the counts were the thing that was
/// macOS-only. The 2 MB threshold now has a 6x margin on the leanest platform.
#[cfg(unix)]
#[test]
fn a_retained_callback_position_is_left_alone() {
    let small = peak_rss("b572_retained_route", SHAPE_572_RETAINED_ROUTE, 200_000);
    let large = peak_rss("b572_retained_route", SHAPE_572_RETAINED_ROUTE, 400_000);
    assert!(
        large > small + 2 * 1024 * 1024,
        "`http::route`'s handler stopped leaking ({} MB -> {} MB). That is not an \
         improvement here: the returned `http::Route` still holds the pointer, so \
         a free means the next request serves freed memory. If this position is \
         ever made safe to free, it moves from RETAINED_CALLBACK_PARAMETERS to \
         SYNCHRONOUS_CALLBACK_PARAMETERS and this case is replaced by assert_flat",
        small / (1024 * 1024),
        large / (1024 * 1024),
    );
}

/// The VALUE half of bug-572, and the half a leak test cannot see.
///
/// Freeing a closure the callee kept is a use-after-free that surfaces LATER —
/// as a wrong value read out of reused memory, or as "Allocation failed" in some
/// unrelated allocation — so every escape the gate declines is exercised for its
/// value, 25 times, against an expectation computed here:
///
/// * `RETURN LAMBDA…` out of a function, and a user HOF that returns its
///   callable parameter.
/// * a closure appended to a `List OF FUNC(…)`, which stores the POINTER
///   (bug-73), and one assigned to a global.
/// * `http::route`, whose returned record keeps the handler.
/// * and, on the other side, the closures that ARE freed — including a nested
///   pair, where a positional mispairing between the two drains would free the
///   wrong one.
#[test]
fn every_closure_argument_shape_still_produces_the_right_value() {
    const SOURCE: &str = "IMPORT io\n\
IMPORT collections\n\
MUT gfn AS FUNC(String) AS Boolean = LAMBDA(s AS String) -> len(s) < 3\n\
FUNC make(cap AS String) AS FUNC(String) AS Boolean\n\
  RETURN LAMBDA(s AS String) -> len(s) < len(cap)\n\
END FUNC\n\
FUNC hold(f AS FUNC(String) AS Boolean) AS FUNC(String) AS Boolean\n\
  RETURN f\n\
END FUNC\n\
FUNC applyTwice(f AS FUNC(Integer) AS Integer, v AS Integer) AS Integer\n\
  RETURN f(f(v))\n\
END FUNC\n\
SUB main()\n\
  LET cap AS String = \"CAPTURED\"\n\
  LET xs AS List OF String = [\"a\", \"bb\", \"ccc\", \"dddd\"]\n\
  LET kept AS List OF String = collections::filter(xs, LAMBDA(s AS String) -> len(s) < len(cap))\n\
  io::print(toString(len(kept)) & collections::get(kept, 0))\n\
  LET small AS List OF String = collections::filter(xs, LAMBDA(s AS String) -> len(s) < 3)\n\
  io::print(toString(len(small)))\n\
  LET bump AS Integer = 5\n\
  io::print(toString(applyTwice(LAMBDA(v AS Integer) -> v + bump, 1)))\n\
  LET g AS FUNC(String) AS Boolean = make(cap)\n\
  io::print(toString(g(\"ab\")) & toString(g(\"abcdefghij\")))\n\
  LET h AS FUNC(String) AS Boolean = hold(LAMBDA(s AS String) -> len(s) < len(cap))\n\
  io::print(toString(h(\"ab\")) & toString(h(\"abcdefghij\")))\n\
  MUT fs AS List OF FUNC(String) AS Boolean = []\n\
  fs = collections::append(fs, LAMBDA(s AS String) -> len(s) < len(cap))\n\
  LET stored AS FUNC(String) AS Boolean = collections::get(fs, 0)\n\
  io::print(toString(stored(\"ab\")) & toString(stored(\"abcdefghij\")))\n\
  gfn = LAMBDA(s AS String) -> len(s) < len(cap)\n\
  io::print(toString(gfn(\"ab\")) & toString(gfn(\"abcdefghij\")))\n\
  io::print(cap & collections::get(xs, 0) & collections::get(xs, 3))\n\
  LET both AS List OF String = collections::filter(collections::filter(xs, LAMBDA(s AS String) -> len(s) < len(cap)), LAMBDA(s AS String) -> len(s) > 1)\n\
  io::print(toString(len(both)) & collections::get(both, 0))\n\
  MUT total AS Integer = 0\n\
  collections::forEach(xs, LAMBDA(s AS String) -> total = total + len(s))\n\
  io::print(toString(total))\n\
  io::print(toString(collections::reduce(xs, 0, LAMBDA(a AS Integer, s AS String) -> a + len(s) + len(cap))))\n\
END SUB\n";

    // Computed here, not read off the program.
    let cap = "CAPTURED";
    let xs = ["a", "bb", "ccc", "dddd"];
    let lengths: usize = xs.iter().map(|s| s.len()).sum();
    let shorter_than_cap: Vec<&&str> = xs.iter().filter(|s| s.len() < cap.len()).collect();
    let expected = [
        format!("{}{}", shorter_than_cap.len(), xs[0]),
        xs.iter().filter(|s| s.len() < 3).count().to_string(),
        (1 + 5 + 5).to_string(),
        "TRUEFALSE".to_string(),
        "TRUEFALSE".to_string(),
        "TRUEFALSE".to_string(),
        "TRUEFALSE".to_string(),
        format!("{cap}{}{}", xs[0], xs[3]),
        format!(
            "{}{}",
            shorter_than_cap.iter().filter(|s| s.len() > 1).count(),
            shorter_than_cap
                .iter()
                .find(|s| s.len() > 1)
                .expect("a kept element longer than one byte"),
        ),
        lengths.to_string(),
        (lengths + xs.len() * cap.len()).to_string(),
    ]
    .join("\n");

    let project = common::temp_project("b572_closure_values", SOURCE);
    let exe = common::build_project(&project);
    // A use-after-free is not deterministic: it depends on whether the arena
    // reuses the block before the read.
    for run in 1..=25 {
        let output = std::process::Command::new(&exe)
            .output()
            .expect("run the closure-argument ownership probe");
        assert!(
            output.status.success(),
            "run {run}: {}\n{}",
            common::exit_description(&output.status),
            String::from_utf8_lossy(&output.stdout)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            expected,
            "run {run}: a closure read a different value — one was freed while a \
             collection, a global, a caller, or an `http::Route` still held it"
        );
    }
    let _ = std::fs::remove_dir_all(&project);
}

// ---------------------------------------------------------------- bug-565

/// bug-565: the inline-`TRAP` ERROR branch leaked TWO arena blocks per trapped
/// error.
///
/// The raiser (`FAIL error(...)`) builds one owned flat `Error` block and PARKS
/// it in the per-thread current-error slot for the catcher to adopt (design "b").
/// The `NirValue::CallResult` lowering never adopted it — it rebuilt a fresh
/// `Error` from the loose registers, orphaning the parked block until the next
/// `FAIL` overwrote the slot — and then `emit_build_result_inline` deep-copied
/// the rebuilt block into the `Result`, orphaning that one too. 780 B per trapped
/// error: **149.8 MB at 200 000, 298.6 MB at 400 000**.
///
/// This is the exact program `a_trap_whose_call_always_fails_still_produces_the_right_value`
/// already asserts the VALUE of; bug-561 recorded it as deliberately not an RSS
/// case because the leak was a separate defect. This is that half.
const B565_TRAP_ALWAYS_FAILS: &str = B561_TRAP_ALWAYS_FAILS;

/// The handler that READS the trapped error, which is the shape the fix could get
/// catastrophically wrong: `e.code` and `RECOVER e.message` both read out of the
/// `Error` this change now frees. They read out of the `Result`'s own COPY, so the
/// free is sound — and the values are asserted below as well as the RSS.
/// 149.8 MB at 200 000 and 298.6 MB at 400 000 before.
const B565_HANDLER_READS_ERROR: &str = "IMPORT io\n\
FUNC always(n AS Integer) AS String\n\
  IF n >= 0 THEN\n    FAIL error(7, \"always-\" & toString(n MOD 3))\n  END IF\n\
  RETURN toString(n)\n\
END FUNC\n\
SUB main()\n\
  MUT acc AS Integer = 0\n\
  MUT codes AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET s AS String = always(i) TRAP(e)\n\
      codes = codes + e.code\n\
      RECOVER e.message\n\
    END TRAP\n\
    acc = acc + len(s)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc) & \" codes=\" & toString(codes))\n\
END SUB\n";

/// The shape bug-571 measured and could not fix: a failing call inside a walk
/// over a `List OF String`. bug-571 took it from 6 -> 11 MB down to the 2 -> 3 MB
/// the loop-free control costs and recorded the remainder as this bug. With eight
/// inner iterations per pass it is the same defect at eight times the rate:
/// **298.6 MB at 50 000 passes, 596.2 MB at 100 000**.
const B565_FAILING_CALL_IN_A_WALK: &str = "IMPORT io\n\
IMPORT collections\n\
FUNC boom(s AS String) AS Integer\n  FAIL error(7, \"boom\")\nEND FUNC\n\
SUB main()\n\
  LET xs AS List OF String = [\"n0\", \"n1\", \"n2\", \"n3\", \"n4\", \"n5\", \"n6\", \"n7\"]\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    MUT j AS Integer = 0\n\
    WHILE j < 8\n\
      LET e AS String = collections::getOr(xs, j, \"\")\n\
      LET v AS Integer = boom(e) TRAP(err)\n\
        RECOVER len(e)\n\
      END TRAP\n\
      acc = acc + v\n\
      j = j + 1\n\
    END WHILE\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// The AUTO-PROPAGATED edge, which is a different emitter: the error leaves the
/// middle of a loop body, unwinds out of `pass`, and is only then trapped. §14.7
/// names auto-propagation as its own scope edge; it reaches the same
/// `NirValue::CallResult` assembly at the trap site. 38.2 MB at 50 000, 75.4 MB
/// at 100 000 before.
const B565_AUTO_PROPAGATED: &str = "IMPORT io\n\
IMPORT collections\n\
FUNC boom(s AS String) AS Integer\n  FAIL error(7, \"boom\")\nEND FUNC\n\
FUNC pass(xs AS List OF String) AS Integer\n\
  MUT j AS Integer = 0\n\
  MUT sum AS Integer = 0\n\
  WHILE j < 8\n\
    LET e AS String = collections::getOr(xs, j, \"\")\n\
    LET v AS Integer = boom(e)\n\
    sum = sum + v\n\
    j = j + 1\n\
  END WHILE\n\
  RETURN sum\n\
END FUNC\n\
SUB main()\n\
  LET xs AS List OF String = [\"n0\", \"n1\", \"n2\", \"n3\", \"n4\", \"n5\", \"n6\", \"n7\"]\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    LET v AS Integer = pass(xs) TRAP(err)\n\
      RECOVER 1\n\
    END TRAP\n\
    acc = acc + v\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// bug-573, landed as the shape bug-565 pinned as still-leaking. The comment it
/// replaces read "an error raised by an inline builtin's own domain check still
/// leaks, and this change declines to touch it", asserted as GROWTH — 39.1 MB at
/// 200 000 iterations and 77.2 MB at 400 000, identical before and after bug-565.
///
/// The leak was on the RAISE side: `_mfb_make_error_result` allocates an
/// `ErrorLoc`, `_mfb_rt_park_error` builds the owned `Error` block by inlining a
/// COPY of it, and the original was orphaned before any `TRAP` was involved.
/// `_mfb_rt_park_error` now releases it, so this is `assert_flat` — and bug-565's
/// worry ("a fix that freed the raiser's `ErrorLoc` here would be freeing a block
/// the propagation path still hands to its caller") is answered by the park's
/// re-point: `x3` is left pointing at the parked block's OWN inlined copy, so a
/// propagation that reads it reads a live, byte-identical `ErrorLoc`.
const B573_BUILTIN_RAISE: &str = "IMPORT io\n\
IMPORT collections\n\
SUB main()\n\
  LET xs AS List OF String = [\"aa\", \"bb\", \"cc\"]\n\
  MUT acc AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET g AS String = collections::get(xs, 9) TRAP(e2)\n\
      RECOVER \"zz\"\n\
    END TRAP\n\
    acc = acc + len(g)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// The control that makes every number above attributable: the SAME program with
/// the producer's condition inverted, so the identical inline `TRAP` over the
/// identical callee never takes its error branch. 1.0 MB flat at both counts
/// BEFORE this change and after — which is what isolates the leak to the error
/// path rather than to the `TRAP`, the callee or the loop. (bug-561 fixed the Ok
/// half; this is the same program with the fixed half exercised.)
const B565_CONTROL_NEVER_FAILS: &str = "IMPORT io\n\
FUNC never(n AS Integer) AS String\n\
  IF n < 0 THEN\n    FAIL error(7, \"never\")\n  END IF\n\
  RETURN toString(n MOD 10)\n\
END FUNC\n\
SUB main()\n\
  MUT acc AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET s AS String = never(i) TRAP(e)\n\
      RECOVER \"fallback\"\n\
    END TRAP\n\
    acc = acc + len(s)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

#[cfg(unix)]
#[test]
fn the_same_trap_whose_call_never_fails_runs_at_constant_rss() {
    assert_flat(
        "b565_never_fails",
        B565_CONTROL_NEVER_FAILS,
        200_000,
        400_000,
    );
}

#[cfg(unix)]
#[test]
fn a_trap_whose_call_always_fails_runs_at_constant_rss() {
    assert_flat(
        "b565_always_fails",
        B565_TRAP_ALWAYS_FAILS,
        200_000,
        400_000,
    );
}

#[cfg(unix)]
#[test]
fn a_trapped_error_read_by_its_handler_runs_at_constant_rss() {
    assert_flat(
        "b565_handler_reads",
        B565_HANDLER_READS_ERROR,
        200_000,
        400_000,
    );
}

#[cfg(unix)]
#[test]
fn a_failing_call_inside_a_collection_walk_runs_at_constant_rss() {
    assert_flat(
        "b565_fail_walk",
        B565_FAILING_CALL_IN_A_WALK,
        50_000,
        100_000,
    );
}

#[cfg(unix)]
#[test]
fn an_auto_propagated_error_out_of_a_loop_body_runs_at_constant_rss() {
    assert_flat("b565_auto_prop", B565_AUTO_PROPAGATED, 50_000, 100_000);
}

#[cfg(unix)]
#[test]
fn an_inline_builtins_own_domain_error_runs_at_constant_rss() {
    assert_flat("b573_builtin_raise", B573_BUILTIN_RAISE, 200_000, 400_000);
}

/// The behaviour pin for bug-565. The fix ADDS an `arena_free` on the ERROR path
/// of every inline `TRAP`, so its failure mode is a double free or a
/// use-after-free — a wrong value, or a fault at some later allocation, never a
/// red assertion. Every shape whose `Error` the guard must handle is here:
///
/// * the ADOPT branch — a `FAIL error(...)` from a user callee, which parks its
///   block; read back through `e.code`, `e.message` and `RECOVER e.message`, all
///   of which read out of the `Result`'s copy AFTER the source has been freed;
/// * a re-raised `Error` local (`FAIL err`), which is an aliasing source and so
///   takes the loose-register REBUILD branch instead;
/// * the REBUILD branch with a stamped `ErrorLoc` — an inline builtin's domain
///   error, where the guard must free the frame's own `ErrorLoc` and not the
///   raiser's;
/// * an error caught by a FUNCTION-level `TRAP`, whose route adopts the same
///   parked slot this change now also adopts from — the one place a double adopt
///   would show;
/// * nested traps, where an inner handler raises and an outer one catches;
/// * `e.source`, so the origin survives the free of the block it was copied from.
const B565_ERROR_SHAPES: &str = "IMPORT io\n\
IMPORT collections\n\
FUNC raiser(n AS Integer) AS String\n\
  IF n MOD 2 = 0 THEN\n    FAIL error(11, \"even-\" & toString(n MOD 4))\n  END IF\n\
  RETURN \"odd\"\n\
END FUNC\n\
FUNC reraiser(n AS Integer) AS String\n\
  LET s AS String = raiser(n) TRAP(inner)\n\
    FAIL inner\n\
  END TRAP\n\
  RETURN s\n\
END FUNC\n\
FUNC viaTrap(n AS Integer) AS Integer\n\
  RETURN len(raiser(n))\n\
  TRAP(err)\n\
    RETURN 0 - err.code\n\
  END TRAP\n\
END FUNC\n\
SUB main()\n\
  LET xs AS List OF String = [\"aa\", \"bb\"]\n\
  MUT codes AS Integer = 0\n\
  MUT texts AS Integer = 0\n\
  MUT lines AS Integer = 0\n\
  MUT viaTrapSum AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < 400\n\
    LET a AS String = raiser(i) TRAP(e)\n\
      codes = codes + e.code\n\
      lines = lines + e.source.line\n\
      RECOVER e.message\n\
    END TRAP\n\
    texts = texts + len(a)\n\
    LET b AS String = reraiser(i) TRAP(e2)\n\
      codes = codes + e2.code\n\
      RECOVER e2.message\n\
    END TRAP\n\
    texts = texts + len(b)\n\
    LET g AS String = collections::get(xs, i MOD 5) TRAP(e3)\n\
      codes = codes + e3.code\n\
      RECOVER e3.message\n\
    END TRAP\n\
    texts = texts + len(g)\n\
    viaTrapSum = viaTrapSum + viaTrap(i)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"codes=\" & toString(codes))\n\
  io::print(\"texts=\" & toString(texts))\n\
  io::print(\"viaTrap=\" & toString(viaTrapSum))\n\
  io::print(\"linesPositive=\" & toString(lines > 0))\n\
END SUB\n";

#[test]
fn every_trapped_error_shape_still_produces_the_right_value() {
    let project = common::temp_project("b565_error_values", B565_ERROR_SHAPES);
    let exe = common::build_project(&project);
    // 200 even iterations raise `error(11, ...)`; each of the two user-callee
    // traps sees it, so 2 * 200 * 11. `collections::get(xs, i MOD 5)` is out of
    // range for `i MOD 5` in {2, 3, 4} — 240 of the 400 iterations — each
    // `ErrIndexOutOfRange` (77050001).
    let expected_codes = 2 * 200 * 11 + 240 * 77_050_001i64;
    // `raiser` returns "odd" (3) on the 200 odd iterations and its message
    // "even-0"/"even-2" (6) on the 200 even ones, through both callees;
    // `collections::get` yields "aa"/"bb" (2) on 160 iterations and the
    // ErrIndexOutOfRange message on the other 240.
    let get_message_len = "List or string index/range is outside valid bounds.".len() as i64;
    let expected_texts = 2 * (200 * 3 + 200 * 6) + 160 * 2 + 240 * get_message_len;
    // `viaTrap` returns len("odd") on odd iterations and -11 on even ones.
    let expected_via_trap = 200 * 3 - 200 * 11;
    let expected = format!(
        "codes={expected_codes}\ntexts={expected_texts}\nviaTrap={expected_via_trap}\n\
         linesPositive=TRUE"
    );
    // A double free is not deterministic: it corrupts the free list and surfaces
    // on some later allocation, which may or may not happen in a given run.
    for run in 1..=25 {
        let output = std::process::Command::new(&exe)
            .output()
            .expect("run the trapped-error ownership probe");
        assert!(
            output.status.success(),
            "run {run}: {}\n{}",
            common::exit_description(&output.status),
            String::from_utf8_lossy(&output.stdout)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            expected,
            "run {run}: a trapped error's value changed — the Error block was freed \
             while the Result, the handler binding or the propagation path still \
             read it"
        );
    }
    let _ = std::fs::remove_dir_all(&project);
}

// ---------------------------------------------------------------- bug-568

/// bug-568: `LET n AS Integer = risky(i) TRAP … END TRAP` leaked 134 B on every
/// call — on the SUCCESS path, with a producer that never fails.
///
/// The bind is `$trap_resN : Result OF T = CallResult(risky(i))`, and
/// `lower_value_owned` decides whether an owning store must deep-copy by asking
/// `call_returns_param_borrow` — a question about the block the CALLEE returns.
/// `risky`'s body is `RETURN i`, so the answer is "borrowed"; but what lowering
/// produced is the `{tag, size, payload}` WRAPPER this frame's own
/// `_mfb_arena_alloc` returned, with the callee's value copied into it. The bind
/// deep-copied that wrapper and abandoned the original.
///
/// It is the callee's `RETURN` shape that decides it, not the payload type, which
/// is why bug-561 read `Result OF Integer` as "never leaked at all": its contrast
/// case (`a_trap_bound_scalar_result_still_runs_at_constant_rss`, still here and
/// still green) returns `n / 2`. `RETURN i` and `RETURN i + 0` leaked;
/// `RETURN i / 2` and `LET r AS Integer = i` + `RETURN r` did not.
///
/// | | N=200k | N=400k |
/// | --- | --- | --- |
/// | before | 26.8 MB | 52.6 MB |
/// | after | 1.0 MB | 1.0 MB |
const B568_TRAP_PARAM_BORROW: &str = "IMPORT io\n\
FUNC risky(i AS Integer) AS Integer\n\
  IF i < 0 THEN\n    FAIL error(1, \"neg\")\n  END IF\n\
  RETURN i\n\
END FUNC\n\
SUB main()\n\
  MUT total AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET n AS Integer = risky(i) TRAP(e)\n\
      RECOVER 0\n\
    END TRAP\n\
    total = total + n\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"total=\" & toString(total))\n\
END SUB\n";

/// The report asked for every scalar payload type, so all four are in one loop:
/// the wrapper is the same 24 bytes whatever the scalar is, and each of the four
/// callees returns a PARAMETER, which is what triggers the copy. Four traps per
/// iteration, so four times the rate: **99.5 MB at 200 000 and 197.8 MB at
/// 400 000 before; 1.0 and 1.0 MB after.**
const B568_TRAP_EVERY_SCALAR: &str = "IMPORT io\n\
FUNC ri(i AS Integer) AS Integer\n\
  IF i < 0 THEN\n    FAIL error(1, \"neg\")\n  END IF\n  RETURN i\n\
END FUNC\n\
FUNC rf(f AS Float, i AS Integer) AS Float\n\
  IF i < 0 THEN\n    FAIL error(1, \"neg\")\n  END IF\n  RETURN f\n\
END FUNC\n\
FUNC rb(b AS Boolean, i AS Integer) AS Boolean\n\
  IF i < 0 THEN\n    FAIL error(1, \"neg\")\n  END IF\n  RETURN b\n\
END FUNC\n\
FUNC ry(y AS Byte, i AS Integer) AS Byte\n\
  IF i < 0 THEN\n    FAIL error(1, \"neg\")\n  END IF\n  RETURN y\n\
END FUNC\n\
SUB main()\n\
  LET one AS Byte = toByte(1)\n\
  MUT total AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET n AS Integer = ri(i) TRAP(e)\n      RECOVER 0\n    END TRAP\n\
    LET f AS Float = rf(1.5, i) TRAP(e2)\n      RECOVER 0.0\n    END TRAP\n\
    LET b AS Boolean = rb(TRUE, i) TRAP(e3)\n      RECOVER FALSE\n    END TRAP\n\
    LET y AS Byte = ry(one, i) TRAP(e4)\n      RECOVER toByte(0)\n    END TRAP\n\
    total = total + n\n\
    IF f > 1.0 THEN\n      total = total + 1\n    END IF\n\
    IF b THEN\n      total = total + 1\n    END IF\n\
    IF y = one THEN\n      total = total + 1\n    END IF\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"total=\" & toString(total))\n\
END SUB\n";

/// The same defect with a `String` payload, where the abandoned wrapper inlines
/// the whole string rather than an 8-byte scalar — 195 B per call: **38.2 MB at
/// 200 000 and 75.4 MB at 400 000 before; 1.0 and 1.0 MB after.**
const B568_TRAP_STRING_BORROW: &str = "IMPORT io\n\
FUNC pick(s AS String, i AS Integer) AS String\n\
  IF i < 0 THEN\n    FAIL error(1, \"neg\")\n  END IF\n\
  RETURN s\n\
END FUNC\n\
SUB main()\n\
  LET base AS String = \"abcdefghijklmnop\"\n\
  MUT total AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET n AS String = pick(base, i) TRAP(e)\n\
      RECOVER \"x\"\n\
    END TRAP\n\
    total = total + len(n)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"total=\" & toString(total))\n\
END SUB\n";

/// The POSITIVE pin: the identical param-borrow callee called WITHOUT a `TRAP`.
///
/// There the lowered value really IS the caller's own argument block, the copy is
/// what makes the binding an owner, and removing it would `arena_free` a live
/// local at scope drop. Flat before and after — and the source `base` is read
/// back on every iteration, so a wrong free shows up as a wrong total rather than
/// only as a fault.
const B568_CONTRAST_NO_TRAP: &str = "IMPORT io\n\
FUNC pick(s AS String) AS String\n  RETURN s\nEND FUNC\n\
SUB main()\n\
  LET base AS String = \"abcdefghijklmnop\"\n\
  MUT total AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET n AS String = pick(base)\n\
    total = total + len(n) + len(base)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"total=\" & toString(total))\n\
END SUB\n";

#[cfg(unix)]
#[test]
fn a_trap_over_a_param_returning_callee_runs_at_constant_rss() {
    assert_flat(
        "b568_param_borrow",
        B568_TRAP_PARAM_BORROW,
        200_000,
        400_000,
    );
}

#[cfg(unix)]
#[test]
fn a_trap_over_every_scalar_payload_runs_at_constant_rss() {
    assert_flat("b568_scalars", B568_TRAP_EVERY_SCALAR, 200_000, 400_000);
}

#[cfg(unix)]
#[test]
fn a_trap_over_a_param_returning_string_callee_runs_at_constant_rss() {
    assert_flat(
        "b568_string_borrow",
        B568_TRAP_STRING_BORROW,
        200_000,
        400_000,
    );
}

#[cfg(unix)]
#[test]
fn a_param_borrow_without_a_trap_still_runs_at_constant_rss() {
    assert_flat("b568_no_trap", B568_CONTRAST_NO_TRAP, 200_000, 400_000);
}

/// The behaviour pin for bug-568. The fix REMOVES a deep copy, so its failure
/// mode is a use-after-free — the binding freeing a block the caller still owns —
/// which surfaces as a wrong value or a fault at some later allocation, never as
/// a red assertion. Every shape whose block the binding must NOT alias is here:
///
/// * a param-borrow callee under a `TRAP`, with the SOURCE read back after the
///   trapped binding has been freed at the end of each iteration;
/// * the same callee WITHOUT a `TRAP`, where the copy is still emitted;
/// * a param-borrow callee whose trapped call FAILS, so the wrapper carries an
///   `Error` rather than the borrowed block;
/// * a record and a collection payload, which are freeable-flat like `String` and
///   so took the same copy;
/// * a callee returning a rodata literal, and one returning its own `toString`
///   argument — the two other `value_needs_owning_copy` verdicts that reach the
///   same `if`.
const B568_BORROW_SHAPES: &str = "IMPORT io\n\
IMPORT collections\n\
TYPE Duo\n  a AS Integer\n  b AS Integer\nEND TYPE\n\
FUNC pick(s AS String, i AS Integer) AS String\n\
  IF i MOD 5 = 0 THEN\n    FAIL error(3, \"five\")\n  END IF\n  RETURN s\n\
END FUNC\n\
FUNC plain(s AS String) AS String\n  RETURN s\nEND FUNC\n\
FUNC lit(i AS Integer) AS String\n\
  IF i < 0 THEN\n    FAIL error(3, \"neg\")\n  END IF\n  RETURN \"literal\"\n\
END FUNC\n\
FUNC ident(s AS String, i AS Integer) AS String\n\
  IF i < 0 THEN\n    FAIL error(3, \"neg\")\n  END IF\n  RETURN toString(s)\n\
END FUNC\n\
FUNC pair(p AS Duo, i AS Integer) AS Duo\n\
  IF i < 0 THEN\n    FAIL error(3, \"neg\")\n  END IF\n  RETURN p\n\
END FUNC\n\
FUNC same(xs AS List OF Integer, i AS Integer) AS List OF Integer\n\
  IF i < 0 THEN\n    FAIL error(3, \"neg\")\n  END IF\n  RETURN xs\n\
END FUNC\n\
SUB main()\n\
  LET base AS String = \"abcdefghij\"\n\
  LET p AS Duo = Duo[7, 9]\n\
  LET xs AS List OF Integer = [1, 2, 3, 4]\n\
  MUT total AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < 4000\n\
    LET a AS String = pick(base, i) TRAP(e)\n      RECOVER \"zz\"\n    END TRAP\n\
    LET b AS String = plain(base)\n\
    LET c AS String = lit(i) TRAP(e2)\n      RECOVER \"zz\"\n    END TRAP\n\
    LET d AS String = ident(base, i) TRAP(e3)\n      RECOVER \"zz\"\n    END TRAP\n\
    LET q AS Duo = pair(p, i) TRAP(e4)\n      RECOVER Duo[0, 0]\n    END TRAP\n\
    LET ys AS List OF Integer = same(xs, i) TRAP(e5)\n      RECOVER []\n    END TRAP\n\
    total = total + len(a) + len(b) + len(c) + len(d)\n\
    total = total + q.a + q.b + collections::get(ys, 3) + len(base)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"total=\" & toString(total))\n\
  io::print(\"base=\" & base)\n\
  io::print(\"pair=\" & toString(p.a) & \",\" & toString(p.b))\n\
  io::print(\"xs=\" & toString(collections::get(xs, 3)))\n\
END SUB\n";

#[test]
fn every_param_borrow_shape_still_produces_the_right_value() {
    let project = common::temp_project("b568_borrow_values", B568_BORROW_SHAPES);
    let exe = common::build_project(&project);
    // 4 000 iterations. `pick` fails on the 800 where `i MOD 5 = 0` (recovering
    // "zz", 2) and returns `base` (10) on the other 3 200. `plain` is 10 every
    // time; `lit` is "literal" (7); `ident` is `base` (10). `q` is 7 + 9 = 16 and
    // `collections::get(ys, 3)` is 4, plus `len(base)` = 10.
    let expected_total: i64 = (800 * 2 + 3_200 * 10) + 4_000 * (10 + 7 + 10 + 16 + 4 + 10);
    let expected = format!("total={expected_total}\nbase=abcdefghij\npair=7,9\nxs=4");
    // A use-after-free is not deterministic: it corrupts the free list and
    // surfaces on some later allocation, which may or may not happen in a run.
    for run in 1..=25 {
        let output = std::process::Command::new(&exe)
            .output()
            .expect("run the param-borrow ownership probe");
        assert!(
            output.status.success(),
            "run {run}: {}\n{}",
            common::exit_description(&output.status),
            String::from_utf8_lossy(&output.stdout)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            expected,
            "run {run}: a param-borrow result changed — the binding freed a block \
             the caller still owns, or the wrapper lost its payload"
        );
    }
    let _ = std::fs::remove_dir_all(&project);
}

// ---------------------------------------------------------------- bug-567

/// bug-567: `RETURN <nested concat>`. `"<" & s & ">"` is
/// `Binary{ Binary{ "<", s }, ">" }`, so the inner concat's block is an INTERIOR
/// temp — not the value that leaves — and the `RETURN`'s
/// `clear_pending_temps_to` truncated it away unfreed. Two
/// `_mfb_rt_string_concat` calls in the callee, zero frees: 64 B per call,
/// 13.3 MB at 200k and 25.6 MB at 400k on the pre-fix compiler.
const SHAPE_567_NESTED_CONCAT: &str = "IMPORT io\n\
FUNC wrap(s AS String) AS String\n  RETURN \"<\" & s & \">\"\nEND FUNC\n\
SUB main()\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    acc = acc + len(wrap(\"ab\"))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// The same defect where the interior temp is a CALL result rather than a nested
/// concat — the bug report's own second row, and the commonest real body shape.
/// 13.3 MB at 200k, 25.6 MB at 400k.
const SHAPE_567_CONCAT_OF_A_CALL: &str = "IMPORT io\n\
FUNC f(n AS Integer) AS String\n  RETURN \"v\" & toString(n MOD 10)\nEND FUNC\n\
SUB main()\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    acc = acc + len(f(i))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// The bug report's CONTROL, and the row that makes the other two attributable:
/// one operator fewer, so both operands are already-owned values and the concat
/// registers exactly one temp — the one that leaves. Flat at 1.0 MB before the
/// fix and after. A POSITIVE pin: the fix ADDS frees, so a shape with nothing
/// interior must gain none.
const SHAPE_567_CONTRAST_TWO_OPERANDS: &str = "IMPORT io\n\
FUNC tail(s AS String) AS String\n  RETURN s & \">\"\nEND FUNC\n\
SUB main()\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    acc = acc + len(tail(\"ab\"))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// `RETURN <collection literal built from interior concats>` — the interior temps
/// are the two element `String`s, and the value that leaves is the `List` block.
/// A different escaping type on the same `RETURN` path.
const SHAPE_567_RETURNED_LIST_OF_CONCATS: &str = "IMPORT io\n\
IMPORT strings\n\
FUNC pair(n AS Integer) AS List OF String\n\
  RETURN [\"a\" & toString(n MOD 10), \"b\" & toString(n MOD 10)]\n\
END FUNC\n\
SUB main()\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    acc = acc + len(strings::join(pair(i), \",\"))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// The cleanup-bearing `RETURN` path: the function owns a local, so
/// `active_cleanups` is NOT empty and the return goes through
/// `store_pending_success_result` + `emit_cleanup_sequence` instead of the
/// register fast path. Same interior temp, different placement for its free.
const SHAPE_567_INTERIOR_TEMP_WITH_A_LIVE_LOCAL: &str = "IMPORT io\n\
FUNC decorate(n AS Integer) AS String\n\
  LET tag AS String = \"t\" & toString(n MOD 7)\n\
  RETURN tag & (\"-\" & toString(n MOD 10))\n\
END FUNC\n\
SUB main()\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    acc = acc + len(decorate(i))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// `FAIL error(7, <concat with a 4 KB interior temp>)`, whose `TransferTemps`
/// class is `AdoptedByTheCatcher` and which therefore keeps truncating. The block
/// is deliberately large: abandoning it would be 40 MB at 10 000 trapped errors,
/// where the whole program now runs at 1.0 MB. See the pin below, and its
/// contrast.
const SHAPE_567_FAIL_WITH_AN_INTERIOR_TEMP: &str = "IMPORT io\n\
IMPORT strings\n\
FUNC boom(n AS Integer) AS String\n\
  FAIL error(7, strings::repeat(\"x\", 4000) & \"y\")\n\
END FUNC\n\
SUB main()\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    LET s AS String = boom(i) TRAP(e)\n\
      RECOVER \"f\"\n\
    END TRAP\n\
    acc = acc + len(s)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// The same program with the `& "y"` removed — one operator fewer, so the 4 KB
/// block IS the message and nothing is interior. The control that makes the
/// equality above attributable.
const SHAPE_567_FAIL_CONTRAST_NO_INTERIOR: &str = "IMPORT io\n\
IMPORT strings\n\
FUNC boom(n AS Integer) AS String\n\
  FAIL error(7, strings::repeat(\"x\", 4000))\n\
END FUNC\n\
SUB main()\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    LET s AS String = boom(i) TRAP(e)\n\
      RECOVER \"f\"\n\
    END TRAP\n\
    acc = acc + len(s)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

#[cfg(unix)]
#[test]
fn returning_a_nested_concat_runs_at_constant_rss() {
    assert_flat(
        "b567_nested_concat",
        SHAPE_567_NESTED_CONCAT,
        200_000,
        400_000,
    );
}

#[cfg(unix)]
#[test]
fn returning_a_concat_of_a_call_result_runs_at_constant_rss() {
    assert_flat(
        "b567_concat_of_a_call",
        SHAPE_567_CONCAT_OF_A_CALL,
        200_000,
        400_000,
    );
}

/// The attributing control: no interior temp, flat before and after.
#[cfg(unix)]
#[test]
fn returning_a_two_operand_concat_still_runs_at_constant_rss() {
    assert_flat(
        "b567_contrast_two_operands",
        SHAPE_567_CONTRAST_TWO_OPERANDS,
        200_000,
        400_000,
    );
}

#[cfg(unix)]
#[test]
fn returning_a_collection_built_from_interior_concats_runs_at_constant_rss() {
    assert_flat(
        "b567_returned_list",
        SHAPE_567_RETURNED_LIST_OF_CONCATS,
        200_000,
        400_000,
    );
}

/// The other `RETURN` lowering path — the one with live cleanups, where the
/// escaping value is parked in `pending_result_slots.value` rather than held in a
/// register across the frees.
#[cfg(unix)]
#[test]
fn returning_an_interior_temp_beside_a_live_local_runs_at_constant_rss() {
    assert_flat(
        "b567_interior_with_local",
        SHAPE_567_INTERIOR_TEMP_WITH_A_LIVE_LOCAL,
        200_000,
        400_000,
    );
}

/// `Fail` is classified `AdoptedByTheCatcher` and must keep truncating: the
/// `Error` block it registers is parked in the per-thread current-error slot for
/// the CATCHER to free (`emit_direct_error_return`, design "b"), so a free here is
/// a double free, not a leak fix.
///
/// bug-567's report predicted a residual interior leak on this path. It does not
/// exist: `error(...)`'s own constructor lowering already frees the message temps
/// before the branch. This is that re-derivation as a pin, in two halves.
///
/// The FLAT half says nothing is abandoned: with bug-565 landed the whole shape
/// runs at 1.0 MB, and an abandoned 4 KB interior block would be 40 MB at 10 000
/// trapped errors.
///
/// The EQUALITY half is what attributes that to the interior temp rather than to
/// bug-565: the same program with `& "y"` removed — one operator fewer, so the
/// 4 KB block IS the message and nothing is interior — reads the same.
///
/// Flatness alone could not say the block was "correctly declined" rather than
/// "wrongly freed"; that half is
/// `codegen_return_interior_temp_drop::a_fail_is_never_given_an_interior_free`,
/// which reads the decision off the emitted code, and the value probe below,
/// which would surface a double free of the adopted `Error` as a wrong message.
#[cfg(unix)]
#[test]
fn a_failing_trap_does_not_leak_its_interior_temp() {
    assert_flat(
        "b567_fail_interior",
        SHAPE_567_FAIL_WITH_AN_INTERIOR_TEMP,
        5_000,
        10_000,
    );
    let interior = peak_rss(
        "b567_fail_interior",
        SHAPE_567_FAIL_WITH_AN_INTERIOR_TEMP,
        10_000,
    );
    let contrast = peak_rss(
        "b567_fail_contrast",
        SHAPE_567_FAIL_CONTRAST_NO_INTERIOR,
        10_000,
    );
    let delta = interior.abs_diff(contrast);
    assert!(
        delta < 4 * 1024 * 1024,
        "the interior-temp `FAIL` and its no-interior contrast diverged by {} MB \
         ({} MB vs {} MB) at 10 000 trapped errors. More on the interior side \
         means the 4 KB block is being abandoned at the `Fail`; less means \
         something started freeing on this path, and the block a `Fail` registers \
         is the `Error` the catcher ADOPTS",
        delta / (1024 * 1024),
        interior / (1024 * 1024),
        contrast / (1024 * 1024),
    );
}

/// The VALUE half. Freeing a block the caller still owns is a use-after-free that
/// surfaces as a wrong string, not as a failing free — so every `RETURN` shape the
/// new interior drop touches is read back, 25 times, against an expectation
/// computed here.
#[test]
fn every_returned_concat_shape_still_produces_the_right_value() {
    const SOURCE: &str = "IMPORT io\n\
IMPORT strings\n\
FUNC wrap(s AS String) AS String\n  RETURN \"<\" & s & \">\"\nEND FUNC\n\
FUNC tail(s AS String) AS String\n  RETURN s & \">\"\nEND FUNC\n\
FUNC v(n AS Integer) AS String\n  RETURN \"v\" & toString(n MOD 10)\nEND FUNC\n\
FUNC three(a AS String) AS String\n  RETURN \"[\" & wrap(a) & \"]\" & toString(len(a))\nEND FUNC\n\
FUNC pair(n AS Integer) AS List OF String\n  RETURN [\"a\" & toString(n), \"b\" & toString(n)]\nEND FUNC\n\
FUNC decorate(n AS Integer) AS String\n\
  LET tag AS String = \"t\" & toString(n)\n\
  RETURN tag & (\"-\" & toString(n + 1))\n\
END FUNC\n\
FUNC guarded(n AS Integer) AS String\n\
  IF n > 0 THEN\n\
    RETURN \"pos:\" & toString(n) & \"!\" & strings::upper(\"x\" & toString(n))\n\
  END IF\n\
  RETURN \"neg\"\n\
END FUNC\n\
FUNC boom(n AS Integer) AS String\n  FAIL error(7, \"x\" & toString(n))\nEND FUNC\n\
SUB main()\n\
  io::print(wrap(\"ab\"))\n\
  io::print(tail(\"q\"))\n\
  io::print(v(37))\n\
  io::print(three(\"zz\"))\n\
  io::print(strings::join(pair(4), \",\"))\n\
  io::print(decorate(9))\n\
  io::print(guarded(3))\n\
  io::print(guarded(0))\n\
  LET caught AS String = boom(5) TRAP(e)\n\
    RECOVER \"caught:\" & e.message\n\
  END TRAP\n\
  io::print(caught)\n\
  MUT i AS Integer = 0\n\
  MUT acc AS String = \"\"\n\
  WHILE i < 5\n\
    acc = acc & wrap(toString(i)) & three(toString(i))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(acc)\n\
END SUB\n";

    // Computed here, not read off the program.
    let wrap = |s: &str| format!("<{s}>");
    let three = |a: &str| format!("[{}]{}", wrap(a), a.len());
    let mut acc = String::new();
    for i in 0..5 {
        acc.push_str(&wrap(&i.to_string()));
        acc.push_str(&three(&i.to_string()));
    }
    let expected = [
        wrap("ab"),
        "q>".to_string(),
        "v7".to_string(),
        three("zz"),
        "a4,b4".to_string(),
        "t9-10".to_string(),
        "pos:3!X3".to_string(),
        "neg".to_string(),
        "caught:x5".to_string(),
        acc,
    ]
    .join("\n");

    let project = common::temp_project("b567_return_values", SOURCE);
    let exe = common::build_project(&project);
    // A use-after-free is not deterministic: it depends on whether the arena
    // reuses the block before the read.
    for run in 1..=25 {
        let output = std::process::Command::new(&exe)
            .output()
            .expect("run the returned-concat ownership probe");
        assert!(
            output.status.success(),
            "run {run}: {}\n{}",
            common::exit_description(&output.status),
            String::from_utf8_lossy(&output.stdout)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            expected,
            "run {run}: a returned string read back wrong — an interior free took \
             a block the caller still owned"
        );
    }
    let _ = std::fs::remove_dir_all(&project);
}

// ---------------------------------------------------------------- bug-566

/// Assert `trapped` grows no faster with its iteration count than `plain` does.
///
/// bug-566's shapes cannot use `assert_flat`: a runtime-helper call with a
/// `String` ARGUMENT leaks ~128 B per call on this compiler whether or not a
/// `TRAP` is anywhere near it (`fs::exists(path)` alone, no `TRAP`, no block
/// result, grows 3.6 -> 6.2 MB at 20k/40k). That is a different defect, filed
/// separately, and it would swamp a flatness assertion here.
///
/// So the pin is COMPARATIVE and it isolates exactly what bug-566 changed: the
/// same call, once under an inline `TRAP` and once not. Before the fix the
/// trapped form grew twice as fast (5.2 MB vs 2.6 MB over the same 20k extra
/// iterations); after it, the two growths match. When the argument leak is fixed
/// both sides go flat and this still holds.
#[cfg(unix)]
fn assert_no_extra_growth(name: &str, trapped: &str, plain: &str, small: u64, large: u64) {
    let trapped_growth =
        peak_rss(name, trapped, large).saturating_sub(peak_rss(name, trapped, small));
    let plain_growth = peak_rss(&format!("{name}_plain"), plain, large).saturating_sub(peak_rss(
        &format!("{name}_plain"),
        plain,
        small,
    ));
    assert!(
        trapped_growth <= plain_growth + 2 * 1024 * 1024,
        "{name}: under an inline `TRAP` the loop grew {} MB between {small} and \
         {large} iterations, but the SAME call bound without a `TRAP` grew only \
         {} MB. The `TRAP` lowering copies the helper's block into the `Result` \
         and must free the original (bug-566)",
        trapped_growth / (1024 * 1024),
        plain_growth / (1024 * 1024),
    );
    // bug-574: these three cases were COMPARATIVE when bug-566 landed, because
    // the plain call they compare against was itself leaking — the marshalled
    // path argument, ~129 B per call for `"b566_probe.txt"`. With that released,
    // both halves are flat, and flatness is the stronger statement: comparative
    // growth cannot tell "both fixed" from "both leaking equally".
    assert!(
        trapped_growth < 8 * 1024 * 1024 && plain_growth < 8 * 1024 * 1024,
        "{name}: the loop grew {} MB trapped / {} MB plain between {small} and \
         {large} iterations. Both forms must now be FLAT: the helper owns the \
         block it returned (bug-566) AND releases the argument it marshalled \
         (bug-574)",
        trapped_growth / (1024 * 1024),
        plain_growth / (1024 * 1024),
    );
}

/// bug-566: `fs::readText` under an inline `TRAP`. The helper allocates the
/// `String` in this thread's arena; `materialize_current_result` copies it into an
/// intermediate, copies that into the `Result`, frees the intermediate (bug-379)
/// and abandoned the original. 6.2 -> 11.4 MB at 20k/40k before, against
/// 3.6 -> 6.2 MB for the same call bound plainly.
const SHAPE_566_TRAPPED_READ_TEXT: &str = "IMPORT io\n\
IMPORT fs\n\
SUB main()\n\
  fs::writeText(\"b566_probe.txt\", \"hello world!\")\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    LET s AS String = fs::readText(\"b566_probe.txt\") TRAP(e)\n\
      RECOVER \"x\"\n\
    END TRAP\n\
    acc = acc + len(s)\n\
    i = i + 1\n\
  END WHILE\n\
  fs::deleteFile(\"b566_probe.txt\")\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// The same call with the `TRAP` removed — the contrast that attributes the extra
/// growth to the `Result` lowering and not to `fs::readText` itself.
const SHAPE_566_PLAIN_READ_TEXT: &str = "IMPORT io\n\
IMPORT fs\n\
SUB main()\n\
  fs::writeText(\"b566_probe.txt\", \"hello world!\")\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    LET s AS String = fs::readText(\"b566_probe.txt\")\n\
    acc = acc + len(s)\n\
    i = i + 1\n\
  END WHILE\n\
  fs::deleteFile(\"b566_probe.txt\")\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// A COLLECTION payload on the same path — `List OF Byte`, whose block is freed
/// by the collection drop rather than the `String` one.
const SHAPE_566_TRAPPED_READ_BYTES: &str = "IMPORT io\n\
IMPORT fs\n\
SUB main()\n\
  fs::writeText(\"b566_probe_b.txt\", \"hello world!\")\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    LET b AS List OF Byte = fs::readBytes(\"b566_probe_b.txt\") TRAP(e)\n\
      RECOVER []\n\
    END TRAP\n\
    acc = acc + len(b)\n\
    i = i + 1\n\
  END WHILE\n\
  fs::deleteFile(\"b566_probe_b.txt\")\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

const SHAPE_566_PLAIN_READ_BYTES: &str = "IMPORT io\n\
IMPORT fs\n\
SUB main()\n\
  fs::writeText(\"b566_probe_b.txt\", \"hello world!\")\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    LET b AS List OF Byte = fs::readBytes(\"b566_probe_b.txt\")\n\
    acc = acc + len(b)\n\
    i = i + 1\n\
  END WHILE\n\
  fs::deleteFile(\"b566_probe_b.txt\")\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// A SCALAR payload under the same `TRAP` machinery: `fs::exists` returns a
/// `Boolean`, so `result_payload_is_block` is false and bug-566 emits nothing at
/// all. Its growth is the shared argument leak, and it is identical before and
/// after — the control that says the `TRAP` lowering itself is not what changed.
const SHAPE_566_TRAPPED_SCALAR: &str = "IMPORT io\n\
IMPORT fs\n\
SUB main()\n\
  fs::writeText(\"b566_probe_s.txt\", \"hello world!\")\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    LET b AS Boolean = fs::exists(\"b566_probe_s.txt\") TRAP(e)\n\
      RECOVER FALSE\n\
    END TRAP\n\
    IF b THEN\n\
      acc = acc + 1\n\
    END IF\n\
    i = i + 1\n\
  END WHILE\n\
  fs::deleteFile(\"b566_probe_s.txt\")\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

const SHAPE_566_PLAIN_SCALAR: &str = "IMPORT io\n\
IMPORT fs\n\
SUB main()\n\
  fs::writeText(\"b566_probe_s.txt\", \"hello world!\")\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    LET b AS Boolean = fs::exists(\"b566_probe_s.txt\")\n\
    IF b THEN\n\
      acc = acc + 1\n\
    END IF\n\
    i = i + 1\n\
  END WHILE\n\
  fs::deleteFile(\"b566_probe_s.txt\")\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

#[cfg(unix)]
#[test]
fn a_trapped_runtime_helper_string_result_grows_no_faster_than_the_plain_call() {
    assert_no_extra_growth(
        "b566_read_text",
        SHAPE_566_TRAPPED_READ_TEXT,
        SHAPE_566_PLAIN_READ_TEXT,
        20_000,
        40_000,
    );
}

#[cfg(unix)]
#[test]
fn a_trapped_runtime_helper_collection_result_grows_no_faster_than_the_plain_call() {
    assert_no_extra_growth(
        "b566_read_bytes",
        SHAPE_566_TRAPPED_READ_BYTES,
        SHAPE_566_PLAIN_READ_BYTES,
        20_000,
        40_000,
    );
}

/// The scalar control. It held before the fix too — which is the point: the extra
/// growth the two cases above measured was the PAYLOAD BLOCK, not the `TRAP`.
#[cfg(unix)]
#[test]
fn a_trapped_runtime_helper_scalar_result_was_never_the_leak() {
    assert_no_extra_growth(
        "b566_scalar",
        SHAPE_566_TRAPPED_SCALAR,
        SHAPE_566_PLAIN_SCALAR,
        20_000,
        40_000,
    );
}

/// The VALUE half, and the half that matters most here: freeing a block this
/// thread does not own is not a leak fix but memory corruption, and it shows up as
/// a wrong value or an unrelated allocation failure rather than as a failing free.
///
/// `thread::waitFor` is the counter-example the whole audit turns on — its result
/// was allocated in the WORKER's arena — so it is exercised beside the helpers that
/// ARE freed, 25 times, with the values computed here.
#[test]
fn every_trapped_runtime_helper_result_still_produces_the_right_value() {
    const SOURCE: &str = "IMPORT io\n\
IMPORT fs\n\
IMPORT os\n\
IMPORT thread\n\
ISOLATED FUNC worker(w AS ThreadWorker OF String TO String, seed AS String) AS String\n\
  RETURN seed & \"-done\"\n\
END FUNC\n\
SUB main()\n\
  fs::writeText(\"b566_values.txt\", \"hello world!\")\n\
  LET s AS String = fs::readText(\"b566_values.txt\") TRAP(e)\n\
    RECOVER \"x\"\n\
  END TRAP\n\
  io::print(s)\n\
  LET b AS List OF Byte = fs::readBytes(\"b566_values.txt\") TRAP(e)\n\
    RECOVER []\n\
  END TRAP\n\
  io::print(toString(len(b)))\n\
  LET missing AS String = fs::readText(\"b566_absent_file.txt\") TRAP(e)\n\
    RECOVER \"recovered\"\n\
  END TRAP\n\
  io::print(missing)\n\
  LET names AS List OF String = fs::listDirectory(\".\") TRAP(e)\n\
    RECOVER []\n\
  END TRAP\n\
  io::print(toString(len(names) > 0))\n\
  LET env AS String = os::getEnvOr(\"B566_NOT_SET\", \"fallback\") TRAP(e)\n\
    RECOVER \"x\"\n\
  END TRAP\n\
  io::print(env)\n\
  LET t AS Thread OF String TO String = thread::start(worker, \"worker\")\n\
  LET out AS String = thread::waitFor(t) TRAP(e)\n\
    RECOVER \"thread-failed\"\n\
  END TRAP\n\
  io::print(out)\n\
  io::print(s & \"/\" & missing & \"/\" & env)\n\
  fs::deleteFile(\"b566_values.txt\")\n\
END SUB\n";

    let expected = [
        "hello world!",
        "12",
        "recovered",
        "TRUE",
        "fallback",
        "worker-done",
        "hello world!/recovered/fallback",
    ]
    .join("\n");

    let project = common::temp_project("b566_values", SOURCE);
    let exe = common::build_project(&project);
    // A cross-arena free is not deterministic: it depends on whether the other
    // arena reuses the block before the read.
    for run in 1..=25 {
        let output = std::process::Command::new(&exe)
            .current_dir(&project)
            .output()
            .expect("run the trapped-runtime-helper ownership probe");
        assert!(
            output.status.success(),
            "run {run}: {}\n{}",
            common::exit_description(&output.status),
            String::from_utf8_lossy(&output.stdout)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            expected,
            "run {run}: a trapped runtime-helper result read back wrong. If it is \
             the `thread::waitFor` line, the free reached a block the WORKER's \
             arena owns — `x19` is per-thread"
        );
    }
    let _ = std::fs::remove_dir_all(&project);
}

// ---------------------------------------------------------------- bug-574
//
// Every `fs::`/`os::`/`net::`/`udp::`/`tcp::` call copies its `String` argument
// into a fresh NUL-terminated arena block for the host call. That block is
// interior to the helper — not a `ValueResult` any node yielded — so no
// caller-side ownership analysis could reach it and nothing freed it.
//
// The measurement that identifies it is the LENGTH SCALING, not the absolute
// growth: at 20 000 iterations a 10-character path cost ~65 B per call and a
// 415-character path ~1 819 B, while a zero-argument helper was flat. So the two
// cases below are the same call with the same result type and the same
// iteration counts, differing only in how long the argument is — the long one is
// what a chunk-growth threshold cannot absorb.

/// `fs::exists` with a 10-character path. `Boolean` result, no `TRAP`, no block
/// anywhere in the shape: whatever grows here is the ARGUMENT.
const SHAPE_574_SHORT_PATH: &str = "IMPORT io\n\
IMPORT fs\n\
SUB main()\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    IF fs::exists(\"/tmp/x.txt\") THEN\n\
      n = n + 1\n\
    END IF\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END SUB\n";

/// The same call with a 415-character path — 28x the leak per call, and the
/// sensitive form. A fix that freed the wrong SIZE (bug-560's shape) or freed on
/// only one exit path still reads as growth here.
const SHAPE_574_LONG_PATH: &str = concat!(
    "IMPORT io\n",
    "IMPORT fs\n",
    "SUB main()\n",
    "  MUT n AS Integer = 0\n",
    "  MUT i AS Integer = 0\n",
    "  WHILE i < {N}\n",
    "    IF fs::exists(\"/tmp/b574probe/",
    "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
    "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
    "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
    "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
    "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
    "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
    ".txt\") THEN\n",
    "      n = n + 1\n",
    "    END IF\n",
    "    i = i + 1\n",
    "  END WHILE\n",
    "  io::print(\"n=\" & toString(n))\n",
    "END SUB\n",
);

/// The same shape through a LOCAL rather than a literal. The report's own third
/// row: hoisting the path out of the loop changed nothing, which is what ruled
/// out a per-iteration copy of the rodata constant and left the marshalling.
const SHAPE_574_LOCAL_PATH: &str = concat!(
    "IMPORT io\n",
    "IMPORT fs\n",
    "SUB main()\n",
    "  LET p AS String = \"/tmp/b574probe/",
    "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
    "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
    "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
    "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
    "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
    "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
    ".txt\"\n",
    "  MUT n AS Integer = 0\n",
    "  MUT i AS Integer = 0\n",
    "  WHILE i < {N}\n",
    "    IF fs::exists(p) THEN\n",
    "      n = n + 1\n",
    "    END IF\n",
    "    i = i + 1\n",
    "  END WHILE\n",
    "  io::print(\"n=\" & toString(n))\n",
    "END SUB\n",
);

/// `os::getEnvOr` with a 400-character variable name that is never set: the
/// `os` family marshals through a different emitter (`marshal_cstring`), and its
/// result is a fresh `String` the binding already owned — so this case separates
/// "the argument is freed" from "the result is freed".
const SHAPE_574_ENV_NAME: &str = concat!(
    "IMPORT io\n",
    "IMPORT os\n",
    "SUB main()\n",
    "  MUT n AS Integer = 0\n",
    "  MUT i AS Integer = 0\n",
    "  WHILE i < {N}\n",
    "    LET v AS String = os::getEnvOr(\"MFB_B574_",
    "EEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEE",
    "EEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEE",
    "EEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEE",
    "EEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEE",
    "EEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEE\", \"d\")\n",
    "    n = n + len(v)\n",
    "    i = i + 1\n",
    "  END WHILE\n",
    "  io::print(\"n=\" & toString(n))\n",
    "END SUB\n",
);

/// `fs::isWithin` marshals TWO paths and two PATH_MAX `realpath` buffers, and
/// returns a `Boolean` — four scratch blocks and nothing to hand back. The
/// multi-scratch case: a release that covered only the first would still grow.
const SHAPE_574_TWO_ARGUMENTS: &str = "IMPORT io\n\
IMPORT fs\n\
SUB main()\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET b AS Boolean = fs::isWithin(\"/tmp\", \"/tmp\") TRAP(e)\n\
      RECOVER FALSE\n\
    END TRAP\n\
    IF b THEN\n\
      n = n + 1\n\
    END IF\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END SUB\n";

/// `fs::currentDirectory` takes NO argument and still allocated a 4 KiB `getcwd`
/// buffer it never freed — 16 384 B per call, the largest row in the family. It
/// is the case that shows the report's framing — "argument marshalling" — was too
/// narrow: the defect is a fixed runtime helper allocating a block for its own
/// use, whether or not an argument motivated it.
///
/// The result is BOUND deliberately. Left unbound (`len(fs::currentDirectory())`)
/// the same loop still grows ~193 B per call, and it does so identically before
/// and after this change: an UNBOUND runtime-helper `String` result has no owner
/// at all — `os::hostName()` leaks 128 B per call unbound and 0 bound, on both
/// binaries. That is a different defect and is not fixed here; binding keeps this
/// case measuring the scratch buffer alone.
const SHAPE_574_NO_ARGUMENT: &str = "IMPORT io\n\
IMPORT fs\n\
SUB main()\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET here AS String = fs::currentDirectory()\n\
    n = n + len(here)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END SUB\n";

/// The POSITIVE pin: `os::arch()` allocates its result and nothing else, and was
/// flat before this change. It must stay flat — a release that reached the
/// RESULT block would show up here as a wrong value or an allocation failure,
/// not as growth, which is why the value assertions below matter as much.
const SHAPE_574_CONTRAST_ARCH: &str = "IMPORT io\n\
IMPORT os\n\
SUB main()\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET a AS String = os::arch()\n\
    n = n + len(a)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END SUB\n";

#[cfg(unix)]
#[test]
fn a_short_path_argument_runs_at_constant_rss() {
    assert_flat("b574_short_path", SHAPE_574_SHORT_PATH, 20_000, 40_000);
}

#[cfg(unix)]
#[test]
fn a_long_path_argument_runs_at_constant_rss() {
    assert_flat("b574_long_path", SHAPE_574_LONG_PATH, 20_000, 40_000);
}

#[cfg(unix)]
#[test]
fn a_long_path_argument_in_a_local_runs_at_constant_rss() {
    assert_flat("b574_local_path", SHAPE_574_LOCAL_PATH, 20_000, 40_000);
}

#[cfg(unix)]
#[test]
fn a_long_environment_name_runs_at_constant_rss() {
    assert_flat("b574_env_name", SHAPE_574_ENV_NAME, 20_000, 40_000);
}

#[cfg(unix)]
#[test]
fn a_call_marshalling_two_arguments_runs_at_constant_rss() {
    assert_flat("b574_two_args", SHAPE_574_TWO_ARGUMENTS, 20_000, 40_000);
}

#[cfg(unix)]
#[test]
fn a_helper_with_no_argument_but_its_own_buffer_runs_at_constant_rss() {
    assert_flat("b574_no_argument", SHAPE_574_NO_ARGUMENT, 20_000, 40_000);
}

#[cfg(unix)]
#[test]
fn a_helper_that_allocates_only_its_result_stays_flat() {
    assert_flat("b574_arch", SHAPE_574_CONTRAST_ARCH, 20_000, 40_000);
}

// ---------------------------------------------------------------- bug-575
//
// `tls` has its own copy of the marshaller (`builtins/tls/gen_shared.rs`), which
// bug-574 did not convert: all twelve of its call sites allocated a NUL-terminated
// arena copy of a host name or a PEM path for the backend call and freed none of
// them. Same defect, same consequence — every `tls::connect` and `tls::listen`
// leaked its host name, in proportion to that name's LENGTH.
//
// ## Why this case is a length comparison and not `assert_flat`
//
// `assert_flat` cannot be used here, and the reason is worth writing down: a
// failing `tls::connect` leaks something else as well. Measured on this shape at
// 20 000/40 000 iterations with a SHORT host, on the already-fixed binary, the
// loop still grows — ~260 B per call on Linux and ~1.9 KB per call on macOS. That
// residue is neither bug-575 nor specific to `tls`: `tcp::connect` in the same
// trapped-failure shape, whose marshalling bug-574 already fixed, grows by the
// same ~260 B per call on the same Linux box. It is the trapped-error path, and it
// is length-INDEPENDENT — identical at a 1 613-character host and a 6 413-character
// one.
//
// So the assertion is the one the bug is actually about: peak RSS must not scale
// with the ARGUMENT's length. Two runs at the same iteration count, differing only
// in how long the host name is, and the difference between them must be noise. The
// length-independent residue is present in both and cancels; the marshalling leak
// does not cancel, because it is 4x larger in the long run.
//
// ## Calibration (measured, `mfb` built from this tree vs. from its parent)
//
// | host chars | 20 000 iterations, peak RSS | before | after |
// | --- | --- | --- | --- |
// | 1 613 | macOS aarch64 (Network.framework) | 173.8 MB | 44.8 MB |
// | 6 413 | macOS aarch64 (Network.framework) | 333.3 MB | 43.5 MB |
// | 1 613 | Linux x86_64 musl (OpenSSL) | 40.4 MB | 8.4 MB |
// | 6 413 | Linux x86_64 musl (OpenSSL) | 160.3 MB | 5.5 MB |
//
// The difference this test measures is therefore +159.5 MB / +119.9 MB before and
// NEGATIVE after (the longer run is the cheaper one once the block is released and
// the arena can reuse it). The threshold is 32 MB: a quarter of the smaller failing
// reading, and ten times the largest passing spread seen.
//
// 20 000 rather than the 200 000 the other cases use, because the leak here is
// ~6 KB per call rather than tens of bytes — the separation at 20 000 is already
// 4x the threshold, and 200 000 would allocate 1.2 GB before failing.
//
// ## What the host name is
//
// A name far past the 253-byte DNS limit, so every resolver rejects it without a
// query — no network, no DNS timeout, no dependence on what the machine's resolver
// does with an unknown name. Both backends marshal the host BEFORE resolving it,
// which is precisely why the leak is reachable through a failing call at all.
//
// Windows is excluded with the rest of the RSS half (`ru_maxrss` has no equivalent
// there), so the Schannel copies of these sites are pinned only by the
// codegen-inspection table in `tests/codegen/codegen_helper_scratch_release.rs`.

/// A loop of `{N}` failing `tls::connect` calls with a host name of
/// `host_chars` characters.
#[cfg(unix)]
fn tls_connect_probe(host_chars: usize) -> String {
    let host = format!("b575-{}.invalid", "z".repeat(host_chars.saturating_sub(13)));
    format!(
        "IMPORT io\n\
         IMPORT tls\n\
         SUB main()\n\
        \x20 MUT n AS Integer = 0\n\
        \x20 MUT i AS Integer = 0\n\
        \x20 WHILE i < {{N}}\n\
        \x20   RES s AS tls::Socket = tls::connect(\"{host}\", 443, 0) TRAP(e)\n\
        \x20     n = n + 1\n\
        \x20     i = i + 1\n\
        \x20     CONTINUE WHILE\n\
        \x20   END TRAP\n\
        \x20   tls::close(s)\n\
        \x20   i = i + 1\n\
        \x20 END WHILE\n\
        \x20 io::print(\"n=\" & toString(n))\n\
         END SUB\n"
    )
}

#[cfg(unix)]
#[test]
fn a_tls_connect_host_name_is_not_leaked_in_proportion_to_its_length() {
    const N: u64 = 20_000;
    let short = peak_rss("b575_tls_host_short", &tls_connect_probe(1_613), N);
    let long = peak_rss("b575_tls_host_long", &tls_connect_probe(6_413), N);
    let grew = long.saturating_sub(short);
    assert!(
        grew < 32 * 1024 * 1024,
        "a {N}-iteration `tls::connect` loop cost {} MB more with a 6 413-character \
         host name than with a 1 613-character one ({} MB -> {} MB). The host is \
         copied into an arena C-string for the backend call and that copy is the \
         helper's own scratch — nothing on the caller side can free it, so every \
         call leaks the name (bug-575)",
        grew / (1024 * 1024),
        short / (1024 * 1024),
        long / (1024 * 1024),
    );
}

/// The VALUE half. A scratch release that reached the block a helper HANDS BACK
/// is a use-after-free the caller performs, and it surfaces as a wrong value or a
/// later unrelated allocation failure — never as a failing free. So every member
/// whose emitter this change touched is read back here, in one process, after a
/// warm-up loop that guarantees the arena has recycled the freed scratch into
/// later allocations: if a released block were still live, the reuse would
/// corrupt it before these lines print.
const SHAPE_574_VALUES: &str = "IMPORT io\n\
IMPORT fs\n\
IMPORT os\n\
IMPORT net\n\
IMPORT strings\n\
SUB main()\n\
  LET d AS String = fs::tempDirectory() & \"/b574_values\"\n\
  fs::createDirectories(d) TRAP(e1)\n\
    RECOVER\n\
  END TRAP\n\
  MUT i AS Integer = 0\n\
  WHILE i < 500\n\
    LET warm AS Boolean = fs::exists(d)\n\
    IF warm THEN\n\
      i = i + 1\n\
    ELSE\n\
      i = i + 1\n\
    END IF\n\
  END WHILE\n\
  fs::writeText(d & \"/a.txt\", \"line one\\nline two\\n\") TRAP(e2)\n\
    RECOVER\n\
  END TRAP\n\
  LET text AS String = fs::readText(d & \"/a.txt\") TRAP(e3)\n\
    RECOVER \"READ FAILED\"\n\
  END TRAP\n\
  io::print(\"text=\" & text)\n\
  LET raw AS List OF Byte = fs::readBytes(d & \"/a.txt\") TRAP(e4)\n\
    RECOVER []\n\
  END TRAP\n\
  io::print(\"bytes=\" & toString(len(raw)))\n\
  io::print(\"exists=\" & toString(fs::exists(d & \"/a.txt\")))\n\
  io::print(\"file=\" & toString(fs::fileExists(d & \"/a.txt\")))\n\
  io::print(\"dir=\" & toString(fs::directoryExists(d)))\n\
  io::print(\"within=\" & toString(fs::isWithin(d, d & \"/a.txt\")))\n\
  LET names AS List OF String = fs::listDirectory(d) TRAP(e5)\n\
    RECOVER []\n\
  END TRAP\n\
  io::print(\"entries=\" & toString(len(names)))\n\
  LET canon AS String = fs::canonicalPath(d & \"/a.txt\") TRAP(e6)\n\
    RECOVER \"CANON FAILED\"\n\
  END TRAP\n\
  io::print(\"canonEndsWithName=\" & toString(strings::endsWith(canon, \"a.txt\")))\n\
  io::print(\"cwd=\" & toString(len(fs::currentDirectory()) > 0))\n\
  io::print(\"tmp=\" & toString(len(fs::tempDirectory()) > 0))\n\
  RES handle AS fs::File = fs::openFile(d & \"/a.txt\", \"r\") TRAP(e7)\n\
    PROPAGATE\n\
  END TRAP\n\
  LET first AS String = fs::readLine(handle) TRAP(e8)\n\
    RECOVER \"LINE FAILED\"\n\
  END TRAP\n\
  io::print(\"line=\" & first)\n\
  fs::close(handle) TRAP(e9)\n\
    RECOVER\n\
  END TRAP\n\
  os::setEnv(\"MFB_B574_VALUE\", \"present\") TRAP(e10)\n\
    RECOVER\n\
  END TRAP\n\
  io::print(\"has=\" & toString(os::hasEnv(\"MFB_B574_VALUE\")))\n\
  io::print(\"env=\" & os::getEnvOr(\"MFB_B574_VALUE\", \"absent\"))\n\
  os::unsetEnv(\"MFB_B574_VALUE\") TRAP(e11)\n\
    RECOVER\n\
  END TRAP\n\
  io::print(\"has2=\" & toString(os::hasEnv(\"MFB_B574_VALUE\")))\n\
  io::print(\"arch=\" & toString(len(os::arch()) > 0))\n\
  LET found AS List OF net::Address = net::lookup(\"localhost\", 80) TRAP(e12)\n\
    RECOVER []\n\
  END TRAP\n\
  io::print(\"lookup=\" & toString(len(found) > 0))\n\
  fs::deleteFile(d & \"/a.txt\") TRAP(e13)\n\
    RECOVER\n\
  END TRAP\n\
  io::print(\"gone=\" & toString(fs::exists(d & \"/a.txt\")))\n\
END SUB\n";

#[test]
fn every_helper_whose_scratch_is_released_still_produces_the_right_value() {
    let project = common::temp_project("b574_values", SHAPE_574_VALUES);
    let exe = common::build_project(&project);
    let expected = "text=line one\nline two\n\
                    \nbytes=18\n\
                    exists=TRUE\n\
                    file=TRUE\n\
                    dir=TRUE\n\
                    within=TRUE\n\
                    entries=1\n\
                    canonEndsWithName=TRUE\n\
                    cwd=TRUE\n\
                    tmp=TRUE\n\
                    line=line one\n\
                    has=TRUE\n\
                    env=present\n\
                    has2=FALSE\n\
                    arch=TRUE\n\
                    lookup=TRUE\n\
                    gone=FALSE";
    // Ten runs: a released-too-early block is only observably wrong once the
    // arena hands it to a later allocation, and which allocation that is depends
    // on the fill pattern the arena seeds per process.
    for run in 0..10 {
        let output = std::process::Command::new(&exe)
            .current_dir(&project)
            .output()
            .expect("run the helper-scratch value probe");
        assert!(
            output.status.success(),
            "run {run}: {}\n{}",
            common::exit_description(&output.status),
            String::from_utf8_lossy(&output.stdout)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            expected,
            "run {run}: a helper whose marshalling scratch is now released read \
             back wrong. The failure direction for bug-574 is a release that \
             reached the block the helper HANDS BACK, which shows up here rather \
             than as a failing free"
        );
    }
    let _ = std::fs::remove_dir_all(&project);
}

// ---------------------------------------------------------------- bug-573
//
// Every error raised through `_mfb_make_error_result` orphaned the `ErrorLoc` it
// had just built: `_mfb_rt_park_error` inlines a COPY of it into the single owned
// `Error` block it parks, and nothing owned the original afterwards, on any path.
//
// The measurement that identifies it as the `ErrorLoc` — rather than anything on
// the `TRAP` side — is that it scales with the recorded SOURCE FILENAME, which is
// inlined into that block and appears nowhere else in the shape. So the two cases
// below are the same program compiled twice, differing only in how deep its
// source file sits: `src/main.mfb` cost ~200 B per raise and a 117-character path
// ~682 B, a slope of 4.6 B per filename byte.

/// Peak RSS of `source` with `{N}` replaced by `count`, compiled at
/// `src/<subdirectory>/main.mfb` so the `ErrorLoc`'s recorded filename is as long
/// as the subdirectory makes it.
#[cfg(unix)]
fn peak_rss_at_depth(name: &str, subdirectory: &str, source: &str, count: u64) -> u64 {
    let program = source.replace("{N}", &count.to_string());
    // `common::temp_project` writes `src/main.mfb`; move the source down into the
    // nested directory (the project's include glob is `**/*.mfb`) so the recorded
    // path grows without changing anything else about the build.
    let project = common::temp_project(&format!("{name}_{count}"), "");
    let nested = project.join("src").join(subdirectory);
    std::fs::create_dir_all(&nested).expect("create nested source directory");
    std::fs::remove_file(project.join("src/main.mfb")).expect("remove the flat source");
    std::fs::write(nested.join("main.mfb"), &program).expect("write nested source");
    let exe = common::build_project(&project);
    let (status, stdout, rss) = common::run_bounded_with_rss(
        &exe,
        std::time::Duration::from_secs(300),
        "the raised-error origin probe did not finish",
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

/// Five 20-character path components: `src/<100 chars>/main.mfb` is 117
/// characters against `src/main.mfb`'s 12, so the per-raise cost of the orphaned
/// `ErrorLoc` was 3.4x the flat program's.
#[cfg(unix)]
const DEEP_SOURCE_DIRECTORY: &str = "dddddddddddddddddddd/dddddddddddddddddddd/\
                                     dddddddddddddddddddd/dddddddddddddddddddd/\
                                     dddddddddddddddddddd";

#[cfg(unix)]
#[test]
fn a_raised_error_runs_at_constant_rss_however_long_its_filename_is() {
    let a = peak_rss_at_depth(
        "b573_deep_raise",
        DEEP_SOURCE_DIRECTORY,
        B573_BUILTIN_RAISE,
        200_000,
    );
    let b = peak_rss_at_depth(
        "b573_deep_raise",
        DEEP_SOURCE_DIRECTORY,
        B573_BUILTIN_RAISE,
        400_000,
    );
    let grew = b.saturating_sub(a);
    assert!(
        grew < 8 * 1024 * 1024,
        "b573_deep_raise: peak RSS grew {} MB between 200 000 and 400 000 raised \
         errors ({} MB -> {} MB) with a 117-character source path. The orphaned \
         block is the `ErrorLoc`: its cost is the filename's length, so this is \
         the sensitive form — the flat `src/main.mfb` program leaked 200 B per \
         raise and this one 682 B (137.6 MB -> 274.1 MB before the fix)",
        grew / (1024 * 1024),
        a / (1024 * 1024),
        b / (1024 * 1024),
    );
}

/// The RAISE shape that does not involve an inline builtin at all: a user `FUNC`
/// that `FAIL`s, caught by the caller. Its `ErrorLoc` comes from the same
/// `_mfb_make_error_result` path.
const B573_USER_FAIL: &str = "IMPORT io\n\
FUNC boom(n AS Integer) AS Integer\n\
  FAIL error(90000001, \"boom\")\n\
END FUNC\n\
SUB main()\n\
  MUT acc AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET v AS Integer = boom(i) TRAP(e)\n\
      RECOVER e.code\n\
    END TRAP\n\
    acc = acc + v\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// A PROPAGATED error: raised two frames down and re-raised through a
/// `PROPAGATE`, so the middle frame hands the error on rather than raising it.
/// This is the shape the park's re-point protects — the propagation path reads
/// the origin after the raiser has parked, and now reads the parked block's own
/// copy.
const B573_PROPAGATED: &str = "IMPORT io\n\
FUNC boom(n AS Integer) AS Integer\n\
  FAIL error(90000002, \"deep\")\n\
END FUNC\n\
FUNC middle(n AS Integer) AS Integer\n\
  LET v AS Integer = boom(n) TRAP(inner)\n\
    PROPAGATE\n\
  END TRAP\n\
  RETURN v\n\
END FUNC\n\
SUB main()\n\
  MUT acc AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET v AS Integer = middle(i) TRAP(e)\n\
      RECOVER e.source.line\n\
    END TRAP\n\
    acc = acc + v\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// A RUNTIME-HELPER error, whose origin is stamped by the call site
/// (`emit_stamp_current_error_source`) rather than by `_mfb_make_error_result` —
/// the second of the three park sites, and the one whose `ErrorLoc` is built
/// directly rather than through the shared assembly.
const B573_HELPER_ERROR: &str = "IMPORT io\n\
IMPORT fs\n\
SUB main()\n\
  MUT acc AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET s AS String = fs::readText(\"/tmp/b573_absent_file.txt\") TRAP(e)\n\
      RECOVER \"x\"\n\
    END TRAP\n\
    acc = acc + len(s)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

/// The POSITIVE pin: the SAME loop with a call that never fails. It was flat
/// before and must stay flat — the change adds an `arena_free` on the error path,
/// so a shape that raises nothing must gain nothing.
const B573_CONTRAST_NO_ERROR: &str = "IMPORT io\n\
FUNC fine(n AS Integer) AS Integer\n\
  RETURN n MOD 3\n\
END FUNC\n\
SUB main()\n\
  MUT acc AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET v AS Integer = fine(i) TRAP(e)\n\
      RECOVER 0\n\
    END TRAP\n\
    acc = acc + v\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END SUB\n";

#[cfg(unix)]
#[test]
fn a_user_fail_runs_at_constant_rss() {
    assert_flat("b573_user_fail", B573_USER_FAIL, 200_000, 400_000);
}

#[cfg(unix)]
#[test]
fn a_propagated_error_runs_at_constant_rss() {
    assert_flat("b573_propagated", B573_PROPAGATED, 200_000, 400_000);
}

#[cfg(unix)]
#[test]
fn a_runtime_helper_error_runs_at_constant_rss() {
    assert_flat("b573_helper", B573_HELPER_ERROR, 200_000, 400_000);
}

#[cfg(unix)]
#[test]
fn a_loop_that_raises_nothing_still_runs_at_constant_rss() {
    assert_flat("b573_no_error", B573_CONTRAST_NO_ERROR, 200_000, 400_000);
}

/// The VALUE half, and the one that matters: the change frees a block the origin
/// was copied FROM, so its failure direction is a use-after-free that reads back
/// as a wrong filename / line / column, or as an unrelated allocation failure
/// later — never as a failing free.
///
/// Every origin-reading shape is here: an inline builtin's domain error, a
/// runtime helper's stamped origin, a user `FAIL`, a `PROPAGATE` two frames down,
/// a function-level `TRAP`, and the untrapped top-level banner (a separate
/// program, since it ends the process). Each reads `source.filename`,
/// `source.line` and `source.char`, all of which live in the block whose original
/// has just been released.
const B573_ORIGIN_VALUES: &str = "IMPORT io\n\
IMPORT collections\n\
IMPORT fs\n\
FUNC deep(n AS Integer) AS Integer\n\
  IF n = 0 THEN\n\
    FAIL error(90000001, \"from deep\")\n\
  END IF\n\
  RETURN deep(n - 1)\n\
END FUNC\n\
FUNC reraise() AS Integer\n\
  LET v AS Integer = deep(2) TRAP(inner)\n\
    PROPAGATE\n\
  END TRAP\n\
  RETURN v\n\
END FUNC\n\
FUNC viaTrap() AS String\n\
  LET xs AS List OF String = [\"aa\"]\n\
  RETURN collections::get(xs, 9)\n\
  TRAP(err)\n\
    RETURN err.source.filename & \":\" & toString(err.source.line)\n\
  END TRAP\n\
END FUNC\n\
SUB main()\n\
  LET xs AS List OF String = [\"aa\", \"bb\"]\n\
  MUT i AS Integer = 0\n\
  MUT churn AS Integer = 0\n\
  WHILE i < 500\n\
    LET c AS String = collections::get(xs, 9) TRAP(warm)\n\
      RECOVER warm.source.filename\n\
    END TRAP\n\
    churn = churn + len(c)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"churn=\" & toString(churn))\n\
  LET a AS String = collections::get(xs, 9) TRAP(e1)\n\
    RECOVER e1.source.filename & \":\" & toString(e1.source.line) & \":\" & toString(e1.source.char)\n\
  END TRAP\n\
  io::print(\"builtin=\" & a)\n\
  LET b AS String = fs::readText(\"/tmp/b573_absent_file.txt\") TRAP(e2)\n\
    RECOVER toString(e2.code) & \"@\" & e2.source.filename & \":\" & toString(e2.source.line)\n\
  END TRAP\n\
  io::print(\"helper=\" & b)\n\
  LET c AS Integer = deep(3) TRAP(e3)\n\
    io::print(\"fail=\" & e3.message & \"@\" & e3.source.filename & \":\" & toString(e3.source.line))\n\
    RECOVER 0\n\
  END TRAP\n\
  io::print(\"failGot=\" & toString(c))\n\
  LET d AS Integer = reraise() TRAP(e4)\n\
    io::print(\"prop=\" & toString(e4.code) & \"@\" & e4.source.filename & \":\" & toString(e4.source.line))\n\
    RECOVER 0\n\
  END TRAP\n\
  io::print(\"propGot=\" & toString(d))\n\
  io::print(\"viaTrap=\" & viaTrap())\n\
END SUB\n";

#[test]
fn every_raised_error_still_reports_its_true_origin() {
    let project = common::temp_project("b573_origins", B573_ORIGIN_VALUES);
    let exe = common::build_project(&project);
    let expected = "churn=6000\n\
                    builtin=src/main.mfb:35:19\n\
                    helper=77030001@src/main.mfb:39\n\
                    fail=from deep@src/main.mfb:6\n\
                    failGot=0\n\
                    prop=90000001@src/main.mfb:6\n\
                    propGot=0\n\
                    viaTrap=src/main.mfb:18";
    // Ten runs: a released-too-early block is only observably wrong once the arena
    // hands it to a later allocation, and which allocation that is depends on the
    // fill pattern the arena seeds per process. The 500-iteration warm-up above
    // guarantees the released `ErrorLoc`s have been recycled before these lines
    // are built.
    for run in 0..10 {
        let output = std::process::Command::new(&exe)
            .current_dir(&project)
            .output()
            .expect("run the raised-error origin probe");
        assert!(
            output.status.success(),
            "run {run}: {}\n{}",
            common::exit_description(&output.status),
            String::from_utf8_lossy(&output.stdout)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            expected,
            "run {run}: a raised error reported the wrong origin. bug-573 frees the \
             `ErrorLoc` the parked `Error` block copied, so its failure direction \
             is exactly this — a filename, line or column read out of memory the \
             arena has already handed to something else"
        );
    }
    let _ = std::fs::remove_dir_all(&project);
}

// ---------------------------------------------------------------- bug-576

/// bug-576: an UNBOUND runtime-helper `String` result had no owner at all.
///
/// `LET s AS String = os::arch()` has always been flat — the `Bind` path's
/// `owns_freeable_value` registers a scope-drop `arena_free` for the block the
/// call yields. The same call written where nothing binds it —
/// `n = n + len(os::arch())`, `"x" & os::arch()`, an argument to another call —
/// yields a `ValueResult` no binding claims, and `register_pending_temp` declined
/// it because a bare `String` needs freshness provenance (bug-536 shape B) that
/// `emit_runtime_helper_call` never set. Measured on the pre-fix release binary,
/// 200k -> 400k iterations: 129 B/call for `os::hostName()`, 64 B/call for
/// `os::arch()`, 260 B/call for `fs::tempDirectory()`; the `LET`-bound spelling of
/// the same call moved 32 KB in total.
///
/// The fix ADDS an `arena_free`, so the hazard it carries is a DOUBLE free, not a
/// leak: every position that already owned its result is pinned below as a
/// positive case, and the value probe reads back every shape whose block now has
/// a statement-scope owner.
const SHAPE_576_UNBOUND_HOSTNAME: &str = "IMPORT io\n\
IMPORT os\n\
SUB main()\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    n = n + len(os::hostName())\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END SUB\n";

/// The helper that allocates NOTHING but its result — the row that proves this is
/// not bug-574's marshalling scratch, which this shape never allocates.
const SHAPE_576_UNBOUND_ARCH: &str = "IMPORT io\n\
IMPORT os\n\
SUB main()\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    n = n + len(os::arch())\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END SUB\n";

/// The largest measured row (260 B/call): a longer result block.
const SHAPE_576_UNBOUND_TEMPDIR: &str = "IMPORT io\n\
IMPORT fs\n\
SUB main()\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    n = n + len(fs::tempDirectory())\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END SUB\n";

/// The result consumed by `&` rather than by `len` — a different consumer, the
/// same ownerless block. The concat's own result is bound, so only the helper's
/// block is at stake here.
const SHAPE_576_UNBOUND_IN_CONCAT: &str = "IMPORT io\n\
IMPORT os\n\
SUB main()\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET joined AS String = \"arch=\" & os::arch()\n\
    n = n + len(joined)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END SUB\n";

/// The result consumed as ANOTHER call's argument — the third unbound position
/// named in the report. The outer call copies what it needs; the helper's block is
/// dead the moment it returns.
const SHAPE_576_UNBOUND_AS_ARGUMENT: &str = "IMPORT io\n\
IMPORT os\n\
IMPORT strings\n\
SUB main()\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET shout AS String = strings::upper(os::arch())\n\
    n = n + len(shout)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END SUB\n";

/// POSITIVE PIN. `LET s AS String = os::arch()` was flat BEFORE this change and
/// must stay flat: the bind claims the pending temp, so the block has exactly one
/// owner. A second free here would be a double free — the shape this cluster
/// nearly shipped twice.
const SHAPE_576_CONTRAST_BOUND_ARCH: &str = "IMPORT io\n\
IMPORT os\n\
SUB main()\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET s AS String = os::arch()\n\
    n = n + len(s)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END SUB\n";

/// POSITIVE PIN, the report's second flat row.
const SHAPE_576_CONTRAST_BOUND_CWD: &str = "IMPORT io\n\
IMPORT fs\n\
SUB main()\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET s AS String = fs::currentDirectory()\n\
    n = n + len(s)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END SUB\n";

/// `RETURN os::arch()` — measured RED at 12 MB / 200k too, and for the same
/// reason with one more step. `lower_returned_value` found no pending temp to
/// claim (there was none to register), so it fell through to the
/// `current_returns_fresh_string` arm and `copy_flat_block`ed the helper's block
/// to deliver the promise — leaving the ORIGINAL with no owner. Marking the
/// result makes it a claimable temp, so the block is MOVED to the caller and the
/// redundant copy disappears with the leak, exactly as bug-536 shape A's claim
/// arm does for a fresh constructor.
///
/// This is also a double-free pin: if the claim missed while the free was
/// registered, the caller reads a freed block — so it is read back in the value
/// probe as well as measured here.
const SHAPE_576_CONTRAST_RETURNED: &str = "IMPORT io\n\
IMPORT os\n\
FUNC whichArch() AS String\n  RETURN os::arch()\nEND FUNC\n\
SUB main()\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET s AS String = whichArch()\n\
    n = n + len(s)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END SUB\n";

/// The result passed straight into a collection — measured RED at 24 MB / 200k.
/// `append` COPIES the element's bytes into the container's data region, so the
/// helper's own block is dead the moment the call returns and nothing owned it.
///
/// It is simultaneously the sharpest double-free pin in the set: if `append` took
/// the pointer instead of copying, the new statement-scope free would `arena_free`
/// a block the list still points into, and the element read back on the next line
/// is what catches that.
///
/// The element is read into a BINDING deliberately. An UNBOUND
/// `collections::getOr(...)` `String` leaks 64 B per call by itself — measured at
/// exactly that on a list built from a LITERAL, with no runtime helper anywhere in
/// the program — which is a different producer's missing freshness mark, not this
/// one's. Binding it keeps this case measuring the helper's block; the unbound
/// `getOr` is filed separately.
const SHAPE_576_CONTRAST_INTO_COLLECTION: &str = "IMPORT io\n\
IMPORT os\n\
IMPORT collections\n\
SUB main()\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    MUT names AS List OF String = []\n\
    names = collections::append(names, os::arch())\n\
    LET element AS String = collections::getOr(names, 0, \"\")\n\
    n = n + len(element)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END SUB\n";

#[cfg(unix)]
#[test]
fn an_unbound_helper_hostname_runs_at_constant_rss() {
    assert_flat(
        "b576_unbound_hostname",
        SHAPE_576_UNBOUND_HOSTNAME,
        200_000,
        400_000,
    );
}

#[cfg(unix)]
#[test]
fn an_unbound_helper_result_that_allocates_only_its_result_runs_at_constant_rss() {
    assert_flat(
        "b576_unbound_arch",
        SHAPE_576_UNBOUND_ARCH,
        200_000,
        400_000,
    );
}

#[cfg(unix)]
#[test]
fn an_unbound_helper_tempdir_runs_at_constant_rss() {
    assert_flat(
        "b576_unbound_tempdir",
        SHAPE_576_UNBOUND_TEMPDIR,
        200_000,
        400_000,
    );
}

#[cfg(unix)]
#[test]
fn an_unbound_helper_result_consumed_by_concat_runs_at_constant_rss() {
    assert_flat(
        "b576_unbound_concat",
        SHAPE_576_UNBOUND_IN_CONCAT,
        200_000,
        400_000,
    );
}

#[cfg(unix)]
#[test]
fn an_unbound_helper_result_passed_to_another_call_runs_at_constant_rss() {
    assert_flat(
        "b576_unbound_argument",
        SHAPE_576_UNBOUND_AS_ARGUMENT,
        200_000,
        400_000,
    );
}

#[cfg(unix)]
#[test]
fn a_bound_helper_result_still_runs_at_constant_rss() {
    assert_flat(
        "b576_bound_arch",
        SHAPE_576_CONTRAST_BOUND_ARCH,
        200_000,
        400_000,
    );
}

#[cfg(unix)]
#[test]
fn a_bound_directory_helper_result_still_runs_at_constant_rss() {
    assert_flat(
        "b576_bound_cwd",
        SHAPE_576_CONTRAST_BOUND_CWD,
        200_000,
        400_000,
    );
}

#[cfg(unix)]
#[test]
fn a_returned_helper_result_still_runs_at_constant_rss() {
    assert_flat(
        "b576_returned",
        SHAPE_576_CONTRAST_RETURNED,
        200_000,
        400_000,
    );
}

#[cfg(unix)]
#[test]
fn a_helper_result_moved_into_a_collection_still_runs_at_constant_rss() {
    assert_flat(
        "b576_into_collection",
        SHAPE_576_CONTRAST_INTO_COLLECTION,
        200_000,
        400_000,
    );
}

/// The VALUE half, and the half that carries the risk. bug-576 ADDS an
/// `arena_free` on a position that previously had none, so the way it fails is a
/// block freed while someone still holds it — a wrong value or a later unrelated
/// allocation failure, never a failing free.
///
/// Every claimed position is exercised in one process, after a churn loop that
/// guarantees the arena has recycled anything freed too early into a later
/// allocation:
///
/// * the unbound positions themselves, read back through `len`, `&` and an outer
///   call;
/// * a `LET`-bound result, a reassigned `MUT`, a returned result and a result
///   moved into a `List OF String` — the four positions that already owned the
///   block and must not gain a second free;
/// * `thread::waitFor`, whose result the WORKER's arena allocated: it is
///   `runtime_call_result_is_foreign_arena`, so it must keep its old exemption.
///   A cross-arena free is the one failure mode in this family that corrupts
///   another thread's heap.
const SHAPE_576_VALUES: &str = "IMPORT io\n\
IMPORT os\n\
IMPORT fs\n\
IMPORT strings\n\
IMPORT collections\n\
IMPORT thread\n\
ISOLATED FUNC worker(w AS ThreadWorker OF String TO String, seed AS String) AS String\n\
  RETURN seed & \"-done\"\n\
END FUNC\n\
FUNC whichArch() AS String\n  RETURN os::arch()\nEND FUNC\n\
SUB main()\n\
  MUT churn AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < 2000\n\
    churn = churn + len(os::arch()) + len(fs::tempDirectory())\n\
    LET scratch AS String = \"pad-\" & toString(i) & \"-pad\"\n\
    churn = churn + len(scratch)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"churn=\" & toString(churn > 0))\n\
  LET arch AS String = os::arch()\n\
  io::print(\"boundMatchesUnbound=\" & toString(len(arch) = len(os::arch())))\n\
  io::print(\"concat=arch:\" & os::arch())\n\
  io::print(\"upper=\" & strings::upper(os::arch()))\n\
  io::print(\"returned=\" & whichArch())\n\
  io::print(\"returnedMatches=\" & toString(whichArch() = arch))\n\
  MUT reassigned AS String = \"seed\"\n\
  reassigned = os::arch()\n\
  io::print(\"reassigned=\" & reassigned)\n\
  MUT names AS List OF String = []\n\
  names = collections::append(names, os::arch())\n\
  names = collections::append(names, os::hostName())\n\
  io::print(\"element=\" & collections::getOr(names, 0, \"MISSING\"))\n\
  io::print(\"elements=\" & toString(len(names)))\n\
  io::print(\"host=\" & toString(len(os::hostName()) > 0))\n\
  io::print(\"tmp=\" & toString(len(fs::tempDirectory()) > 0))\n\
  io::print(\"cwd=\" & toString(len(fs::currentDirectory()) > 0))\n\
  LET t AS Thread OF String TO String = thread::start(worker, \"worker\")\n\
  LET out AS String = thread::waitFor(t) TRAP(e)\n\
    RECOVER \"thread-failed\"\n\
  END TRAP\n\
  io::print(\"thread=\" & out)\n\
  io::print(\"archAgain=\" & arch)\n\
END SUB\n";

#[test]
fn every_unbound_helper_result_position_still_produces_the_right_value() {
    let project = common::temp_project("b576_values", SHAPE_576_VALUES);
    let exe = common::build_project(&project);
    // `os::arch()` is a fixed string for the host, so the expectation is derived
    // from the one line that reads it back through a shape this change does not
    // touch (an assignment from a `MUT`) rather than hard-coded per platform.
    for run in 1..=25 {
        let output = std::process::Command::new(&exe)
            .current_dir(&project)
            .output()
            .expect("run the unbound-helper-result ownership probe");
        assert!(
            output.status.success(),
            "run {run}: {}\n{}",
            common::exit_description(&output.status),
            String::from_utf8_lossy(&output.stdout)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<&str> = stdout.trim().lines().collect();
        let arch = lines
            .iter()
            .find_map(|line| line.strip_prefix("reassigned="))
            .unwrap_or_else(|| panic!("run {run}: no `reassigned` line in:\n{stdout}"))
            .to_string();
        assert!(
            !arch.is_empty(),
            "run {run}: the architecture read back empty:\n{stdout}"
        );
        let expected = [
            "churn=TRUE".to_string(),
            "boundMatchesUnbound=TRUE".to_string(),
            format!("concat=arch:{arch}"),
            format!("upper={}", arch.to_uppercase()),
            format!("returned={arch}"),
            "returnedMatches=TRUE".to_string(),
            format!("reassigned={arch}"),
            format!("element={arch}"),
            "elements=2".to_string(),
            "host=TRUE".to_string(),
            "tmp=TRUE".to_string(),
            "cwd=TRUE".to_string(),
            "thread=worker-done".to_string(),
            format!("archAgain={arch}"),
        ]
        .join("\n");
        assert_eq!(
            stdout.trim(),
            expected,
            "run {run}: an unbound runtime-helper result read back wrong. bug-576 \
             adds a statement-scope `arena_free` to positions that had no owner, \
             so its failure direction is a block freed while someone still holds \
             it — a `thread=` mismatch means the free reached the WORKER's arena"
        );
    }
    let _ = std::fs::remove_dir_all(&project);
}

// ---------------------------------------------------------------- bug-592

/// bug-592: an UNBOUND `collections::getOr` `String` element had no owner.
///
/// `§14.6` of `mfb spec language memory-semantics`: "Reads produce owned values,
/// not aliases into the buffer." An owned value nothing binds is dropped at
/// statement scope — but a bare `String` temp is freed there only with freshness
/// provenance (bug-536 shape B), and `getOr` lost its mark: the MISS path's
/// `emit_copy_owned_string` runs after the found path's materialization and
/// overwrites the mark with its own register, which is not the lowering's
/// result, so `lower_value`'s identity test rejected it. `get` has no copying
/// miss path (it raises), which is why `get` was always flat.
///
/// Measured on the pre-fix release binary (`integ-576-590`), 200k -> 400k
/// iterations, `/usr/bin/time -l` peak RSS:
///
/// | shape | per call |
/// | --- | --- |
/// | list `getOr` hit | 64 B |
/// | list `getOr` miss | 129 B |
/// | `Map OF String TO String` (hash probe) `getOr` hit / miss | 64 B / 128 B |
/// | `Map OF Scalar TO String` (entry scan) `getOr` hit / miss | 65 B / 130 B |
/// | unbound `get`, list / hash map / scan map | 0 B |
/// | `LET`-bound `getOr` | 0 B |
///
/// The fix ADDS a statement-scope `arena_free`, so the failure it risks is a
/// WILD or DOUBLE free, not a leak: a block the container, the caller's default,
/// or an owning binding still holds. The positive pins below and the value probe
/// are that half.
const SHAPE_592_LIST_GETOR_HIT: &str = "IMPORT io\n\
IMPORT collections\n\
SUB main()\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    MUT names AS List OF String = [\"aarch64\"]\n\
    n = n + len(collections::getOr(names, 0, \"\"))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END SUB\n";

/// The miss path: the result is `emit_copy_owned_string`'s copy of the caller's
/// default. Freeing it must never free the DEFAULT itself — the value probe reads
/// the default back after thousands of misses.
const SHAPE_592_LIST_GETOR_MISS: &str = "IMPORT io\n\
IMPORT collections\n\
SUB main()\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    MUT names AS List OF String = [\"aarch64\"]\n\
    n = n + len(collections::getOr(names, 5, \"fallback\"))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END SUB\n";

/// `lower_map_get_or`'s hash-probe arm (a `String` key is probe-eligible).
const SHAPE_592_HASH_MAP_GETOR_HIT: &str = "IMPORT io\n\
IMPORT collections\n\
SUB main()\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    MUT m AS Map OF String TO String = Map OF String TO String {\"k\" := \"aarch64\"}\n\
    n = n + len(collections::getOr(m, \"k\", \"\"))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END SUB\n";

const SHAPE_592_HASH_MAP_GETOR_MISS: &str = "IMPORT io\n\
IMPORT collections\n\
SUB main()\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    MUT m AS Map OF String TO String = Map OF String TO String {\"k\" := \"aarch64\"}\n\
    n = n + len(collections::getOr(m, \"zz\", \"fallback\"))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END SUB\n";

/// `lower_map_get_or`'s linear entry-scan arm (a `Scalar` key is not
/// probe-eligible) — a separate emission path with its own join point.
const SHAPE_592_SCAN_MAP_GETOR_HIT: &str = "IMPORT io\n\
IMPORT collections\n\
SUB main()\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    MUT m AS Map OF Scalar TO String = Map OF Scalar TO String {}\n\
    m = collections::set(m, `k`, \"aarch64\")\n\
    n = n + len(collections::getOr(m, `k`, \"\"))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END SUB\n";

const SHAPE_592_SCAN_MAP_GETOR_MISS: &str = "IMPORT io\n\
IMPORT collections\n\
SUB main()\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    MUT m AS Map OF Scalar TO String = Map OF Scalar TO String {}\n\
    m = collections::set(m, `k`, \"aarch64\")\n\
    n = n + len(collections::getOr(m, `z`, \"fallback\"))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END SUB\n";

/// `RETURN collections::getOr(..)` — with no pending temp to claim, the callee's
/// `function_returns_fresh_string` promise was delivered by a `copy_flat_block`
/// that left the original block ownerless (bug-576's returned row, one producer
/// over). Marked, the temp is claimed and MOVED to the caller. Also a
/// double-free pin: a missed claim with a registered free hands the caller a
/// freed block, which the value probe reads back.
const SHAPE_592_RETURNED: &str = "IMPORT io\n\
IMPORT collections\n\
FUNC firstName(names AS List OF String) AS String\n  RETURN collections::getOr(names, 0, \"none\")\nEND FUNC\n\
SUB main()\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    MUT names AS List OF String = [\"aarch64\"]\n\
    LET s AS String = firstName(names)\n\
    n = n + len(s)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END SUB\n";

/// POSITIVE PIN. The report's flat row: the bind owns the block. With the mark
/// the bind CLAIMS the pending temp instead; a missed claim is a double free.
const SHAPE_592_CONTRAST_BOUND_GETOR: &str = "IMPORT io\n\
IMPORT collections\n\
SUB main()\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    MUT names AS List OF String = [\"aarch64\"]\n\
    LET e AS String = collections::getOr(names, 0, \"\")\n\
    n = n + len(e)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END SUB\n";

/// POSITIVE PIN. Unbound `get` on a list and on both map arms was already flat —
/// the producer's own mark survived because the miss path raises. The join-point
/// mark restates it; it must not add a second free.
const SHAPE_592_CONTRAST_UNBOUND_GET: &str = "IMPORT io\n\
IMPORT collections\n\
SUB main()\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    MUT names AS List OF String = [\"aarch64\"]\n\
    MUT m AS Map OF String TO String = Map OF String TO String {\"k\" := \"x86_64\"}\n\
    MUT s AS Map OF Scalar TO String = Map OF Scalar TO String {}\n\
    s = collections::set(s, `k`, \"riscv64\")\n\
    n = n + len(collections::get(names, 0)) + len(collections::get(m, \"k\")) + len(collections::get(s, `k`))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END SUB\n";

/// POSITIVE PIN — the ALIAS direction. plan-86 E: `LET e = get(L, i)` used only
/// as a `MATCH` scrutinee over an immutable `L` makes `e` an ALIAS into `L`'s
/// data region (`borrow_get_result`), never freed. Its element is a non-`String`
/// union, so `mark_fresh_element_result` never marks it, and
/// `register_pending_temp` early-returns while the flag is set. A wrong free here
/// is an `arena_free` into the container; the value probe reads the container
/// back (and adds a borrowed read whose map KEY is an unbound `String` `getOr`
/// inside the borrowed initializer) to see that as a corrupted neighbour.
const SHAPE_592_CONTRAST_BORROWED_GET: &str = "IMPORT io\n\
IMPORT collections\n\
TYPE Dot\n  x AS Integer\nEND TYPE\n\
TYPE Tag\n  name AS String\nEND TYPE\n\
UNION Shape\n  Dot\n  Tag\nEND UNION\n\
SUB main()\n\
  LET shapes AS List OF Shape = [Dot[1], Tag[\"tag-name-long-enough-to-matter\"], Dot[3]]\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET e AS Shape = collections::get(shapes, i - (i / 3) * 3)\n\
    MATCH e\n\
      CASE Dot(d)\n\
        n = n + d.x\n\
      CASE Tag(t)\n\
        n = n + len(t.name)\n\
    END MATCH\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END SUB\n";

#[cfg(unix)]
#[test]
fn an_unbound_list_getor_hit_runs_at_constant_rss() {
    assert_flat(
        "b592_list_getor_hit",
        SHAPE_592_LIST_GETOR_HIT,
        200_000,
        400_000,
    );
}

#[cfg(unix)]
#[test]
fn an_unbound_list_getor_miss_runs_at_constant_rss() {
    assert_flat(
        "b592_list_getor_miss",
        SHAPE_592_LIST_GETOR_MISS,
        200_000,
        400_000,
    );
}

#[cfg(unix)]
#[test]
fn an_unbound_hash_map_getor_hit_runs_at_constant_rss() {
    assert_flat(
        "b592_hash_map_getor_hit",
        SHAPE_592_HASH_MAP_GETOR_HIT,
        200_000,
        400_000,
    );
}

#[cfg(unix)]
#[test]
fn an_unbound_hash_map_getor_miss_runs_at_constant_rss() {
    assert_flat(
        "b592_hash_map_getor_miss",
        SHAPE_592_HASH_MAP_GETOR_MISS,
        200_000,
        400_000,
    );
}

#[cfg(unix)]
#[test]
fn an_unbound_scan_map_getor_hit_runs_at_constant_rss() {
    assert_flat(
        "b592_scan_map_getor_hit",
        SHAPE_592_SCAN_MAP_GETOR_HIT,
        200_000,
        400_000,
    );
}

#[cfg(unix)]
#[test]
fn an_unbound_scan_map_getor_miss_runs_at_constant_rss() {
    assert_flat(
        "b592_scan_map_getor_miss",
        SHAPE_592_SCAN_MAP_GETOR_MISS,
        200_000,
        400_000,
    );
}

#[cfg(unix)]
#[test]
fn a_returned_getor_element_runs_at_constant_rss() {
    assert_flat("b592_returned", SHAPE_592_RETURNED, 200_000, 400_000);
}

#[cfg(unix)]
#[test]
fn a_bound_getor_element_still_runs_at_constant_rss() {
    assert_flat(
        "b592_bound_getor",
        SHAPE_592_CONTRAST_BOUND_GETOR,
        200_000,
        400_000,
    );
}

#[cfg(unix)]
#[test]
fn an_unbound_get_element_still_runs_at_constant_rss() {
    assert_flat(
        "b592_unbound_get",
        SHAPE_592_CONTRAST_UNBOUND_GET,
        200_000,
        400_000,
    );
}

#[cfg(unix)]
#[test]
fn a_borrowed_get_element_still_runs_at_constant_rss() {
    assert_flat(
        "b592_borrowed_get",
        SHAPE_592_CONTRAST_BORROWED_GET,
        200_000,
        400_000,
    );
}

/// The VALUE half, and the half that carries the risk. Every position whose
/// `getOr`/`get` `String` now has a statement-scope owner, and every position that
/// must NOT gain one, exercised in one process after a churn loop that recycles
/// anything freed too early into a later allocation:
///
/// * unbound `getOr` hit and miss on all three emission paths (list, hash map,
///   scan map) and unbound `get` on list and hash map;
/// * the caller's DEFAULT (`dflt`), read back after thousands of misses — the
///   miss path frees its copy, never the default;
/// * a `LET`-bound, a reassigned `MUT`, a returned, and an appended element — the
///   positions that own the block through a claim;
/// * two plan-86 E borrowed (ALIAS) reads: a union element used only as a `MATCH`
///   scrutinee, and one whose map KEY is itself an unbound `String` `getOr`
///   inside the borrowed initializer;
/// * the containers themselves, read back element by element at the end — a
///   free that landed INTO a container shows up here as a corrupted element.
const SHAPE_592_VALUES: &str = "IMPORT io\n\
IMPORT collections\n\
TYPE Dot\n  x AS Integer\nEND TYPE\n\
TYPE Tag\n  name AS String\nEND TYPE\n\
UNION Shape\n  Dot\n  Tag\nEND UNION\n\
FUNC firstName(names AS List OF String) AS String\n  RETURN collections::getOr(names, 0, \"none\")\nEND FUNC\n\
SUB main()\n\
  LET names AS List OF String = [\"alpha-element-000\", \"beta-element-1111\", \"gamma-element-22222\"]\n\
  LET byKey AS Map OF String TO String = Map OF String TO String {\"a\" := \"map-alpha-value\", \"b\" := \"map-beta-value-longer\"}\n\
  MUT byScalar AS Map OF Scalar TO String = Map OF Scalar TO String {}\n\
  byScalar = collections::set(byScalar, `a`, \"scalar-alpha-value\")\n\
  LET shapes AS List OF Shape = [Dot[1], Tag[\"tag-name-long-enough-to-matter\"], Dot[3]]\n\
  LET byShape AS Map OF String TO Shape = Map OF String TO Shape {\"alpha-element-000\" := Tag[\"via-map\"], \"none\" := Dot[9]}\n\
  LET dflt AS String = \"default-\" & toString(7)\n\
  MUT churn AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < 5000\n\
    churn = churn + len(collections::getOr(names, i - (i / 4) * 4, dflt))\n\
    churn = churn + len(collections::getOr(byKey, \"a\", dflt)) + len(collections::getOr(byKey, \"zz\", dflt))\n\
    churn = churn + len(collections::getOr(byScalar, `a`, dflt)) + len(collections::getOr(byScalar, `z`, dflt))\n\
    churn = churn + len(collections::get(names, 1)) + len(collections::get(byKey, \"b\"))\n\
    LET scratch AS String = \"pad-\" & toString(i) & \"-pad\"\n\
    churn = churn + len(scratch)\n\
    LET e AS Shape = collections::get(shapes, i - (i / 3) * 3)\n\
    MATCH e\n\
      CASE Dot(d)\n\
        churn = churn + d.x\n\
      CASE Tag(t)\n\
        churn = churn + len(t.name)\n\
    END MATCH\n\
    LET k AS Shape = collections::get(byShape, collections::getOr(names, 5, \"none\"))\n\
    MATCH k\n\
      CASE Dot(d)\n\
        churn = churn + d.x\n\
      CASE Tag(t)\n\
        churn = churn + len(t.name)\n\
    END MATCH\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"churn=\" & toString(churn))\n\
  io::print(\"concat=\" & collections::getOr(names, 2, \"x\") & \"|\" & collections::getOr(names, 9, dflt))\n\
  io::print(\"dflt=\" & dflt)\n\
  LET bound AS String = collections::getOr(names, 1, dflt)\n\
  MUT reassigned AS String = \"seed\"\n\
  reassigned = collections::getOr(byKey, \"b\", dflt)\n\
  io::print(\"bound=\" & bound & \" reassigned=\" & reassigned)\n\
  io::print(\"returned=\" & firstName(names))\n\
  MUT copied AS List OF String = []\n\
  copied = collections::append(copied, collections::getOr(names, 0, dflt))\n\
  copied = collections::append(copied, collections::getOr(byScalar, `q`, dflt))\n\
  io::print(\"copied=\" & collections::get(copied, 0) & \",\" & collections::get(copied, 1))\n\
  io::print(\"names=\" & collections::get(names, 0) & \",\" & collections::get(names, 1) & \",\" & collections::get(names, 2))\n\
  io::print(\"maps=\" & collections::get(byKey, \"a\") & \",\" & collections::get(byKey, \"b\") & \",\" & collections::get(byScalar, `a`))\n\
  io::print(\"bound=\" & bound & \" dflt=\" & dflt)\n\
END SUB\n";

#[test]
fn every_unbound_collection_element_position_still_produces_the_right_value() {
    // The churn total, derived from the literal lengths rather than from any
    // compiler's output (a leaky binary prints the same number; a wrong free does
    // not).
    let names = [
        "alpha-element-000",
        "beta-element-1111",
        "gamma-element-22222",
    ];
    let dflt = "default-7";
    let tag = "tag-name-long-enough-to-matter";
    let mut churn: usize = 0;
    for i in 0..5000usize {
        churn += match i % 4 {
            3 => dflt.len(),
            j => names[j].len(),
        };
        churn += "map-alpha-value".len() + dflt.len();
        churn += "scalar-alpha-value".len() + dflt.len();
        churn += names[1].len() + "map-beta-value-longer".len();
        churn += format!("pad-{i}-pad").len();
        churn += match i % 3 {
            0 => 1,
            1 => tag.len(),
            _ => 3,
        };
        churn += 9; // byShape["none"] = Dot[9]
    }
    let expected = [
        format!("churn={churn}"),
        format!("concat={}|{dflt}", names[2]),
        format!("dflt={dflt}"),
        format!("bound={} reassigned=map-beta-value-longer", names[1]),
        format!("returned={}", names[0]),
        format!("copied={},{dflt}", names[0]),
        format!("names={},{},{}", names[0], names[1], names[2]),
        "maps=map-alpha-value,map-beta-value-longer,scalar-alpha-value".to_string(),
        format!("bound={} dflt={dflt}", names[1]),
    ]
    .join("\n");

    let project = common::temp_project("b592_values", SHAPE_592_VALUES);
    let exe = common::build_project(&project);
    for run in 1..=25 {
        let output = std::process::Command::new(&exe)
            .current_dir(&project)
            .output()
            .expect("run the unbound-collection-element ownership probe");
        assert!(
            output.status.success(),
            "run {run}: {}\n{}",
            common::exit_description(&output.status),
            String::from_utf8_lossy(&output.stdout)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(
            stdout.trim(),
            expected,
            "run {run}: a collection element read back wrong. bug-592 adds a \
             statement-scope `arena_free` to an unbound `getOr` `String`, so its \
             failure direction is a block freed while someone still holds it — a \
             `names=`/`maps=` mismatch means the free landed INTO a container, a \
             `dflt=` mismatch that it freed the caller's default"
        );
    }
    let _ = std::fs::remove_dir_all(&project);
}

/// bug-592 audit finding: plan-86 E's `borrow_get_result` flag covered the WHOLE
/// initializer, not the borrowed `get` node. So a fresh temp in one of the
/// borrowed call's OPERANDS — here an unbound `String` `getOr` used as the map
/// KEY — had its statement-scope registration suppressed too, and leaked 64 B per
/// evaluation (measured on the bug-592 join-point fix alone, 200k -> 400k). The
/// flag now applies only inside the borrowed node's own `lower_value` frame; each
/// operand frame lowers it with the flag clear, which is the ordinary copy + free
/// path. `materialize_owned_element` reads the same narrowed flag, so a nested
/// `get` operand is COPIED (never an unfreed alias) exactly when it is freed.
const SHAPE_592_BORROWED_GET_WITH_A_FRESH_KEY: &str = "IMPORT io\n\
IMPORT collections\n\
TYPE Dot\n  x AS Integer\nEND TYPE\n\
TYPE Tag\n  name AS String\nEND TYPE\n\
UNION Shape\n  Dot\n  Tag\nEND UNION\n\
SUB main()\n\
  LET byShape AS Map OF String TO Shape = Map OF String TO Shape {\"alpha\" := Tag[\"via-map\"], \"none\" := Dot[9]}\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    MUT names AS List OF String = [\"alpha\"]\n\
    LET k AS Shape = collections::get(byShape, collections::getOr(names, 0, \"none\"))\n\
    MATCH k\n\
      CASE Dot(d)\n\
        n = n + d.x\n\
      CASE Tag(t)\n\
        n = n + len(t.name)\n\
    END MATCH\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END SUB\n";

#[cfg(unix)]
#[test]
fn a_borrowed_get_with_a_fresh_key_operand_runs_at_constant_rss() {
    assert_flat(
        "b592_borrowed_get_fresh_key",
        SHAPE_592_BORROWED_GET_WITH_A_FRESH_KEY,
        200_000,
        400_000,
    );
}

// ---------------------------------------------------------------- bug-593
//
// A failing call under an inline `TRAP` grew memory by a flat ~1 KB per call
// when the binding's type was not a flat value — a `RES` (`fs::open`,
// `tcp::connect`, `tls::connect`, or a user callee returning a resource) or a
// `List OF net::Address`. bug-574 and bug-575 both measured it on runtime helpers
// and read it as "the failing helper"; it is not the helper. A user `FUNC` that
// `FAIL`s into a `RES` binding grows identically, and a failing `fs::readText`
// (a `String`) does not grow at all.
//
// The inline-`TRAP` desugar binds `$trap_resN : Result OF T = CallResult(..)`.
// That `{tag, size, payload}` wrapper is allocated by this frame and, on the
// error path, carries the whole trapped `Error` inlined at +16. A flat `T` gets
// an owned scope drop for it (`is_freeable_flat_value`); a non-flat `T` got
// none, so every failing iteration orphaned the wrapper and the `Error` in it.
// That is why the growth was flat in the ARGUMENT and differed by platform: its
// size is the error MESSAGE's.
//
// ## Why these are message-length comparisons and not `assert_flat`
//
// A `RES` binding's error path also allocates the default CLOSED resource record
// for `$trap_valN`, and a resource record is never reclaimed — it is the
// tombstone every alias reads the closed flag from (plan-52-B, Open Decisions:
// "Should the record itself ever be reclaimed? Not here."). A SUCCEEDING
// `fs::open` + `fs::close` loop grows the same way. When these cases were
// measured, a `List OF net::Address` binding likewise kept its default empty
// list, because `net::Address` was a pointer-`String` record with no drop
// (bug-599; plan-132 flattened it, and `a_looped_net_lookup_runs_at_constant_rss`
// now pins the list flat). Neither was this bug, and freeing either would have
// moved a lifetime, so neither shape could be flat here.
//
// What IS this bug is scaled by the trapped `Error`, and nothing else is. So each
// case runs the same program twice at the same count, differing only in the
// length of the message the callee `FAIL`s with; the tombstone and the default
// list are identical in both runs and cancel. Measured at 20 000 iterations,
// macOS aarch64, peak RSS:
//
// | binding | message | base (04c81a605) | fixed |
// | --- | --- | --- | --- |
// | `RES fs::File` | 14 chars | 17.8 MB | 8.8 MB |
// | `RES fs::File` | 4 014 chars | 328.7 MB | 8.8 MB |
// | `List OF net::Address` | 14 chars | 14.2 MB | 5.0 MB |
// | `List OF net::Address` | 4 014 chars | 328.8 MB | 5.0 MB |
//
// +311 MB before and 0 after on both; the threshold is 32 MB. The two cases take
// the fix's two drop kinds: a resource payload is a pointer word nothing aliases,
// so its wrapper is released on every path (`ResultWrapperDrop::Always`); a
// `List OF net::Address` payload is inlined in the wrapper and the Ok binding
// aliases it, so only the error-tagged wrapper is (`ErrorOnly`).
//
// The report's own shapes, measured the same way (base -> fixed): a refused
// `tcp::connect` 212.9 -> 424.3 MB became 81.4 -> 161.3 MB at 200 000 / 400 000;
// `tls::connect` refused 54.2 -> 101.5 MB became 38.7 -> 69.6 MB at
// 20 000 / 40 000; a failing `net::lookup` 23.2 -> 40.4 MB became
// 10.1 -> 14.1 MB. What remains on each is the tombstone or the default list.

/// A loop of `{N}` iterations whose callee always `FAIL`s with `message` into a
/// binding declared `binding`, produced by `producer` (whose success body is
/// never reached).
#[cfg(unix)]
fn b593_trapped_error_probe(imports: &str, binding: &str, producer: &str, message: &str) -> String {
    format!(
        "IMPORT io\n\
         {imports}\
         FUNC make(i AS Integer) AS {binding}\n\
        \x20 IF i >= 0 THEN\n\
        \x20   FAIL error(7, \"{message}\")\n\
        \x20 END IF\n\
        \x20 {producer}\n\
         END FUNC\n\
         SUB main()\n\
        \x20 MUT n AS Integer = 0\n\
        \x20 MUT i AS Integer = 0\n\
        \x20 WHILE i < {{N}}\n\
        \x20   __BIND__ = make(i) TRAP(e)\n\
        \x20     n = n + 1\n\
        \x20     i = i + 1\n\
        \x20     CONTINUE WHILE\n\
        \x20   END TRAP\n\
        \x20   __USE__\n\
        \x20   i = i + 1\n\
        \x20 END WHILE\n\
        \x20 io::print(\"n=\" & toString(n))\n\
         END SUB\n"
    )
}

/// Assert that the two message lengths cost the same peak RSS at `N` iterations.
#[cfg(unix)]
fn b593_assert_error_not_retained(name: &str, short: &str, long: &str) {
    const N: u64 = 20_000;
    let small = peak_rss(&format!("{name}_short"), short, N);
    let large = peak_rss(&format!("{name}_long"), long, N);
    let grew = large.saturating_sub(small);
    assert!(
        grew < 32 * 1024 * 1024,
        "{name}: a {N}-iteration loop of trapped errors cost {} MB more with a \
         4 014-character error message than with a 14-character one ({} MB -> {} MB). \
         The inline `TRAP` built a `Result` wrapper holding the whole `Error` inline \
         and nothing released it (bug-593)",
        grew / (1024 * 1024),
        small / (1024 * 1024),
        large / (1024 * 1024),
    );
}

#[cfg(unix)]
fn b593_long_message() -> String {
    format!("b593-long-msg-{}", "m".repeat(4_000))
}

#[cfg(unix)]
fn b593_resource_probe(message: &str) -> String {
    b593_trapped_error_probe(
        "IMPORT fs\n",
        "fs::File",
        "RES f AS fs::File = fs::open(\"/tmp/b593-does-not-exist/nope.txt\", \"r\")\n  RETURN f",
        message,
    )
    .replace("__BIND__", "RES f AS fs::File")
    .replace("__USE__", "fs::close(f)")
}

#[cfg(unix)]
fn b593_address_list_probe(message: &str) -> String {
    b593_trapped_error_probe(
        "IMPORT net\n",
        "List OF net::Address",
        "RETURN net::lookup(\"127.0.0.1\", 80)",
        message,
    )
    .replace("__BIND__", "LET xs AS List OF net::Address")
    .replace("__USE__", "n = n + len(xs)")
}

#[cfg(unix)]
#[test]
fn a_trapped_error_bound_as_a_resource_is_not_retained() {
    b593_assert_error_not_retained(
        "b593_res",
        &b593_resource_probe("b593-short-msg"),
        &b593_resource_probe(&b593_long_message()),
    );
}

#[cfg(unix)]
#[test]
fn a_trapped_error_bound_as_an_address_list_is_not_retained() {
    b593_assert_error_not_retained(
        "b593_list",
        &b593_address_list_probe("b593-short-msg"),
        &b593_address_list_probe(&b593_long_message()),
    );
}

/// The VALUE half, and the direction that matters: the fix ADDS a free of the
/// `Result` wrapper, and on the error path that wrapper holds the trapped `Error`
/// inline. Anything that still read that `Error` after the scope drop would read
/// freed memory — a wrong message or origin, or a fault at a later allocation,
/// never a failing free. So every way a handler can carry the `Error` across a
/// drop edge is here, each followed by more allocation before it is compared:
///
/// * `RETURN` from inside the handler, building the result from `e`;
/// * `FAIL inner` — a re-raise, where the drop runs on the `FAIL` edge and the
///   caller must still see the ORIGINAL origin (line 17, the `fs::open`);
/// * the hoisted chain form (bug-457), whose `Error` goes through `$trap_err`;
/// * a refused `tcp::connect`, and a user callee failing into `RES`;
/// * `List OF net::Address`, whose Ok payload IS inlined in the wrapper and
///   aliased by the binding — its Ok wrapper must NOT be released, and a resolved
///   address read back after more allocation is the pin for that;
/// * a SUCCEEDING `fs::open` read back, whose Ok wrapper now is released.
///
/// Every iteration's row is compared with the first iteration's, and the first
/// row with the text the base compiler printed, where nothing was freed.
const B593_VALUES: &str = "IMPORT io\n\
IMPORT fs\n\
IMPORT tcp\n\
IMPORT net\n\
IMPORT collections\n\
FUNC describe(e AS Error) AS String\n  RETURN toString(e.code) & \"|\" & e.message & \"|\" & e.source.filename & \":\" & toString(e.source.line)\n\
END FUNC\n\
FUNC openMissing(i AS Integer) AS String\n  RES f AS fs::File = fs::open(\"/tmp/b593-does-not-exist/nope.txt\", \"r\") TRAP(e)\n    RETURN \"open:\" & describe(e)\n  END TRAP\n  fs::close(f)\n  RETURN \"opened\"\n\
END FUNC\n\
FUNC reraiseMissing(i AS Integer) AS Integer\n  RES f AS fs::File = fs::open(\"/tmp/b593-does-not-exist/nope.txt\", \"r\") TRAP(inner)\n    FAIL inner\n  END TRAP\n  fs::close(f)\n  RETURN i\n\
END FUNC\n\
FUNC pathFor(i AS Integer) AS String\n  IF i < 0 THEN\n    FAIL error(9, \"negative\")\n  END IF\n  RETURN \"/tmp/b593-does-not-exist/hoisted.txt\"\n\
END FUNC\n\
FUNC hoisted(i AS Integer) AS String\n  RES f AS fs::File = fs::open(pathFor(i), \"r\") TRAP(e)\n    RETURN \"hoisted:\" & describe(e)\n  END TRAP\n  fs::close(f)\n  RETURN \"opened\"\n\
END FUNC\n\
FUNC refused(i AS Integer) AS String\n  RES s AS tcp::Socket = tcp::connect(\"127.0.0.1\", 1, 1000) TRAP(e)\n    RETURN \"tcp:\" & describe(e)\n  END TRAP\n  tcp::close(s)\n  RETURN \"connected\"\n\
END FUNC\n\
FUNC opener(i AS Integer) AS fs::File\n  IF i >= 0 THEN\n    FAIL error(7, \"opener-\" & toString(i MOD 1))\n  END IF\n  RES f AS fs::File = fs::open(\"/tmp/b593-does-not-exist/nope.txt\", \"r\")\n  RETURN f\n\
END FUNC\n\
FUNC userRes(i AS Integer) AS String\n  RES f AS fs::File = opener(i) TRAP(e)\n    RETURN \"user:\" & describe(e)\n  END TRAP\n  fs::close(f)\n  RETURN \"opened\"\n\
END FUNC\n\
FUNC okOpen(path AS String) AS String\n  RES f AS fs::File = fs::open(path, \"r\") TRAP(e)\n    RETURN \"ok-open-failed:\" & describe(e)\n  END TRAP\n  LET text AS String = fs::readAll(f)\n  fs::close(f)\n  RETURN \"ok:\" & text\n\
END FUNC\n\
FUNC lookupOk(i AS Integer) AS String\n  LET xs AS List OF net::Address = net::lookup(\"127.0.0.1\", 80 + i MOD 1) TRAP(e)\n    RETURN \"lookup-failed:\" & describe(e)\n  END TRAP\n  LET churn AS List OF String = [\"c1-\" & toString(i), \"c2-\" & toString(i), \"c3-\" & toString(i)]\n  LET first AS net::Address = collections::get(xs, 0)\n  RETURN \"lookup:\" & first.host & \":\" & toString(first.port) & \"/\" & toString(len(churn))\n\
END FUNC\n\
FUNC lookupFails(i AS Integer) AS String\n  LET xs AS List OF net::Address = net::lookup(\"b593-zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz.invalid\", 80) TRAP(e)\n    RETURN \"lookup:\" & describe(e)\n  END TRAP\n  RETURN \"resolved:\" & toString(len(xs))\n\
END FUNC\n\
SUB main()\n  LET okPath AS String = fs::pathJoin([fs::tempDirectory(), \"b593_values_probe.txt\"])\n  fs::writeText(okPath, \"hello\")\n  MUT firsts AS List OF String = []\n  MUT mismatches AS Integer = 0\n  MUT i AS Integer = 0\n  WHILE i < 500\n    LET r0 AS String = openMissing(i)\n    LET r1 AS String = toString(reraiseMissing(i)) TRAP(e1)\n      RECOVER \"reraised:\" & describe(e1)\n    END TRAP\n    LET r2 AS String = hoisted(i)\n    LET r3 AS String = refused(i)\n    LET r4 AS String = userRes(i)\n    LET r5 AS String = okOpen(okPath)\n    LET r6 AS String = lookupOk(i)\n    LET r7 AS String = lookupFails(i)\n    LET junk AS List OF String = [r0 & \"#\", r1 & \"#\", r2 & \"#\", r3 & \"#\", r4 & \"#\", r5 & \"#\", r6 & \"#\", r7 & \"#\"]\n    LET row AS List OF String = [r0, r1, r2, r3, r4, r5, r6, r7]\n    IF i = 0 THEN\n      firsts = row\n    ELSE\n      MUT k AS Integer = 0\n      WHILE k < len(row)\n        IF collections::get(row, k) <> collections::get(firsts, k) THEN\n          mismatches = mismatches + 1\n        END IF\n        k = k + 1\n      END WHILE\n    END IF\n    mismatches = mismatches + len(junk) - 8\n    i = i + 1\n  END WHILE\n  FOR EACH line IN firsts\n    io::print(line)\n  NEXT\n  io::print(\"mismatches=\" & toString(mismatches))\n\
END SUB\n";

/// `B593_VALUES`' stdout on the base compiler (04c81a605), which released no
/// wrapper at all and so could not have read one back freed.
const B593_VALUES_EXPECTED: &str =
    "open:77030001|Filesystem path does not exist.|src/main.mfb:10\n\
reraised:77030001|Filesystem path does not exist.|src/main.mfb:17\n\
hoisted:77030001|Filesystem path does not exist.|src/main.mfb:30\n\
tcp:77070003|Network operation failed before a connection was established.|src/main.mfb:37\n\
user:7|opener-0|src/main.mfb:45\n\
ok:hello\n\
lookup:127.0.0.1:80/3\n\
lookup:77070002|Network host name or address could not be resolved.|src/main.mfb:74\n\
mismatches=0";

#[cfg(unix)]
#[test]
fn every_failing_resource_call_still_reports_its_error_and_origin() {
    let project = common::temp_project("b593_values", B593_VALUES);
    let exe = common::build_project(&project);
    // A use-after-free is not deterministic: a freed wrapper only reads back
    // wrong once the arena has handed its bytes to a later allocation.
    for run in 1..=10 {
        let output = std::process::Command::new(&exe)
            .output()
            .expect("run the bug-593 value probe");
        assert!(
            output.status.success(),
            "run {run}: {}\n{}",
            common::exit_description(&output.status),
            String::from_utf8_lossy(&output.stdout)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            B593_VALUES_EXPECTED,
            "run {run}: a trapped error or a trapped value read back wrong. bug-593 \
             releases the `Result` wrapper an inline `TRAP` built, which holds the \
             trapped `Error` inline — a changed message or origin means something \
             still read the wrapper after its scope drop"
        );
    }
    let _ = std::fs::remove_dir_all(&project);
}

// ---------------------------------------------------------------- bug-599

/// bug-599 positive pin. The `net::Address` builder now frees its `inet_ntop`
/// scratch buffer, and `net::lookup` frees the temporary record whose two words it
/// copies into the list, so the failure direction is a block freed while an
/// `Address` still points at it: a host that reads back empty or garbled, or a
/// port read from a clobbered record. The builder parks the record pointer across
/// the free and writes the PORT after it, so the port is the sharpest witness.
///
/// Every value is read back after churn that would recycle a freed block:
///
/// * `kept` — an element copied out of a list that was the callee's local, read
///   after 500 further lookups;
/// * `total` — each element read both by iteration and by index inside the loop;
/// * `iter` — the plain `FOR EACH` output;
/// * `tcp`/`datagram` — the builder's other callers (`localAddress`,
///   `udp::receive`'s nested `from`);
/// * `trapped`/`propagated` — bug-593's error shapes on a failing lookup, which
///   must still report their code and origin line.
///
/// The error MESSAGE is not asserted: resolver wording is the platform's. Measured
/// identical, line for line, against the compiler before the change.
#[cfg(any(target_os = "macos", target_os = "linux"))]
const SHAPE_599_ADDRESS_VALUES: &str = "IMPORT io\n\
IMPORT net\n\
IMPORT tcp\n\
IMPORT udp\n\
IMPORT collections\n\
FUNC firstOf(host AS String, port AS Integer) AS net::Address\n\
  LET found = net::lookup(host, port)\n\
  RETURN collections::get(found, 0)\n\
END FUNC\n\
FUNC failing(host AS String) AS Integer\n\
  LET xs = net::lookup(host)\n\
  RETURN len(xs)\n\
END FUNC\n\
FUNC main AS Integer\n\
  LET kept = firstOf(\"127.0.0.1\", 8080)\n\
  MUT total AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < 500\n\
    LET xs = net::lookup(\"127.0.0.1\", 443)\n\
    FOR EACH a IN xs\n\
      total = total + len(a.host) + a.port\n\
    NEXT\n\
    LET e0 = collections::get(xs, 0)\n\
    total = total + e0.port\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"kept \" & kept.host & \":\" & toString(kept.port))\n\
  io::print(\"total \" & toString(total))\n\
  FOR EACH a IN net::lookup(\"127.0.0.1\", 53)\n\
    io::print(\"iter \" & a.host & \":\" & toString(a.port))\n\
  NEXT\n\
  RES server = tcp::listen(\"127.0.0.1\", 0)\n\
  MUT stable AS Boolean = TRUE\n\
  LET first = tcp::localAddress(server)\n\
  MUT j AS Integer = 0\n\
  WHILE j < 500\n\
    LET b = tcp::localAddress(server)\n\
    IF b.port <> first.port OR b.host <> \"127.0.0.1\" THEN\n\
      stable = FALSE\n\
    END IF\n\
    j = j + 1\n\
  END WHILE\n\
  io::print(\"tcp \" & first.host & \" stable=\" & toString(stable) & \" portOk=\" & toString(first.port > 0))\n\
  RES sock = udp::bind(\"127.0.0.1\", 0)\n\
  LET at = udp::localAddress(sock)\n\
  RES peer = udp::bind(\"127.0.0.1\", 0)\n\
  udp::send(peer, at, \"ping\")\n\
  LET dg = udp::receive(sock, 5000)\n\
  io::print(\"datagram \" & dg.from.host & \" \" & toString(len(dg.bytes)) & \" portOk=\" & toString(dg.from.port > 0))\n\
  LET bad = net::lookup(\"no-such-host.invalid\") TRAP(e)\n\
    io::print(\"trapped code=\" & toString(e.code))\n\
    RECOVER []\n\
  END TRAP\n\
  io::print(\"bad \" & toString(len(bad)))\n\
  LET n = failing(\"no-such-host.invalid\") TRAP(e2)\n\
    io::print(\"propagated code=\" & toString(e2.code) & \" line=\" & toString(e2.source.line))\n\
    RECOVER -1\n\
  END TRAP\n\
  io::print(\"n \" & toString(n))\n\
  RETURN 0\n\
END FUNC\n";

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn every_address_reads_back_after_its_builder_scratch_is_freed() {
    let expected = [
        "kept 127.0.0.1:8080".to_string(),
        // 500 iterations x (len("127.0.0.1") + 443 iterated + 443 indexed).
        format!("total {}", 500 * (9 + 443 + 443)),
        "iter 127.0.0.1:53".to_string(),
        "tcp 127.0.0.1 stable=TRUE portOk=TRUE".to_string(),
        "datagram 127.0.0.1 4 portOk=TRUE".to_string(),
        "trapped code=77070002".to_string(),
        "bad 0".to_string(),
        "propagated code=77070002 line=11".to_string(),
        "n -1".to_string(),
    ]
    .join("\n");
    let project = common::temp_project("b599_address_values", SHAPE_599_ADDRESS_VALUES);
    let exe = common::build_project(&project);
    for run in 1..=5 {
        let output = std::process::Command::new(&exe)
            .current_dir(&project)
            .output()
            .expect("run the net::Address read-back probe");
        assert!(
            output.status.success(),
            "run {run}: {}\n{}",
            common::exit_description(&output.status),
            String::from_utf8_lossy(&output.stdout)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            expected,
            "run {run}: a net::Address read back wrong. bug-599 frees the address \
             builder's scratch buffer and lookup's temporary record — a wrong \
             `host` means a freed block was still referenced, a wrong `port` that \
             the record pointer was not restored across the free"
        );
    }
    let _ = std::fs::remove_dir_all(&project);
}

// ---------------------------------------------------------------- bug-599 / plan-132

/// plan-132 (bug-599): once `net::Address` and `udp::Datagram` use the ordinary flat
/// record layout, a `List OF net::Address`, a lone `net::Address`, a `Datagram` and
/// the host `String` inside each have an owner and are freed. Peak RSS on the
/// compiler before plan-132 (macOS, 200k -> 400k iterations): `net::lookup`
/// 99.8 -> 198.3 MB, `tcp::localAddress` 38.0 -> 74.9 MB, a user-built list of two
/// copies 196.4 -> 391.8 MB.
#[cfg(any(target_os = "macos", target_os = "linux"))]
const SHAPE_599_LOOKUP_LOOP: &str = "IMPORT io\nIMPORT net\n\
FUNC main AS Integer\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET xs = net::lookup(\"127.0.0.1\")\n\
    n = n + len(xs)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n \" & toString(n))\n\
  RETURN 0\n\
END FUNC\n";

#[cfg(any(target_os = "macos", target_os = "linux"))]
const SHAPE_599_LOCAL_ADDRESS_LOOP: &str = "IMPORT io\nIMPORT net\nIMPORT tcp\n\
FUNC main AS Integer\n\
  RES server = tcp::listen(\"127.0.0.1\", 0)\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET b = tcp::localAddress(server)\n\
    n = n + len(b.host)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n \" & toString(n))\n\
  RETURN 0\n\
END FUNC\n";

#[cfg(any(target_os = "macos", target_os = "linux"))]
const SHAPE_599_USER_BUILT_LIST_LOOP: &str = "IMPORT io\nIMPORT net\nIMPORT collections\n\
FUNC main AS Integer\n\
  LET a = collections::get(net::lookup(\"127.0.0.1\"), 0)\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    MUT ys AS List OF net::Address = []\n\
    ys = collections::append(ys, a)\n\
    ys = collections::append(ys, a)\n\
    n = n + len(ys)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n \" & toString(n))\n\
  RETURN 0\n\
END FUNC\n";

#[cfg(any(target_os = "macos", target_os = "linux"))]
const SHAPE_599_DATAGRAM_LOOP: &str = "IMPORT io\nIMPORT net\nIMPORT udp\n\
FUNC main AS Integer\n\
  RES sock = udp::bind(\"127.0.0.1\", 0)\n\
  LET at = udp::localAddress(sock)\n\
  RES peer = udp::bind(\"127.0.0.1\", 0)\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    udp::send(peer, at, \"ping\")\n\
    LET dg = udp::receive(sock, 64)\n\
    n = n + len(dg.bytes) + len(dg.from.host)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n \" & toString(n))\n\
  RETURN 0\n\
END FUNC\n";

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn a_looped_net_lookup_runs_at_constant_rss() {
    assert_flat("b599_lookup_loop", SHAPE_599_LOOKUP_LOOP, 200_000, 400_000);
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn a_looped_local_address_runs_at_constant_rss() {
    assert_flat(
        "b599_local_address_loop",
        SHAPE_599_LOCAL_ADDRESS_LOOP,
        200_000,
        400_000,
    );
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn a_looped_user_built_address_list_runs_at_constant_rss() {
    assert_flat(
        "b599_user_built_list_loop",
        SHAPE_599_USER_BUILT_LIST_LOOP,
        200_000,
        400_000,
    );
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn a_looped_udp_receive_runs_at_constant_rss() {
    assert_flat(
        "b599_datagram_loop",
        SHAPE_599_DATAGRAM_LOOP,
        200_000,
        400_000,
    );
}

// ---------------------------------------------------------------- shape C / plan-134-G

/// bug-536 shape C, union form: a recursive-union value bound each iteration. Before
/// plan-134-G nothing freed a value whose type reaches a type cycle — 52.7 MB at 400k
/// iterations, 104.3 MB at 800k (plan-134-A §2.2, `c_union_rss`).
const SHAPE_C_UNION_BIND: &str = r#"IMPORT io
IMPORT json
SUB main()
  MUT i AS Integer = 0
  WHILE i < {N}
    LET v AS json::Json = json::JsonNull[NOTHING]
    i = i + 1
  END WHILE
  io::print("done")
END SUB
"#;

/// bug-536 shape C, record form (`c_record_rss`: 105.1 MB at 400k, 209.1 MB at 800k).
const SHAPE_C_RECORD_BIND: &str = r#"IMPORT io
TYPE Node
  kids AS List OF Node
  tag AS Integer
END TYPE
SUB main()
  MUT i AS Integer = 0
  MUT acc AS Integer = 0
  WHILE i < {N}
    LET nd AS Node = Node[kids := [], tag := i]
    acc = acc + nd.tag
    i = i + 1
  END WHILE
  io::print("acc=" & toString(acc))
END SUB
"#;

/// A recursive local overwritten each iteration: the old graph is the `Assign` free's.
const SHAPE_C_REASSIGN: &str = r#"IMPORT io
TYPE Node
  kids AS List OF Node
  tag AS Integer
END TYPE
SUB main()
  MUT cur AS Node = Node[kids := [], tag := 0]
  MUT i AS Integer = 0
  WHILE i < {N}
    cur = Node[kids := [], tag := i]
    i = i + 1
  END WHILE
  io::print("tag=" & toString(cur.tag))
END SUB
"#;

/// A recursive global overwritten each iteration: the `StoreGlobal` old-value free.
const SHAPE_C_GLOBAL: &str = r#"IMPORT io
TYPE Node
  kids AS List OF Node
  tag AS Integer
END TYPE
MUT GN AS Node = Node[kids := [], tag := 0]
SUB main()
  MUT i AS Integer = 0
  WHILE i < {N}
    GN = Node[kids := [], tag := i]
    i = i + 1
  END WHILE
  io::print("tag=" & toString(GN.tag))
END SUB
"#;

/// An unbound recursive temporary: a function's fresh result consumed as another call's
/// argument inside one expression, freed at the end of the statement. (The
/// `len(json::stringify(json::parse(t)))` form of this case belongs to plan-134-H: a
/// `--debug` build leaks the same 2 912 B per iteration whether `json::parse`'s result is
/// bound or unbound — blocks left inside json's list-building helpers, not the temp.)
const SHAPE_C_UNBOUND_TEMP: &str = r#"IMPORT io
TYPE Node
  kids AS List OF Node
  tag AS Integer
END TYPE
FUNC mk(t AS Integer) AS Node
  RETURN Node[kids := [Node[kids := [], tag := t]], tag := t]
END FUNC
FUNC total(n AS Node) AS Integer
  MUT sum AS Integer = n.tag
  FOR EACH k IN n.kids
    sum = sum + total(k)
  NEXT
  RETURN sum
END FUNC
SUB main()
  MUT acc AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < {N}
    acc = acc + total(mk(i))
    i = i + 1
  END WHILE
  io::print("acc=" & toString(acc))
END SUB
"#;

/// A recursive value captured by a non-escaping closure: the closure drop frees the
/// captured graph, and the capture's source is either copied or moved out of.
const SHAPE_C_CLOSURE: &str = r#"IMPORT io
TYPE Node
  kids AS List OF Node
  tag AS Integer
END TYPE
SUB main()
  MUT acc AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < {N}
    LET cap AS Node = Node[kids := [], tag := i]
    LET f AS FUNC(Integer) AS Integer = LAMBDA(k AS Integer) -> k + cap.tag
    acc = acc + f(1)
    i = i + 1
  END WHILE
  io::print("acc=" & toString(acc))
END SUB
"#;

/// bug-538's owned copy out of a container: `collections::get` of a recursive element.
const SHAPE_C_GET: &str = r#"IMPORT io
IMPORT collections
TYPE Node
  kids AS List OF Node
  tag AS Integer
END TYPE
SUB main()
  LET xs AS List OF Node = [Node[kids := [], tag := 7]]
  MUT acc AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < {N}
    LET e AS Node = collections::get(xs, 0)
    acc = acc + e.tag
    i = i + 1
  END WHILE
  io::print("acc=" & toString(acc))
END SUB
"#;

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn a_looped_recursive_union_bind_runs_at_constant_rss() {
    assert_flat(
        "c_recursive_union_bind",
        SHAPE_C_UNION_BIND,
        400_000,
        800_000,
    );
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn a_looped_recursive_record_bind_runs_at_constant_rss() {
    assert_flat(
        "c_recursive_record_bind",
        SHAPE_C_RECORD_BIND,
        400_000,
        800_000,
    );
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn a_looped_recursive_reassignment_runs_at_constant_rss() {
    assert_flat("c_recursive_reassign", SHAPE_C_REASSIGN, 400_000, 800_000);
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn a_looped_recursive_global_overwrite_runs_at_constant_rss() {
    assert_flat("c_recursive_global", SHAPE_C_GLOBAL, 400_000, 800_000);
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn a_looped_unbound_recursive_temp_runs_at_constant_rss() {
    assert_flat(
        "c_recursive_unbound_temp",
        SHAPE_C_UNBOUND_TEMP,
        400_000,
        800_000,
    );
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn a_looped_recursive_closure_capture_runs_at_constant_rss() {
    assert_flat("c_recursive_closure", SHAPE_C_CLOSURE, 400_000, 800_000);
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn a_looped_recursive_get_result_runs_at_constant_rss() {
    assert_flat("c_recursive_get", SHAPE_C_GET, 400_000, 800_000);
}

// ---------------------------------------------------------------- shape C / plan-134-H

/// plan-134-H: parsing the same 480 003-byte JSON document K times costs its tree once.
/// `tools/recursive-value-bench/programs/json_repeat` with K = {N}: 204 / 402 / 799 MB at
/// K = 1 / 2 / 4 before plan-134 (plan-134-A §2.1).
const SHAPE_C_JSON_REPEAT: &str = r#"IMPORT io
IMPORT json
SUB main()
  MUT text AS String = "["
  MUT j AS Integer = 0
  WHILE j < 20000
    text = text & "{\u{22}a\u{22}:[1,2,3],\u{22}b\u{22}:\u{22}xyz\u{22}},"
    j = j + 1
  END WHILE
  text = text & "0]"
  MUT i AS Integer = 0
  WHILE i < {N}
    LET v AS json::Json = json::parse(text)
    i = i + 1
  END WHILE
  io::print("bytes=" & toString(len(text)))
END SUB
"#;

/// plan-134-H: a 100 000-character `regex::findAll` repeated K times costs its matcher graphs
/// once. `tools/recursive-value-bench/programs/regex_repeat` with K = {N}: 345 / 673 / 1329 MB
/// at K = 1 / 2 / 4 before plan-134.
const SHAPE_C_REGEX_REPEAT: &str = r#"IMPORT io
IMPORT regex
SUB main()
  MUT subject AS String = ""
  MUT j AS Integer = 0
  WHILE j < 10000
    subject = subject & "abcab1234 "
    j = j + 1
  END WHILE
  MUT hits AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < {N}
    hits = len(regex::findAll(subject, "[a-c]+[0-9]+"))
    i = i + 1
  END WHILE
  io::print("hits=" & toString(hits))
END SUB
"#;

/// plan-134-H (from plan-134-G): the json form of the unbound recursive temp. After G a
/// `--debug` build leaves `live_bytes 2912000` per 1 000 iterations whether `json::parse`'s
/// result is bound or not — blocks left inside json's list-building helpers.
const SHAPE_C_JSON_UNBOUND_TEMP: &str = r#"IMPORT io
IMPORT json
SUB main()
  LET t AS String = "[1,{\u{22}a\u{22}:[2,3]}]"
  MUT acc AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < {N}
    acc = acc + len(json::stringify(json::parse(t)))
    i = i + 1
  END WHILE
  io::print("acc=" & toString(acc))
END SUB
"#;

/// plan-134 (final gate): an inline-`TRAP`ped `json::parse` of a document with nested
/// children, both the Ok and the RECOVER path. The `Result` wrapper holds the value's top
/// block inline and the graph below it: after plan-134-G the callee's block was
/// graph-dropped under the wrapper (a use-after-free, `rt_union_trap_bind_default`), and
/// the Ok wrapper — never released for a block payload (bug-593 `ErrorOnly`) — kept its
/// graph for the life of the process.
const SHAPE_C_JSON_TRAP_RECOVER: &str = r#"IMPORT io
IMPORT json
SUB main()
  MUT acc AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < {N}
    LET d AS json::Json = json::parse("[1,[2,3],{\u{22}a\u{22}:\u{22}xyz\u{22}}]") TRAP(e)
      RECOVER json::JsonNull[NOTHING]
    END TRAP
    LET bad AS json::Json = json::parse("[oops") TRAP(e)
      RECOVER json::JsonNull[NOTHING]
    END TRAP
    acc = acc + len(json::stringify(d)) + len(json::stringify(bad))
    i = i + 1
  END WHILE
  io::print("acc=" & toString(acc))
END SUB
"#;

/// plan-134 (final gate): an inline-`TRAP` bind of a flat data union. The desugar's slot
/// starts as the union's synthesized default, whose variant record was byte-copied into
/// the union and never freed — 16 B per bind, the same on the pre-plan compiler.
const SHAPE_TRAP_UNION_DEFAULT: &str = r#"IMPORT io
TYPE Circle
  radius AS Integer
END TYPE
TYPE Rect
  w AS Integer
  h AS Integer
END TYPE
UNION Shape
  Circle
  Rect
END UNION
FUNC makeShape(n AS Integer) AS Shape
  IF n < 0 THEN FAIL error(90004440, "negative")
  RETURN Circle[n]
END FUNC
FUNC score(s AS Shape) AS Integer
  MATCH s
    CASE Circle(c)
      RETURN c.radius
    CASE Rect(r)
      RETURN r.w + r.h
  END MATCH
END FUNC
SUB main()
  MUT acc AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < {N}
    LET ok AS Shape = makeShape(7) TRAP(e)
      RECOVER Rect[1, 2]
    END TRAP
    LET bad AS Shape = makeShape(-1) TRAP(e)
      RECOVER Rect[20, 22]
    END TRAP
    acc = acc + score(ok) + score(bad)
    i = i + 1
  END WHILE
  io::print("acc=" & toString(acc))
END SUB
"#;

/// plan-134 (final gate): a fresh temporary passed to a call that FAILs. The statement-end
/// free is jumped past by the call's error exit — 48 B per failure for the list literal,
/// 32 B for the concat, the same on the pre-plan compiler.
const SHAPE_FAILING_CALL_TEMP_ARGUMENT: &str = r#"IMPORT io
FUNC boom(n AS Integer, xs AS List OF Integer) AS Integer
  IF n >= 0 THEN FAIL error(90004440, "boom")
  RETURN n + len(xs)
END FUNC
FUNC boomS(n AS Integer, s AS String) AS Integer
  IF n >= 0 THEN FAIL error(90004440, "boom")
  RETURN n + len(s)
END FUNC
FUNC holder(n AS Integer) AS Integer
  LET v AS Integer = boom(n, [])
  RETURN v + boomS(n, "a" & toString(n))
END FUNC
FUNC tryHolder(n AS Integer) AS Integer
  RETURN holder(n)
  TRAP(e)
    RETURN 1
  END TRAP
END FUNC
FUNC tryHolderS(n AS Integer) AS Integer
  RETURN boomS(n, "a" & toString(n))
  TRAP(e)
    RETURN len(e.message)
  END TRAP
END FUNC
SUB main()
  MUT acc AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < {N}
    acc = acc + tryHolder(i) + tryHolderS(i)
    i = i + 1
  END WHILE
  io::print("acc=" & toString(acc))
END SUB
"#;

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn a_looped_inline_trap_union_bind_runs_at_constant_rss() {
    assert_flat(
        "trap_union_default",
        SHAPE_TRAP_UNION_DEFAULT,
        400_000,
        800_000,
    );
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn a_looped_failing_call_with_a_temp_argument_runs_at_constant_rss() {
    assert_flat(
        "failing_call_temp_argument",
        SHAPE_FAILING_CALL_TEMP_ARGUMENT,
        400_000,
        800_000,
    );
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn a_looped_trapped_recursive_json_parse_runs_at_constant_rss() {
    assert_flat(
        "c_recursive_json_trap_recover",
        SHAPE_C_JSON_TRAP_RECOVER,
        200_000,
        400_000,
    );
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn a_repeated_json_parse_of_one_document_runs_at_constant_rss() {
    assert_flat("c_recursive_json_repeat", SHAPE_C_JSON_REPEAT, 1, 4);
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn a_repeated_regex_find_all_runs_at_constant_rss() {
    assert_flat("c_recursive_regex_repeat", SHAPE_C_REGEX_REPEAT, 1, 4);
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn a_looped_unbound_recursive_json_temp_runs_at_constant_rss() {
    assert_flat(
        "c_recursive_json_unbound_temp",
        SHAPE_C_JSON_UNBOUND_TEMP,
        400_000,
        800_000,
    );
}

// ------------------------------------------------- bug-620 / bug-621: condition temps

/// bug-620 / bug-621: a condition's heap temps are measured on the `--debug` report's main-arena
/// `live_bytes`, which is exact where peak RSS is paged: a program that frees every block it
/// allocates reports the same number at both counts. `{N}` is the call count; `per_call` is the
/// value `g` contributes per call, so the printed total pins the behaviour too.
#[cfg(unix)]
fn condition_temp_report(name: &str, function: &str, call: &str, count: u64) -> (String, u64, u64) {
    let source = format!(
        "IMPORT io\nIMPORT strings\nIMPORT collections\n\
TYPE Tag\n  tag AS String\n  depth AS Integer\nEND TYPE\n\
{function}\n\
SUB main()\n\
  MUT total AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {count}\n\
    total = total + {call}\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"total=\" & toString(total))\n\
END SUB\n"
    );
    let case = format!("{name}_{count}");
    let exe = common::debug_report::build_debug(&case, &source);
    let (stdout, stderr) = common::debug_report::run_ok(&case, &exe);
    let lines = common::debug_report::arena_lines(&case, &stderr);
    (
        stdout.trim().to_string(),
        common::debug_report::counter(&case, &lines, 0, "live_bytes"),
        common::debug_report::counter(&case, &lines, 0, "double_free_skips"),
    )
}

/// Assert `g` frees every condition temp it allocates: equal `live_bytes` at 1000 and 2000
/// calls, no double free skipped, and the right total.
#[cfg(unix)]
fn assert_condition_temps_freed(name: &str, function: &str, call: &str, per_call: i64) {
    let (out_small, live_small, skips_small) = condition_temp_report(name, function, call, 1000);
    let (out_large, live_large, skips_large) = condition_temp_report(name, function, call, 2000);
    assert_eq!(
        out_small,
        format!("total={}", per_call * 1000),
        "{name}: wrong value"
    );
    assert_eq!(
        out_large,
        format!("total={}", per_call * 2000),
        "{name}: wrong value"
    );
    assert_eq!(
        (skips_small, skips_large),
        (0, 0),
        "{name}: the arena skipped a double free"
    );
    assert_eq!(
        live_small,
        live_large,
        "{name}: main-arena live_bytes grew {} B between 1000 and 2000 calls — a condition \
         temp is not freed on some path out of its statement",
        live_large.saturating_sub(live_small)
    );
}

/// bug-620's reproduction: the one-line `IF … THEN RETURN` leaves past the `IF`'s drop.
#[cfg(unix)]
#[test]
fn an_if_condition_temp_is_freed_when_the_branch_returns() {
    assert_condition_temps_freed(
        "cond_if_return",
        "FUNC g(s AS String) AS Integer\n  IF strings::lower(s) <> \"zz\" THEN RETURN 0\n  RETURN 1\nEND FUNC",
        "g(\"p\")",
        0,
    );
}

/// bug-620 `r7_block`: the block form.
#[cfg(unix)]
#[test]
fn a_block_if_condition_temp_is_freed_when_the_branch_returns() {
    assert_condition_temps_freed(
        "cond_if_block_return",
        "FUNC g(s AS String) AS Integer\n  IF strings::lower(s) <> \"zz\" THEN\n    RETURN 1\n  END IF\n  RETURN 0\nEND FUNC",
        "g(\"p\")",
        1,
    );
}

/// bug-620: an `ELSEIF` is a nested `If` in the else body — same hole.
#[cfg(unix)]
#[test]
fn an_elseif_condition_temp_is_freed_when_the_branch_returns() {
    assert_condition_temps_freed(
        "cond_elseif_return",
        "FUNC g(s AS String) AS Integer\n  IF len(s) > 5 THEN\n    RETURN 2\n  ELSEIF strings::lower(s) <> \"zz\" THEN\n    RETURN 1\n  END IF\n  RETURN 0\nEND FUNC",
        "g(\"p\")",
        1,
    );
}

/// bug-620: the returned value is itself a fresh block, so the enclosing condition temp is
/// freed alongside an escaping value.
#[cfg(unix)]
#[test]
fn an_if_condition_temp_is_freed_when_the_branch_returns_a_fresh_string() {
    assert_condition_temps_freed(
        "cond_if_return_fresh",
        "FUNC h(s AS String) AS String\n  IF strings::lower(s) <> \"zz\" THEN RETURN strings::upper(s) & \"!\"\n  RETURN s\nEND FUNC",
        "len(h(\"p\"))",
        2,
    );
}

/// bug-620 `ct_close`: a record copy in the condition, left by `EXIT DO`.
#[cfg(unix)]
#[test]
fn an_if_condition_record_temp_is_freed_on_exit_do() {
    assert_condition_temps_freed(
        "cond_if_exit_do",
        "FUNC g(name AS String) AS Integer\n  LET stack AS List OF Tag = [Tag[\"a\", 1], Tag[\"b\", 2], Tag[\"c\", 3]]\n  MUT k AS Integer = -1\n  MUT s AS Integer = 2\n  DO WHILE s >= 0\n    IF collections::get(stack, s).tag = name THEN\n      k = s\n      EXIT DO\n    END IF\n    s = s - 1\n  LOOP\n  RETURN k\nEND FUNC",
        "g(\"b\")",
        1,
    );
}

/// bug-620: `EXIT FOR`.
#[cfg(unix)]
#[test]
fn an_if_condition_temp_is_freed_on_exit_for() {
    assert_condition_temps_freed(
        "cond_if_exit_for",
        "FUNC g(s AS String) AS Integer\n  MUT k AS Integer = 0\n  FOR j = 1 TO 3\n    IF strings::lower(s) = \"p\" THEN EXIT FOR\n    k = k + j\n  NEXT\n  RETURN k\nEND FUNC",
        "g(\"P\")",
        0,
    );
}

/// bug-620: `EXIT WHILE`.
#[cfg(unix)]
#[test]
fn an_if_condition_temp_is_freed_on_exit_while() {
    assert_condition_temps_freed(
        "cond_if_exit_while",
        "FUNC g(s AS String) AS Integer\n  MUT k AS Integer = 0\n  WHILE k < 3\n    IF strings::lower(s) = \"p\" THEN EXIT WHILE\n    k = k + 1\n  END WHILE\n  RETURN k\nEND FUNC",
        "g(\"P\")",
        0,
    );
}

/// bug-620: `CONTINUE WHILE`, taken on every pass.
#[cfg(unix)]
#[test]
fn an_if_condition_temp_is_freed_on_continue_while() {
    assert_condition_temps_freed(
        "cond_if_continue_while",
        "FUNC g(s AS String) AS Integer\n  MUT k AS Integer = 0\n  MUT n AS Integer = 0\n  WHILE k < 3\n    k = k + 1\n    IF strings::lower(s) = \"p\" THEN CONTINUE WHILE\n    n = n + 1\n  END WHILE\n  RETURN k + n\nEND FUNC",
        "g(\"P\")",
        3,
    );
}

/// bug-620: `CONTINUE FOR`.
#[cfg(unix)]
#[test]
fn an_if_condition_temp_is_freed_on_continue_for() {
    assert_condition_temps_freed(
        "cond_if_continue_for",
        "FUNC g(s AS String) AS Integer\n  MUT n AS Integer = 0\n  FOR j = 1 TO 3\n    IF strings::lower(s) = \"p\" THEN CONTINUE FOR\n    n = n + 1\n  NEXT\n  RETURN n\nEND FUNC",
        "g(\"P\")",
        0,
    );
}

/// bug-620: a `CONTINUE WHILE` from inside a `FOR` leaves two `IF`s and the `FOR`, and must
/// free both conditions' temps.
#[cfg(unix)]
#[test]
fn a_continue_out_of_a_nested_loop_frees_every_enclosing_condition_temp() {
    assert_condition_temps_freed(
        "cond_nested_continue",
        "FUNC g(s AS String) AS Integer\n  MUT k AS Integer = 0\n  WHILE k < 3\n    k = k + 1\n    IF strings::lower(s) = \"p\" THEN\n      FOR j = 1 TO 2\n        IF strings::upper(s) = \"P\" THEN CONTINUE WHILE\n      NEXT\n    END IF\n  END WHILE\n  RETURN k\nEND FUNC",
        "g(\"P\")",
        3,
    );
}

/// bug-621's reproduction: `DO WHILE`, nine passes per call.
#[cfg(unix)]
#[test]
fn a_do_while_condition_temp_is_freed_on_every_pass() {
    assert_condition_temps_freed(
        "cond_do_while",
        "FUNC g(s AS String) AS Integer\n  LET n AS Integer = len(s)\n  MUT i AS Integer = 0\n  DO WHILE i < n AND strings::mid(s, i, 1) <> \"=\"\n    i = i + 1\n  LOOP\n  RETURN i\nEND FUNC",
        "g(\"abcdefgh=\")",
        8,
    );
}

/// bug-621 `r10_whileend`: `WHILE … END WHILE`.
#[cfg(unix)]
#[test]
fn a_while_condition_temp_is_freed_on_every_pass() {
    assert_condition_temps_freed(
        "cond_while",
        "FUNC g(s AS String) AS Integer\n  LET n AS Integer = len(s)\n  MUT i AS Integer = 0\n  WHILE i < n AND strings::mid(s, i, 1) <> \"=\"\n    i = i + 1\n  END WHILE\n  RETURN i\nEND FUNC",
        "g(\"abcdefgh=\")",
        8,
    );
}

/// bug-621: `DO … LOOP UNTIL` tests its condition after the body.
#[cfg(unix)]
#[test]
fn a_loop_until_condition_temp_is_freed_on_every_pass() {
    assert_condition_temps_freed(
        "cond_loop_until",
        "FUNC g(s AS String) AS Integer\n  MUT i AS Integer = 0\n  DO\n    i = i + 1\n  LOOP UNTIL strings::mid(s, i, 1) = \"=\"\n  RETURN i\nEND FUNC",
        "g(\"abcdefgh=\")",
        8,
    );
}

/// bug-621: a record copy in the condition.
#[cfg(unix)]
#[test]
fn a_while_condition_record_temp_is_freed_on_every_pass() {
    assert_condition_temps_freed(
        "cond_while_record",
        "FUNC g(name AS String) AS Integer\n  LET xs AS List OF Tag = [Tag[\"a\", 1], Tag[\"b\", 2], Tag[\"c\", 3], Tag[\"d\", 4]]\n  MUT i AS Integer = 0\n  WHILE i < 4 AND collections::get(xs, i).tag <> name\n    i = i + 1\n  END WHILE\n  RETURN i\nEND FUNC",
        "g(\"d\")",
        3,
    );
}

/// POSITIVE pins that were flat before bug-620/621: a false condition falls through to the
/// `IF`'s own drop; a `LET`-bound temp is an owned local; a `MATCH` scrutinee is bound to a
/// `$match` local; a `FOR … TO` bound is evaluated once into a synthetic local.
#[cfg(unix)]
#[test]
fn condition_temps_that_already_had_an_owner_stay_flat() {
    assert_condition_temps_freed(
        "cond_if_false",
        "FUNC g(s AS String) AS Integer\n  IF strings::lower(s) <> \"zz\" THEN RETURN 0\n  RETURN 1\nEND FUNC",
        "g(\"ZZ\")",
        1,
    );
    assert_condition_temps_freed(
        "cond_bound_first",
        "FUNC g(s AS String) AS Integer\n  LET l AS String = strings::lower(s)\n  IF l <> \"zz\" THEN RETURN 0\n  RETURN 1\nEND FUNC",
        "g(\"p\")",
        0,
    );
    assert_condition_temps_freed(
        "cond_match_scrutinee",
        "FUNC g(s AS String) AS Integer\n  MATCH strings::lower(s)\n    CASE \"p\"\n      RETURN 1\n    CASE ELSE\n      RETURN 0\n  END MATCH\n  RETURN 2\nEND FUNC",
        "g(\"P\")",
        1,
    );
    assert_condition_temps_freed(
        "cond_for_bound",
        "FUNC g(s AS String) AS Integer\n  MUT n AS Integer = 0\n  FOR j = 1 TO len(strings::lower(s))\n    n = n + j\n  NEXT\n  RETURN n\nEND FUNC",
        "g(\"abcdefgh\")",
        36,
    );
}

/// bug-620, found while fixing it: a `FOR EACH` over a fresh collection keeps it as a pending
/// temp of the loop statement, so a `RETURN` from the body — which never reaches that
/// statement's end drop — leaked the whole list (192 B per call for a three-item split). The
/// returned item is a copy, so the list is freed under the escaping-value guard.
#[cfg(unix)]
#[test]
fn a_return_from_a_for_each_frees_its_fresh_collection() {
    assert_condition_temps_freed(
        "for_each_return_item",
        "FUNC pick(s AS String) AS String\n  FOR EACH x IN strings::split(s, \",\")\n    IF strings::lower(x) = \"b\" THEN RETURN x\n  NEXT\n  RETURN \"\"\nEND FUNC",
        "len(pick(\"a,B,c\"))",
        1,
    );
    assert_condition_temps_freed(
        "for_each_return_derived",
        "FUNC g(s AS String) AS Integer\n  FOR EACH x IN strings::split(s, \",\")\n    IF x = \"b\" THEN RETURN len(x & \"!\")\n  NEXT\n  RETURN 0\nEND FUNC",
        "g(\"a,b,c\")",
        2,
    );
}

/// bug-620 × bug-621: a `RETURN` from a `WHILE` body whose condition allocates on every pass
/// returns a fresh value built from the same input.
#[cfg(unix)]
#[test]
fn a_return_from_a_while_with_a_condition_temp_frees_every_block() {
    assert_condition_temps_freed(
        "while_condition_return",
        "FUNC g(s AS String) AS String\n  MUT i AS Integer = 0\n  WHILE strings::mid(s, i, 1) <> \"=\"\n    IF strings::mid(s, i, 1) = \"c\" THEN RETURN strings::left(s, i)\n    i = i + 1\n  END WHILE\n  RETURN s\nEND FUNC",
        "len(g(\"abcd=\"))",
        2,
    );
}

// ------------------------------------------------- bug-622: thread start + waitFor

/// bug-622: a `thread::start` + `thread::waitFor` loop, measured on the `--debug` report's
/// main-arena `live_bytes` (exact, where peak RSS is paged). Returns the program's output and
/// `live_bytes`; `{N}` is the thread count. A skipped double free fails the case outright.
#[cfg(unix)]
fn thread_loop_report(name: &str, source: &str, count: u64) -> (String, u64) {
    let program = source.replace("{N}", &count.to_string());
    let case = format!("{name}_{count}");
    let exe = common::debug_report::build_debug(&case, &program);
    let (stdout, stderr) = common::debug_report::run_ok(&case, &exe);
    let lines = common::debug_report::arena_lines(&case, &stderr);
    assert_eq!(
        common::debug_report::counter(&case, &lines, 0, "double_free_skips"),
        0,
        "{case}: the arena skipped a double free"
    );
    (
        stdout.trim().to_string(),
        common::debug_report::counter(&case, &lines, 0, "live_bytes"),
    )
}

/// Main-arena `live_bytes` growth between 50 and 100 threads, after checking both outputs.
#[cfg(unix)]
fn thread_loop_growth(name: &str, source: &str, small_out: &str, large_out: &str) -> u64 {
    let (out_small, live_small) = thread_loop_report(name, source, 50);
    let (out_large, live_large) = thread_loop_report(name, source, 100);
    assert_eq!(out_small, small_out, "{name}: wrong value at 50 threads");
    assert_eq!(out_large, large_out, "{name}: wrong value at 100 threads");
    live_large.saturating_sub(live_small)
}

/// The scalar result: nothing is copied back, so all growth is the thread's own plumbing —
/// the control block, the worker arena state and the four queues `thread::start` carves out of
/// the parent arena.
const B622_INTEGER_RESULT: &str = "IMPORT io\nIMPORT thread\n\
ISOLATED FUNC work(w AS ThreadWorker OF String TO Integer, seed AS String) AS Integer\n  RETURN len(seed)\nEND FUNC\n\
SUB main()\n\
  MUT total AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET t AS Thread OF String TO Integer = thread::start(work, \"abc\")\n\
    total = total + thread::waitFor(t)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"total=\" & toString(total))\n\
END SUB\n";

/// A handle whose worker has finished, dropped at scope exit with no `waitFor`.
const B622_COMPLETED_DROP: &str = "IMPORT io\nIMPORT os\nIMPORT thread\n\
ISOLATED FUNC work(w AS ThreadWorker OF String TO Integer, seed AS String) AS Integer\n  RETURN len(seed)\nEND FUNC\n\
SUB main()\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET t AS Thread OF String TO Integer = thread::start(work, \"abc\")\n\
    WHILE thread::isRunning(t)\n      os::sleep(1)\n    END WHILE\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"done=\" & toString(i))\n\
END SUB\n";

const B622_STRING_BOUND: &str = "IMPORT io\nIMPORT thread\n\
ISOLATED FUNC work(w AS ThreadWorker OF String TO String, seed AS String) AS String\n  RETURN seed & \"-\" & seed\nEND FUNC\n\
SUB main()\n\
  MUT total AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET t AS Thread OF String TO String = thread::start(work, \"abc\")\n\
    LET r AS String = thread::waitFor(t)\n\
    total = total + len(r)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"total=\" & toString(total))\n\
END SUB\n";

const B622_STRING_UNBOUND: &str = "IMPORT io\nIMPORT thread\n\
ISOLATED FUNC work(w AS ThreadWorker OF String TO String, seed AS String) AS String\n  RETURN seed & \"-\" & seed\nEND FUNC\n\
SUB main()\n\
  MUT total AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET t AS Thread OF String TO String = thread::start(work, \"abc\")\n\
    total = total + len(thread::waitFor(t))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"total=\" & toString(total))\n\
END SUB\n";

const B622_STRING_TRAPPED: &str = "IMPORT io\nIMPORT thread\n\
ISOLATED FUNC work(w AS ThreadWorker OF String TO String, seed AS String) AS String\n  RETURN seed & \"-\" & seed\nEND FUNC\n\
SUB main()\n\
  MUT total AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET t AS Thread OF String TO String = thread::start(work, \"abc\")\n\
    LET r AS String = thread::waitFor(t) TRAP(e)\n      RECOVER \"\"\n    END TRAP\n\
    total = total + len(r)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"total=\" & toString(total))\n\
END SUB\n";

const B622_UNION: &str = "IMPORT io\nIMPORT thread\n\
TYPE TwEl\n  tag AS String\n  kids AS List OF TwNode\nEND TYPE\n\
TYPE TwText\n  text AS String\nEND TYPE\n\
UNION TwNode\n  TwEl\n  TwText\nEND UNION\n\
ISOLATED FUNC work(w AS ThreadWorker OF String TO TwNode, seed AS String) AS TwNode\n\
  LET leaf AS TwNode = TwText[seed & \"!\"]\n\
  LET root AS TwNode = TwEl[\"div\", [leaf, leaf]]\n\
  RETURN root\n\
END FUNC\n\
SUB main()\n\
  MUT count AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET t AS Thread OF String TO TwNode = thread::start(work, \"abc\")\n\
    LET r AS TwNode = thread::waitFor(t)\n\
    MATCH r\n      CASE TwEl(e)\n        count = count + len(e.kids)\n      CASE ELSE\n    END MATCH\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"count=\" & toString(count))\n\
END SUB\n";

const B622_RECORD: &str = "IMPORT io\nIMPORT thread\n\
TYPE TwEl\n  tag AS String\n  kids AS List OF TwNode\nEND TYPE\n\
TYPE TwText\n  text AS String\nEND TYPE\n\
UNION TwNode\n  TwEl\n  TwText\nEND UNION\n\
TYPE TwResult\n  ok AS Boolean\n  document AS TwNode\n  title AS String\nEND TYPE\n\
ISOLATED FUNC work(w AS ThreadWorker OF String TO TwResult, seed AS String) AS TwResult\n\
  LET leaf AS TwNode = TwText[seed & \"!\"]\n\
  LET root AS TwNode = TwEl[\"div\", [leaf, leaf]]\n\
  RETURN TwResult[TRUE, root, seed & \"?\"]\n\
END FUNC\n\
SUB main()\n\
  MUT total AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET t AS Thread OF String TO TwResult = thread::start(work, \"abc\")\n\
    LET r AS TwResult = thread::waitFor(t)\n\
    total = total + len(r.title)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"total=\" & toString(total))\n\
END SUB\n";

/// The same call-site copy on the message plane: the parent's `thread::receive` of a
/// worker-sent `String`.
const B622_RECEIVE: &str = "IMPORT io\nIMPORT thread\n\
ISOLATED FUNC work(w AS ThreadWorker OF String TO Integer, seed AS String) AS Integer\n\
  thread::send(w, seed & \"!\")\n\
  RETURN len(seed)\n\
END FUNC\n\
SUB main()\n\
  MUT total AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET t AS Thread OF String TO Integer = thread::start(work, \"abc\")\n\
    LET m AS String = thread::receive(t)\n\
    total = total + len(m) + thread::waitFor(t)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"total=\" & toString(total))\n\
END SUB\n";

/// bug-622 Part B: once a thread has been waited for and its handle dropped, the parent arena
/// holds nothing for it — not the control block, the worker arena state or the queues.
#[cfg(unix)]
#[test]
fn a_waited_for_thread_leaves_nothing_in_the_parent_arena() {
    let grew = thread_loop_growth(
        "b622_integer_result",
        B622_INTEGER_RESULT,
        "total=150",
        "total=300",
    );
    assert_eq!(
        grew, 0,
        "main-arena live_bytes grew {grew} B between 50 and 100 waited-for threads — \
         thread::start's plumbing is never freed"
    );
}

/// bug-622 Part B: the same for a completed thread whose handle is dropped without `waitFor`.
#[cfg(unix)]
#[test]
fn a_completed_thread_dropped_without_waitfor_leaves_nothing_in_the_parent_arena() {
    let grew = thread_loop_growth(
        "b622_completed_drop",
        B622_COMPLETED_DROP,
        "done=50",
        "done=100",
    );
    assert_eq!(
        grew, 0,
        "main-arena live_bytes grew {grew} B between 50 and 100 completed, dropped threads"
    );
}

/// bug-622 Part A: the parent-arena copy of a thread result is owned by whatever receives it
/// — a binding, a statement temp, a trapped `Result` — and freed like any fresh value. Each
/// shape must grow exactly as much as the scalar-result loop, whose growth is the plumbing
/// alone (zero once Part B holds), so this gate does not depend on Part B.
#[cfg(unix)]
#[test]
fn a_thread_result_copy_is_freed_by_its_owner() {
    let baseline = thread_loop_growth(
        "b622_baseline",
        B622_INTEGER_RESULT,
        "total=150",
        "total=300",
    );
    for (name, source, small_out, large_out) in [
        (
            "b622_string_bound",
            B622_STRING_BOUND,
            "total=350",
            "total=700",
        ),
        (
            "b622_string_unbound",
            B622_STRING_UNBOUND,
            "total=350",
            "total=700",
        ),
        (
            "b622_string_trapped",
            B622_STRING_TRAPPED,
            "total=350",
            "total=700",
        ),
        ("b622_union", B622_UNION, "count=100", "count=200"),
        ("b622_record", B622_RECORD, "total=200", "total=400"),
        ("b622_receive", B622_RECEIVE, "total=350", "total=700"),
    ] {
        let grew = thread_loop_growth(name, source, small_out, large_out);
        assert_eq!(
            grew,
            baseline,
            "{name}: main-arena live_bytes grew {grew} B between 50 and 100 threads, \
             {} B more than the scalar-result loop — the result copy is never freed",
            grew.saturating_sub(baseline)
        );
    }
}
