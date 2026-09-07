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
