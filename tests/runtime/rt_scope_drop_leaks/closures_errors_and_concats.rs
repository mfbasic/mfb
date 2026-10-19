//! plan-176-C: `rt_scope_drop_leaks`' closure-argument (bug-572), trapped-error
//! (bug-565), param-borrow (bug-568) and returned-concat (bug-567) tests.
//! `B573_BUILTIN_RAISE` and the shared helpers live in the root, `rt_scope_drop_leaks.rs`.

use super::*;

// -------------------------------------------------------------- bug-572

/// bug-572, the bug report's own reproduction: a capturing `LAMBDA` passed to a
/// HOF leaked its whole environment on every call — the env block, the deep copy
/// of each captured value, and the 16-byte closure object. It is independent of
/// the callback's RESULT type, which is why the predicate is `Boolean`: that
/// allocates no result block at all, so nothing here is bug-562/569's leak.
/// 13 MB at 50 000 calls, 25 MB at 100 000.
const SHAPE_572_CAPTURING_FILTER: &str = "IMPORT io\n\
IMPORT collections\n\
FUNC main() AS Nothing\n\
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
END FUNC\n";

/// The contrast, one token wide: dropping the capture makes the lambda lower to
/// a `FunctionRef` over a static BSS descriptor rather than a `Closure`, so it
/// allocates nothing. 1.0 MB flat at both counts before AND after — a fix that
/// made the case above pass by suppressing an allocation would show up here.
const SHAPE_572_CONTRAST_CAPTURELESS: &str = "IMPORT io\n\
IMPORT collections\n\
FUNC main() AS Nothing\n\
  LET xs AS List OF String = [\"n0\", \"n1\", \"n2\", \"n3\", \"n4\", \"n5\", \"n6\", \"n7\"]\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    LET c AS List OF String = collections::filter(xs, LAMBDA(s AS String) -> len(s) < 8)\n\
    acc = acc + len(collections::get(c, 0))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END FUNC\n";

/// `collections::reduce`'s BINARY callback (`FUNC(U, T) AS U`, parameter index 2)
/// — a different position on the allow-list, and the one `callback_member`'s
/// unary rule deliberately excludes. 13 -> 25 MB before.
const SHAPE_572_REDUCE: &str = "IMPORT io\n\
IMPORT collections\n\
FUNC main() AS Nothing\n\
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
END FUNC\n";

/// `collections::forEach`, the one position that admits a BY-REF capture of a
/// `MUT` binding (`is_nonescaping_callback_arg`). A by-ref env slot holds a
/// pointer to the parent's stack slot, not an owned block, and the environment's
/// routine (bug-781) copies such a word verbatim and never frees it — only the
/// env array and the object are reclaimed. 7 -> 13 MB before; a wild free of
/// the by-ref slot would corrupt the caller's frame instead.
const SHAPE_572_FOREACH_BY_REF: &str = "IMPORT io\n\
IMPORT collections\n\
FUNC main() AS Nothing\n\
  LET xs AS List OF String = [\"n0\", \"n1\", \"n2\", \"n3\", \"n4\", \"n5\", \"n6\", \"n7\"]\n\
  MUT total AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    collections::forEach(xs, LAMBDA(s AS String) -> total = total + len(s))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"total=\" & toString(total))\n\
END FUNC\n";

/// `collections::mapValues`, whose callback is invoked from its own `.mfb` body
/// rather than a native loop — the callee only borrows it, and the caller's
/// statement drops it. 15 -> 29 MB before.
const SHAPE_572_MAP_VALUES: &str = "IMPORT io\n\
IMPORT collections\n\
FUNC main() AS Nothing\n\
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
END FUNC\n";

/// `http::route`'s handler is a non-isolated `FUNC` parameter that the returned
/// `http::Route` KEEPS: the server invokes it per request. Since bug-781 the
/// record keeps its own copy, so every route this loop builds and drops — and
/// the caller's handler temp — is freed (`a_retained_callback_position_is_freed_by_its_owners`).
const SHAPE_572_RETAINED_ROUTE: &str = "IMPORT io\n\
IMPORT http\n\
FUNC main() AS Nothing\n\
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
END FUNC\n";

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

/// `http::route` KEEPS its handler — and since bug-781 the `http::Route` keeps
/// its OWN copy (single-owner closures, `mfb spec memory closures`), so the
/// caller's temp, the reassigned-away `last`, and each dropped `r` are all freed.
/// Before bug-781 the record shared the caller's pointer, so nothing could free
/// it and the loop grew (51 -> 101 MB at 200k/400k on macOS aarch64, 13 -> 26 MB
/// on Linux); that growth was the retired arena-lifetime closure rule, not a
/// safety margin. The values a retained closure computes stay pinned by
/// `every_closure_argument_shape_still_produces_the_right_value`, and the copy
/// the route makes by `codegen_closure_temp_drop`.
#[cfg(unix)]
#[test]
fn a_retained_callback_position_is_freed_by_its_owners() {
    assert_flat(
        "b572_retained_route",
        SHAPE_572_RETAINED_ROUTE,
        200_000,
        400_000,
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
FUNC main() AS Nothing\n\
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
END FUNC\n";

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
FUNC main() AS Nothing\n\
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
END FUNC\n";

/// The shape bug-571 measured and could not fix: a failing call inside a walk
/// over a `List OF String`. bug-571 took it from 6 -> 11 MB down to the 2 -> 3 MB
/// the loop-free control costs and recorded the remainder as this bug. With eight
/// inner iterations per pass it is the same defect at eight times the rate:
/// **298.6 MB at 50 000 passes, 596.2 MB at 100 000**.
const B565_FAILING_CALL_IN_A_WALK: &str = "IMPORT io\n\
IMPORT collections\n\
FUNC boom(s AS String) AS Integer\n  FAIL error(7, \"boom\")\nEND FUNC\n\
FUNC main() AS Nothing\n\
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
END FUNC\n";

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
FUNC main() AS Nothing\n\
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
END FUNC\n";

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
FUNC main() AS Nothing\n\
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
END FUNC\n";

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
FUNC main() AS Nothing\n\
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
END FUNC\n";

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
FUNC main() AS Nothing\n\
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
END FUNC\n";

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
FUNC main() AS Nothing\n\
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
END FUNC\n";

/// The same defect with a `String` payload, where the abandoned wrapper inlines
/// the whole string rather than an 8-byte scalar — 195 B per call: **38.2 MB at
/// 200 000 and 75.4 MB at 400 000 before; 1.0 and 1.0 MB after.**
const B568_TRAP_STRING_BORROW: &str = "IMPORT io\n\
FUNC pick(s AS String, i AS Integer) AS String\n\
  IF i < 0 THEN\n    FAIL error(1, \"neg\")\n  END IF\n\
  RETURN s\n\
END FUNC\n\
FUNC main() AS Nothing\n\
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
END FUNC\n";

/// The POSITIVE pin: the identical param-borrow callee called WITHOUT a `TRAP`.
///
/// There the lowered value really IS the caller's own argument block, the copy is
/// what makes the binding an owner, and removing it would `arena_free` a live
/// local at scope drop. Flat before and after — and the source `base` is read
/// back on every iteration, so a wrong free shows up as a wrong total rather than
/// only as a fault.
const B568_CONTRAST_NO_TRAP: &str = "IMPORT io\n\
FUNC pick(s AS String) AS String\n  RETURN s\nEND FUNC\n\
FUNC main() AS Nothing\n\
  LET base AS String = \"abcdefghijklmnop\"\n\
  MUT total AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET n AS String = pick(base)\n\
    total = total + len(n) + len(base)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"total=\" & toString(total))\n\
END FUNC\n";

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
FUNC main() AS Nothing\n\
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
END FUNC\n";

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
FUNC main() AS Nothing\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    acc = acc + len(wrap(\"ab\"))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END FUNC\n";

/// The same defect where the interior temp is a CALL result rather than a nested
/// concat — the bug report's own second row, and the commonest real body shape.
/// 13.3 MB at 200k, 25.6 MB at 400k.
const SHAPE_567_CONCAT_OF_A_CALL: &str = "IMPORT io\n\
FUNC f(n AS Integer) AS String\n  RETURN \"v\" & toString(n MOD 10)\nEND FUNC\n\
FUNC main() AS Nothing\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    acc = acc + len(f(i))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END FUNC\n";

/// The bug report's CONTROL, and the row that makes the other two attributable:
/// one operator fewer, so both operands are already-owned values and the concat
/// registers exactly one temp — the one that leaves. Flat at 1.0 MB before the
/// fix and after. A POSITIVE pin: the fix ADDS frees, so a shape with nothing
/// interior must gain none.
const SHAPE_567_CONTRAST_TWO_OPERANDS: &str = "IMPORT io\n\
FUNC tail(s AS String) AS String\n  RETURN s & \">\"\nEND FUNC\n\
FUNC main() AS Nothing\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    acc = acc + len(tail(\"ab\"))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END FUNC\n";

/// `RETURN <collection literal built from interior concats>` — the interior temps
/// are the two element `String`s, and the value that leaves is the `List` block.
/// A different escaping type on the same `RETURN` path.
const SHAPE_567_RETURNED_LIST_OF_CONCATS: &str = "IMPORT io\n\
IMPORT strings\n\
FUNC pair(n AS Integer) AS List OF String\n\
  RETURN [\"a\" & toString(n MOD 10), \"b\" & toString(n MOD 10)]\n\
END FUNC\n\
FUNC main() AS Nothing\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    acc = acc + len(strings::join(pair(i), \",\"))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END FUNC\n";

/// The cleanup-bearing `RETURN` path: the function owns a local, so
/// `active_cleanups` is NOT empty and the return goes through
/// `store_pending_success_result` + `emit_cleanup_sequence` instead of the
/// register fast path. Same interior temp, different placement for its free.
const SHAPE_567_INTERIOR_TEMP_WITH_A_LIVE_LOCAL: &str = "IMPORT io\n\
FUNC decorate(n AS Integer) AS String\n\
  LET tag AS String = \"t\" & toString(n MOD 7)\n\
  RETURN tag & (\"-\" & toString(n MOD 10))\n\
END FUNC\n\
FUNC main() AS Nothing\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    acc = acc + len(decorate(i))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END FUNC\n";

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
FUNC main() AS Nothing\n\
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
END FUNC\n";

/// The same program with the `& "y"` removed — one operator fewer, so the 4 KB
/// block IS the message and nothing is interior. The control that makes the
/// equality above attributable.
const SHAPE_567_FAIL_CONTRAST_NO_INTERIOR: &str = "IMPORT io\n\
IMPORT strings\n\
FUNC boom(n AS Integer) AS String\n\
  FAIL error(7, strings::repeat(\"x\", 4000))\n\
END FUNC\n\
FUNC main() AS Nothing\n\
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
END FUNC\n";

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
FUNC main() AS Nothing\n\
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
END FUNC\n";

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
