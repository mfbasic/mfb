//! plan-176-C: `rt_scope_drop_leaks`' trap-bound result tests (bug-561), HOF callback
//! tests (bug-569) and `FOR EACH` item tests (bug-571). `B561_TRAP_ALWAYS_FAILS` and
//! the shared helpers live in the root, `rt_scope_drop_leaks.rs`.

use super::*;

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
FUNC main() AS Nothing\n\
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
END FUNC\n";

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
FUNC main() AS Nothing\n\
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
END FUNC\n";

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
FUNC main() AS Nothing\n\
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
END FUNC\n";

/// The bug report's own claim, kept as a POSITIVE pin: it recorded
/// `Result OF Integer` as the WORST case at 128 B per call. It never leaked —
/// a scalar payload lives in the `Result`'s own block, which was always freed —
/// and it must stay flat, because the fix adds a free next to it.
const B561_CONTRAST_SCALAR: &str = "IMPORT io\n\
FUNC half(n AS Integer) AS Integer\n\
  IF n < 0 THEN\n    FAIL error(7, \"negative\")\n  END IF\n\
  RETURN n / 2\n\
END FUNC\n\
FUNC main() AS Nothing\n\
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
END FUNC\n";

/// The other POSITIVE pin: the same producers with NO `TRAP` were already flat
/// (their block is bound and freed by the binding's own scope drop). The fix
/// must not give them a second free.
const B561_CONTRAST_NO_TRAP: &str = "IMPORT io\n\
IMPORT collections\n\
IMPORT strings\n\
FUNC fname(n AS Integer) AS String\n  RETURN toString(n MOD 10)\nEND FUNC\n\
FUNC trio(n AS Integer) AS List OF Integer\n  RETURN [n, n + 1, n + 2]\nEND FUNC\n\
FUNC main() AS Nothing\n\
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
END FUNC\n";

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
FUNC main() AS Nothing\n\
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
END FUNC\n";

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
FUNC main() AS Nothing\n\
  LET xs AS List OF String = [\"n0\", \"n1\", \"n2\", \"n3\", \"n4\", \"n5\", \"n6\", \"n7\"]\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    LET c AS List OF String = collections::transform(xs, pick)\n\
    acc = acc + len(collections::get(c, 0))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END FUNC\n";

/// `sortBy`'s key projection. String items + a String key declines the native
/// fast path, so this is the `.mfb` body's route to the callback.
const SHAPE_569_SORT_BY: &str = "IMPORT io\n\
IMPORT collections\n\
FUNC key(s AS String) AS String\n  RETURN toString(s)\nEND FUNC\n\
FUNC main() AS Nothing\n\
  LET xs AS List OF String = [\"n3\", \"n1\", \"n7\", \"n0\", \"n5\", \"n2\", \"n6\", \"n4\"]\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    LET c AS List OF String = collections::sortBy(xs, key)\n\
    acc = acc + len(collections::get(c, 0))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END FUNC\n";

/// `groupBy`'s value projection, on the native fast path (Integer key).
const SHAPE_569_GROUP_BY: &str = "IMPORT io\n\
IMPORT collections\n\
FUNC kf(s AS String) AS Integer\n  RETURN len(s)\nEND FUNC\n\
FUNC vf(s AS String) AS String\n  RETURN toString(s)\nEND FUNC\n\
FUNC main() AS Nothing\n\
  LET xs AS List OF String = [\"n0\", \"n1\", \"n2\", \"n3\", \"n4\", \"n5\", \"n6\", \"n7\"]\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    LET c AS Map OF Integer TO List OF String = collections::groupBy(xs, kf, vf)\n\
    acc = acc + len(collections::get(c, 2))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END FUNC\n";

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
FUNC main() AS Nothing\n\
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
END FUNC\n";

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
FUNC main() AS Nothing\n\
  LET xs AS List OF String = [\"n0\", \"n1\", \"n2\", \"n3\", \"n4\", \"n5\", \"n6\", \"n7\"]\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    LET c AS List OF String = collections::transform(xs, pick)\n\
    acc = acc + len(collections::get(c, 0)) + len(collections::get(c, 7))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc) & \" src=\" & collections::get(xs, 0))\n\
END FUNC\n";

/// A fixed-width callback over the same list. It never allocated and was always
/// flat, and it must stay flat: a fix that freed something here would be freeing
/// a scalar.
const SHAPE_569_FIXED_WIDTH_CONTRAST: &str = "IMPORT io\n\
IMPORT collections\n\
FUNC pick(s AS String) AS Integer\n  RETURN len(s)\nEND FUNC\n\
FUNC main() AS Nothing\n\
  LET xs AS List OF String = [\"n0\", \"n1\", \"n2\", \"n3\", \"n4\", \"n5\", \"n6\", \"n7\"]\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    LET c AS List OF Integer = collections::transform(xs, pick)\n\
    acc = acc + collections::get(c, 0)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END FUNC\n";

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
FUNC main() AS Nothing\n\
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
      RETURN\n\
    END IF\n\
    IF n <> \"held-\" & toString(rep) THEN\n\
      io::print(\"INDIRECT WRONG at rep=\" & toString(rep) & \" n=\" & n)\n\
      RETURN\n\
    END IF\n\
    IF collections::get(xs, 0) <> \"ab\" OR collections::get(xs, 7) <> \"op\" THEN\n\
      io::print(\"SOURCE CLOBBERED at rep=\" & toString(rep))\n\
      RETURN\n\
    END IF\n\
    rep = rep + 1\n\
  END WHILE\n\
  io::print(sig)\n\
  io::print(\"ok\")\n\
END FUNC\n";

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
FUNC main() AS Nothing\n\
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
END FUNC\n";

/// A `Map OF String TO String` materialises BOTH sides per entry — the key and
/// the value are separate loads — so it leaked twice as fast: 50 MB at 50 000,
/// 99 MB at 100 000.
const SHAPE_571_MAP_OF_STRING: &str = "IMPORT io\n\
IMPORT collections\n\
FUNC main() AS Nothing\n\
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
END FUNC\n";

/// A `Set OF String` reaches the same materialising arm on its own code path in
/// `lower_for_each`; it leaked identically (25 -> 50 MB) and is the arm a
/// `List`/`Map` enumeration silently omits.
const SHAPE_571_SET_OF_STRING: &str = "IMPORT io\n\
IMPORT collections\n\
FUNC main() AS Nothing\n\
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
END FUNC\n";

/// The bug report's contrast, and the case that says it is the `String` element
/// and not the loop: `FOR EACH` over a `Map OF Integer TO Integer` reading
/// `e.key`/`e.value` was 1.0 MB flat at both counts before the fix, and must stay
/// flat after it — its payload arms materialise nothing, so a fix that freed
/// anything here would be freeing a scalar.
const SHAPE_571_CONTRAST_MAP_OF_INTEGER: &str = "IMPORT io\n\
IMPORT collections\n\
FUNC main() AS Nothing\n\
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
END FUNC\n";

/// The `List` half of the same contrast.
const SHAPE_571_CONTRAST_LIST_OF_INTEGER: &str = "IMPORT io\n\
FUNC main() AS Nothing\n\
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
END FUNC\n";

/// `EXIT FOR` leaves the loop from the MIDDLE of an iteration, branching straight
/// to the end label past the fall-through drop. 16 -> 31 MB before.
const SHAPE_571_EXIT_FOR: &str = "IMPORT io\n\
FUNC main() AS Nothing\n\
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
END FUNC\n";

/// `CONTINUE FOR` branches to the TOP of the loop, also past the fall-through
/// drop. 25 -> 50 MB before.
const SHAPE_571_CONTINUE_FOR: &str = "IMPORT io\n\
FUNC main() AS Nothing\n\
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
END FUNC\n";

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
FUNC main() AS Nothing\n\
  LET xs AS List OF String = [\"n0\", \"n1\", \"n2\", \"n3\", \"n4\", \"n5\", \"n6\", \"n7\"]\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    LET got AS String = pick(xs)\n\
    acc = acc + len(got)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc) & \" src=\" & pick(xs))\n\
END FUNC\n";

/// A body that USES the element beyond `len` — appends it to a collection and
/// concatenates it into a string. Both are owning consumers that COPY (§14.6:
/// "inserting into a container copies or moves the inserted value into the
/// container; it never stores a non-owning alias"), so the block stays the
/// loop's to drop — but that has to be measured, not assumed. 27 -> 54 MB before;
/// the printed `src=` proves the source list is intact afterward.
const SHAPE_571_USES_THE_ITEM: &str = "IMPORT io\n\
IMPORT collections\n\
FUNC main() AS Nothing\n\
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
END FUNC\n";

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
FUNC main() AS Nothing\n\
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
END FUNC\n";

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
