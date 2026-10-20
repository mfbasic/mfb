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

#[path = "rt_scope_drop_leaks/closures_errors_and_concats.rs"]
mod closures_errors_and_concats;
#[path = "rt_scope_drop_leaks/helper_results_and_raised_errors.rs"]
mod helper_results_and_raised_errors;
#[path = "rt_scope_drop_leaks/loops_and_condition_temps.rs"]
mod loops_and_condition_temps;
#[path = "rt_scope_drop_leaks/named_loop_exits.rs"]
mod named_loop_exits;
#[path = "rt_scope_drop_leaks/return_and_string_shapes.rs"]
mod return_and_string_shapes;
#[path = "rt_scope_drop_leaks/trap_binds_hofs_and_for_each.rs"]
mod trap_binds_hofs_and_for_each;
#[path = "rt_scope_drop_leaks/unbound_results_and_gets.rs"]
mod unbound_results_and_gets;

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
FUNC main() AS Nothing\n\
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
END FUNC\n";
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
FUNC main() AS Nothing\n\
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
END FUNC\n";
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
FUNC main() AS Nothing\n\
  MUT total AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {count}\n\
    total = total + {call}\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"total=\" & toString(total))\n\
END FUNC\n"
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
FUNC main() AS Nothing\n\
  MUT total AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    RES t AS Thread OF String TO Integer = thread::start(work, \"abc\")\n\
    total = total + thread::waitFor(t)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"total=\" & toString(total))\n\
END FUNC\n";

/// A handle whose worker has finished, dropped at scope exit with no `waitFor`.
const B622_COMPLETED_DROP: &str = "IMPORT io\nIMPORT os\nIMPORT thread\n\
ISOLATED FUNC work(w AS ThreadWorker OF String TO Integer, seed AS String) AS Integer\n  RETURN len(seed)\nEND FUNC\n\
FUNC main() AS Nothing\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    RES t AS Thread OF String TO Integer = thread::start(work, \"abc\")\n\
    WHILE thread::isRunning(t)\n      os::sleep(1)\n    END WHILE\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"done=\" & toString(i))\n\
END FUNC\n";

const B622_STRING_BOUND: &str = "IMPORT io\nIMPORT thread\n\
ISOLATED FUNC work(w AS ThreadWorker OF String TO String, seed AS String) AS String\n  RETURN seed & \"-\" & seed\nEND FUNC\n\
FUNC main() AS Nothing\n\
  MUT total AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    RES t AS Thread OF String TO String = thread::start(work, \"abc\")\n\
    LET r AS String = thread::waitFor(t)\n\
    total = total + len(r)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"total=\" & toString(total))\n\
END FUNC\n";

const B622_STRING_UNBOUND: &str = "IMPORT io\nIMPORT thread\n\
ISOLATED FUNC work(w AS ThreadWorker OF String TO String, seed AS String) AS String\n  RETURN seed & \"-\" & seed\nEND FUNC\n\
FUNC main() AS Nothing\n\
  MUT total AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    RES t AS Thread OF String TO String = thread::start(work, \"abc\")\n\
    total = total + len(thread::waitFor(t))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"total=\" & toString(total))\n\
END FUNC\n";

const B622_STRING_TRAPPED: &str = "IMPORT io\nIMPORT thread\n\
ISOLATED FUNC work(w AS ThreadWorker OF String TO String, seed AS String) AS String\n  RETURN seed & \"-\" & seed\nEND FUNC\n\
FUNC main() AS Nothing\n\
  MUT total AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    RES t AS Thread OF String TO String = thread::start(work, \"abc\")\n\
    LET r AS String = thread::waitFor(t) TRAP(e)\n      RECOVER \"\"\n    END TRAP\n\
    total = total + len(r)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"total=\" & toString(total))\n\
END FUNC\n";

const B622_UNION: &str = "IMPORT io\nIMPORT thread\n\
TYPE TwEl\n  tag AS String\n  kids AS List OF TwNode\nEND TYPE\n\
TYPE TwText\n  text AS String\nEND TYPE\n\
UNION TwNode\n  TwEl\n  TwText\nEND UNION\n\
ISOLATED FUNC work(w AS ThreadWorker OF String TO TwNode, seed AS String) AS TwNode\n\
  LET leaf AS TwNode = TwText[seed & \"!\"]\n\
  LET root AS TwNode = TwEl[\"div\", [leaf, leaf]]\n\
  RETURN root\n\
END FUNC\n\
FUNC main() AS Nothing\n\
  MUT count AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    RES t AS Thread OF String TO TwNode = thread::start(work, \"abc\")\n\
    LET r AS TwNode = thread::waitFor(t)\n\
    MATCH r\n      CASE TwEl(e)\n        count = count + len(e.kids)\n      CASE ELSE\n    END MATCH\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"count=\" & toString(count))\n\
END FUNC\n";

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
FUNC main() AS Nothing\n\
  MUT total AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    RES t AS Thread OF String TO TwResult = thread::start(work, \"abc\")\n\
    LET r AS TwResult = thread::waitFor(t)\n\
    total = total + len(r.title)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"total=\" & toString(total))\n\
END FUNC\n";

/// The same call-site copy on the message plane: the parent's `thread::receive` of a
/// worker-sent `String`.
const B622_RECEIVE: &str = "IMPORT io\nIMPORT thread\n\
ISOLATED FUNC work(w AS ThreadWorker OF String TO Integer, seed AS String) AS Integer\n\
  thread::send(w, seed & \"!\")\n\
  RETURN len(seed)\n\
END FUNC\n\
FUNC main() AS Nothing\n\
  MUT total AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    RES t AS Thread OF String TO Integer = thread::start(work, \"abc\")\n\
    LET m AS String = thread::receive(t)\n\
    total = total + len(m) + thread::waitFor(t)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"total=\" & toString(total))\n\
END FUNC\n";

/// A trapped thread query that does not read a value across the boundary: the inline `TRAP`
/// builds its `Result` in this frame, and that wrapper leaked 144 B per trapped
/// `thread::isRunning` while every `thread.*` `CallResult` counted as runtime-managed.
const B622_TRAPPED_QUERY: &str = "IMPORT io\nIMPORT thread\n\
ISOLATED FUNC work(w AS ThreadWorker OF String TO Integer, seed AS String) AS Integer\n  RETURN len(seed)\nEND FUNC\n\
FUNC main() AS Nothing\n\
  MUT total AS Integer = 0\n\
  MUT done AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    RES t AS Thread OF String TO Integer = thread::start(work, \"abc\")\n\
    total = total + thread::waitFor(t)\n\
    LET running AS Boolean = thread::isRunning(t) TRAP(e)\n      RECOVER TRUE\n    END TRAP\n\
    IF NOT running THEN done = done + 1\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"total=\" & toString(total) & \" done=\" & toString(done))\n\
END FUNC\n";

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

/// A handle lent to a function that waits for it (plan-156-C: passing a thread lends it): the
/// callee's parameter drops nothing, so the plumbing is freed once, by the caller's drop.
const B622_MOVED_TO_CALLEE: &str = "IMPORT io\nIMPORT thread\n\
ISOLATED FUNC work(w AS ThreadWorker OF String TO Integer, seed AS String) AS Integer\n  RETURN len(seed)\nEND FUNC\n\
FUNC finish(t AS Thread OF String TO Integer) AS Integer\n  RETURN thread::waitFor(t)\nEND FUNC\n\
FUNC main() AS Nothing\n\
  MUT total AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    RES t AS Thread OF String TO Integer = thread::start(work, \"abc\")\n\
    total = total + finish(t)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"total=\" & toString(total))\n\
END FUNC\n";

/// A `RES Thread` bound afresh every pass, plus one outside the loop: each pass's handle
/// is dropped at the end of its pass. (plan-156-F: a `RES` binding is not reassigned, so
/// this is what the former `MUT` reassignment is spelled as now.)
const B622_REASSIGNED: &str = "IMPORT io\nIMPORT thread\n\
ISOLATED FUNC work(w AS ThreadWorker OF String TO Integer, seed AS String) AS Integer\n  RETURN len(seed)\nEND FUNC\n\
FUNC main() AS Nothing\n\
  MUT total AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  RES first AS Thread OF String TO Integer = thread::start(work, \"abc\")\n\
  total = total + thread::waitFor(first)\n\
  WHILE i < {N}\n\
    RES t AS Thread OF String TO Integer = thread::start(work, \"abc\")\n\
    total = total + thread::waitFor(t)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"total=\" & toString(total))\n\
END FUNC\n";

/// bug-622 Part B positive pins: the free happens exactly once whichever binding holds the
/// handle last — a caller that lent it to a callee, or a binding per pass of a loop.
/// A second drop of a freed block would show as a skipped double free or a wrong value.
#[cfg(unix)]
#[test]
fn a_moved_or_reassigned_thread_handle_is_freed_exactly_once() {
    let moved = thread_loop_growth(
        "b622_moved_to_callee",
        B622_MOVED_TO_CALLEE,
        "total=150",
        "total=300",
    );
    assert_eq!(
        moved, 0,
        "main-arena live_bytes grew {moved} B between 50 and 100 handles moved into a callee"
    );
    let reassigned =
        thread_loop_growth("b622_reassigned", B622_REASSIGNED, "total=153", "total=303");
    assert_eq!(
        reassigned, 0,
        "main-arena live_bytes grew {reassigned} B between 50 and 100 reassigned handles"
    );
}

/// One handle under two names: `RES t2 = t` moves it to `t2` (plan-156-F: a `RES` binding
/// from another is a move), which retrieves the result and asks again for the same result
/// (`waitFor` is repeatable, plan-156-C). Freed once, by `t2`'s drop.
const B622_ALIAS: &str = "IMPORT io\nIMPORT thread\n\
ISOLATED FUNC work(w AS ThreadWorker OF String TO Integer, seed AS String) AS Integer\n  RETURN len(seed)\nEND FUNC\n\
FUNC main() AS Nothing\n\
  MUT total AS Integer = 0\n\
  MUT repeated AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    RES t AS Thread OF String TO Integer = thread::start(work, \"abc\")\n\
    RES t2 AS Thread OF String TO Integer = t\n\
    total = total + thread::waitFor(t2)\n\
    LET again AS Integer = thread::waitFor(t2) TRAP(e)\n      RECOVER e.code\n    END TRAP\n\
    IF again = 3 THEN repeated = repeated + 1\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"total=\" & toString(total) & \" repeated=\" & toString(repeated))\n\
END FUNC\n";

/// A handle lent to a function that waits for it, then read again by the caller: the caller
/// still holds an open handle, and the thread is done.
const B622_MOVED_REUSE: &str = "IMPORT io\nIMPORT thread\n\
ISOLATED FUNC work(w AS ThreadWorker OF String TO Integer, seed AS String) AS Integer\n  RETURN len(seed)\nEND FUNC\n\
FUNC finish(t AS Thread OF String TO Integer) AS Integer\n  RETURN thread::waitFor(t)\nEND FUNC\n\
FUNC main() AS Nothing\n\
  MUT total AS Integer = 0\n\
  MUT done AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    RES t AS Thread OF String TO Integer = thread::start(work, \"abc\")\n\
    total = total + finish(t)\n\
    IF NOT thread::isRunning(t) THEN done = done + 1\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"total=\" & toString(total) & \" done=\" & toString(done))\n\
END FUNC\n";

/// An inline `TRAP` on `thread::start`: the handle is bound once as the trap's value and
/// again as `t`.
const B622_INLINE_TRAP_START: &str = "IMPORT io\nIMPORT thread\n\
ISOLATED FUNC work(w AS ThreadWorker OF String TO Integer, seed AS String) AS Integer\n  RETURN len(seed)\nEND FUNC\n\
FUNC once() AS Integer\n\
  RES t AS Thread OF String TO Integer = thread::start(work, \"abc\") TRAP(e)\n    RETURN -100\n  END TRAP\n\
  RETURN thread::waitFor(t)\n\
END FUNC\n\
FUNC main() AS Nothing\n\
  MUT total AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    total = total + once()\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"total=\" & toString(total))\n\
END FUNC\n";

/// A completed worker's handle, dropped by the trap route of a `FAIL`. The handler cannot
/// name it (`TYPE_USE_AFTER_MOVE`: a resource the route closed), so it answers -1 itself.
const B622_TRAP_ROUTED: &str = "IMPORT io\nIMPORT os\nIMPORT thread\n\
ISOLATED FUNC work(w AS ThreadWorker OF String TO Integer, seed AS String) AS Integer\n  RETURN len(seed)\nEND FUNC\n\
FUNC routed() AS Integer\n\
  RES t AS Thread OF String TO Integer = thread::start(work, \"abc\")\n\
  WHILE thread::isRunning(t)\n    os::sleep(1)\n  END WHILE\n\
  FAIL error(77050002, \"forced\")\n\
  RETURN 0\n\
  TRAP(err)\n\
    RETURN -1\n\
  END TRAP\n\
END FUNC\n\
FUNC main() AS Nothing\n\
  MUT total AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    total = total + routed()\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"total=\" & toString(total))\n\
END FUNC\n";

/// bug-622: a handle more than one binding can reach — an alias, a caller that lent it, the
/// value binding of an inline `TRAP`, a function whose `TRAP` route dropped it — stays
/// readable while it is open (no crash, no skipped double free) and is freed exactly once.
/// Freeing at the first drop crashed all four shapes (exit 139).
#[cfg(unix)]
#[test]
fn a_shared_thread_handle_stays_readable_and_is_freed_once() {
    for (name, source, small_out, large_out) in [
        (
            "b622_alias",
            B622_ALIAS,
            "total=150 repeated=50",
            "total=300 repeated=100",
        ),
        (
            "b622_moved_reuse",
            B622_MOVED_REUSE,
            "total=150 done=50",
            "total=300 done=100",
        ),
        (
            "b622_inline_trap_start",
            B622_INLINE_TRAP_START,
            "total=150",
            "total=300",
        ),
        (
            "b622_trap_routed",
            B622_TRAP_ROUTED,
            "total=-50",
            "total=-100",
        ),
    ] {
        let grew = thread_loop_growth(name, source, small_out, large_out);
        assert_eq!(
            grew, 0,
            "{name}: main-arena live_bytes grew {grew} B between 50 and 100 shared handles"
        );
    }
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
        (
            "b622_trapped_query",
            B622_TRAPPED_QUERY,
            "total=150 done=50",
            "total=300 done=100",
        ),
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
