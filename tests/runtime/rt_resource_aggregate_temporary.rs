//! bug-999: an aggregate of resources that nothing but its own statement can hold
//! is closed, and freed, with that statement.
//!
//! `len(mkList())`, with `mkList` returning a `List OF RES fs::File`, used to
//! leave every file the list held open for the life of the process: no binding
//! took the list, and bug-842's statement temporaries cover a call whose result
//! IS a resource, not one that holds some.
//!
//! One class of such calls is now a temporary its statement owns outright: a call
//! to a function body taking only plain scalars, written straight into an
//! argument of a call that keeps nothing of it and can only answer with a plain
//! scalar. Nothing can keep such a result, so the statement adopts what it reaches
//! exactly as the bound form `LET t = mkList()` does and drops it at its end.
//!
//! The positions where a value that outlives the statement can keep the aggregate
//! (a constructor argument, a member read, a `FOR EACH` iterable) are NOT in the
//! class. The last test here holds them to the one thing that must stay true of
//! them whatever is decided later: what they keep stays open and usable.
#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use common::debug_report::{arena_lines, build_debug, counter};
use std::path::Path;
use std::process::Command;

const PRELUDE: &str = r#"IMPORT collections
IMPORT fs
IMPORT io
IMPORT os

TYPE Holder
  h AS RES fs::File
  n AS Integer
END TYPE

TYPE Layer
  items AS List OF RES fs::File
END TYPE

FUNC count() AS Integer
  RETURN toInt(os::getEnv("N"))
END FUNC

FUNC mkList() AS List OF RES fs::File
  MUT xs AS List OF RES fs::File = []
  RES f AS fs::File = fs::createTempFile()
  xs = collections::append(xs, f)
  RES g AS fs::File = fs::createTempFile()
  xs = collections::append(xs, g)
  RETURN xs
END FUNC

FUNC mkListN(k AS Integer, tag AS String) AS List OF RES fs::File
  MUT xs AS List OF RES fs::File = []
  FOR i = 1 TO k
    RES f AS fs::File = fs::createTempFile()
    xs = collections::append(xs, f)
  NEXT
  RETURN xs
END FUNC

FUNC mkTwice() AS List OF RES fs::File
  MUT xs AS List OF RES fs::File = []
  RES f AS fs::File = fs::createTempFile()
  xs = collections::append(xs, f)
  xs = collections::append(xs, f)
  RETURN xs
END FUNC

FUNC mkMap() AS Map OF String TO RES fs::File
  MUT m AS Map OF String TO RES fs::File = Map OF String TO RES fs::File {}
  RES f AS fs::File = fs::createTempFile()
  m = collections::set(m, "a", f)
  RETURN m
END FUNC

FUNC mkHolders() AS List OF Holder
  MUT hs AS List OF Holder = []
  RES f AS fs::File = fs::createTempFile()
  hs = collections::append(hs, Holder[f, 1])
  RETURN hs
END FUNC

FUNC mkLayer() AS Layer
  RETURN Layer[items := mkList()]
END FUNC

FUNC written(xs AS List OF RES fs::File) AS Integer
  MUT t AS Integer = 0
  FOR EACH h IN xs
    fs::writeAll(h, "x")
    t = t + 1
  NEXT
  RETURN t
END FUNC

FUNC holders(hs AS List OF Holder) AS Integer
  RETURN len(hs)
END FUNC
"#;

/// `PRELUDE`, then `extra` declarations, then a `main` that runs `body` `N` times
/// with `n` to add to and `i` the iteration, and prints `n`.
fn program(extra: &str, body: &str) -> String {
    format!(
        "{PRELUDE}\n{extra}\nFUNC main() AS Integer\n  MUT n AS Integer = 0\n  \
         FOR i = 1 TO count()\n{body}\n  NEXT\n  io::print(\"n=\" & toString(n))\n  \
         RETURN 0\nEND FUNC\n"
    )
}

struct Counts {
    allocs: u64,
    frees: u64,
    live: u64,
    double_free_skips: u64,
    stdout: String,
}

fn run_at(name: &str, exe: &Path, n: u32) -> Counts {
    let (status, stdout, stderr) = common::run_capture_with_env(exe, &[("N", n.to_string())]);
    assert_eq!(status, 0, "{name} N={n} failed:\n{stdout}\n{stderr}");
    let lines = arena_lines(name, &stderr);
    Counts {
        allocs: counter(name, &lines, 0, "alloc_calls"),
        frees: counter(name, &lines, 0, "free_calls"),
        live: counter(name, &lines, 0, "live_bytes"),
        double_free_skips: counter(name, &lines, 0, "double_free_skips"),
        stdout,
    }
}

/// Run `exe` at `n` iterations with at most 64 open descriptors; a statement that
/// leaves one file open per iteration runs out within the first hundred.
fn run_limited(exe: &Path, n: u32) -> (bool, String, String) {
    let out = Command::new("sh")
        .arg("-c")
        .arg("ulimit -n 64 && exec \"$0\"")
        .arg(exe)
        .env("N", n.to_string())
        .output()
        .expect("run the program under sh");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// One statement of the class: at 10 and at 20 iterations the arena ends with as
/// many frees as allocations, nothing live and no double free skipped; 300
/// iterations fit in 64 descriptors; and `n` is `per_iteration` times the count.
fn assert_statement_owns_it(name: &str, extra: &str, body: &str, per_iteration: u32) {
    let exe = build_debug(name, &program(extra, body));
    for n in [10, 20] {
        let run = run_at(name, &exe, n);
        assert_eq!(
            run.double_free_skips, 0,
            "{name} N={n}: the arena skipped a double free"
        );
        assert_eq!(
            (run.allocs, run.live),
            (run.frees, 0),
            "{name} N={n}: alloc_calls {} free_calls {} live_bytes {}",
            run.allocs,
            run.frees,
            run.live
        );
        assert_eq!(
            run.stdout,
            format!("n={}\n", n * per_iteration),
            "{name} N={n}"
        );
    }
    let (ok, stdout, stderr) = run_limited(&exe, 300);
    assert!(
        ok && stdout == format!("n={}\n", 300 * per_iteration),
        "{name}: 300 iterations must fit in 64 descriptors; stdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

/// The report's own program: 2000 x `len(mkList())` under a 64-descriptor limit.
#[test]
fn the_reported_loop_runs_out_of_no_descriptors() {
    let source = "IMPORT fs\nIMPORT io\nIMPORT collections\n\n\
         FUNC mkList() AS List OF RES fs::File\n\
         \x20 MUT xs AS List OF RES fs::File = []\n\
         \x20 RES f AS fs::File = fs::createTempFile()\n\
         \x20 xs = collections::append(xs, f)\n\
         \x20 RETURN xs\n\
         END FUNC\n\n\
         FUNC main AS Integer\n\
         \x20 MUT n AS Integer = 0\n\
         \x20 FOR i = 1 TO 2000\n\
         \x20   n = n + len(mkList())\n\
         \x20 NEXT\n\
         \x20 io::print(\"n=\" & toString(n))\n\
         \x20 RETURN 0\n\
         END FUNC\n";
    let exe = build_debug("bug999_reported", source);
    let (ok, stdout, stderr) = run_limited(&exe, 0);
    assert!(
        ok && stdout == "n=2000\n",
        "every file `len(mkList())` opened must be closed with its statement (bug-999); \
         stdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

#[test]
fn an_aggregate_given_to_a_builtin_that_only_measures_it_is_dropped_with_the_statement() {
    assert_statement_owns_it("bug999_len", "", "    n = n + len(mkList())", 2);
    assert_statement_owns_it("bug999_len_map", "", "    n = n + len(mkMap())", 1);
    assert_statement_owns_it(
        "bug999_len_scalar_args",
        "",
        "    n = n + len(mkListN(3, \"t\"))",
        3,
    );
}

/// A record the list holds twice is closed and freed once.
#[test]
fn an_element_listed_twice_is_dropped_once() {
    assert_statement_owns_it("bug999_twice", "", "    n = n + len(mkTwice())", 2);
}

/// The receiving call is a function body that reads the elements and keeps none.
#[test]
fn an_aggregate_given_to_a_function_that_keeps_nothing_is_dropped_with_the_statement() {
    assert_statement_owns_it("bug999_user", "", "    n = n + written(mkList())", 2);
    assert_statement_owns_it("bug999_holders", "", "    n = n + holders(mkHolders())", 1);
}

/// The statement is not an assignment: a condition, a loop condition, a `RETURN`
/// (the early one and the last one), a nested argument, and more than one such
/// call in one expression.
#[test]
fn every_statement_kind_drops_its_own_temporaries() {
    assert_statement_owns_it(
        "bug999_if",
        "",
        "    IF len(mkList()) > 1 THEN\n      n = n + 1\n    END IF",
        1,
    );
    assert_statement_owns_it(
        "bug999_while",
        "",
        "    MUT j AS Integer = 0\n    WHILE j < len(mkList())\n      j = j + 1\n    END WHILE\n    n = n + j",
        2,
    );
    assert_statement_owns_it(
        "bug999_return",
        "FUNC early(i AS Integer) AS Integer\n  IF i MOD 2 = 0 THEN\n    RETURN len(mkList())\n  END IF\n  \
         RETURN written(mkList()) + len(mkList()) - 2\nEND FUNC\n",
        "    n = n + early(i)",
        2,
    );
    assert_statement_owns_it(
        "bug999_nested_argument",
        "",
        "    n = n + toInt(toString(len(mkList())))",
        2,
    );
    assert_statement_owns_it(
        "bug999_three",
        "",
        "    n = n + len(mkList()) + written(mkList()) + len(mkListN(1, \"a\"))",
        5,
    );
}

/// The receiving call fails after the producer ran. The error exit drops the
/// temporary too, whether the failure is trapped in the same function or
/// propagates out of it.
#[test]
fn a_failing_receiving_call_drops_the_temporary_on_the_error_exit() {
    assert_statement_owns_it(
        "bug999_error_exit",
        "FUNC refuse(xs AS List OF RES fs::File) AS Integer
  IF len(xs) > 0 THEN
             FAIL error(77, \"refused\")
  END IF
  RETURN 0
END FUNC

         FUNC trapped(i AS Integer) AS Integer
  RETURN refuse(mkList()) + 1
  TRAP(err)
             RETURN 5
  END TRAP
END FUNC

         FUNC propagates(i AS Integer) AS Integer
  RETURN refuse(mkList()) + len(mkList())
END FUNC

         FUNC catches(i AS Integer) AS Integer
  RETURN propagates(i)
  TRAP(err)
             RETURN 7
  END TRAP
END FUNC
",
        "    n = n + trapped(i) + catches(i)",
        12,
    );
}

/// A statement that skips the producer on some iterations (`CONTINUE`, a
/// short-circuited operand) drops nothing for the ones it skipped.
#[test]
fn a_skipped_producer_drops_nothing() {
    let name = "bug999_skipped";
    let body = "    IF i < 0 AND len(mkList()) > 0 THEN\n      n = n + 100\n    END IF\n    \
                IF i MOD 2 = 0 THEN\n      CONTINUE FOR\n    END IF\n    n = n + len(mkList())";
    let exe = build_debug(name, &program("", body));
    for n in [10, 20] {
        let run = run_at(name, &exe, n);
        assert_eq!(run.double_free_skips, 0, "{name} N={n}");
        assert_eq!(
            (run.allocs, run.live),
            (run.frees, 0),
            "{name} N={n}: alloc_calls {} free_calls {} live_bytes {}",
            run.allocs,
            run.frees,
            run.live
        );
        assert_eq!(run.stdout, format!("n={n}\n"), "{name} N={n}");
    }
}

/// The receiving function stores one element in a module-level list. That element
/// is the module instance's from then on: the statement closes the other file and
/// leaves this one open, and it is still writable after the loop.
#[test]
fn an_element_the_receiving_function_stores_at_module_level_stays_open() {
    let name = "bug999_stash";
    let source = format!(
        "{PRELUDE}\nMUT kept AS List OF RES fs::File = []\n\n\
         FUNC stash(xs AS List OF RES fs::File) AS Integer\n\
         \x20 kept = collections::append(kept, collections::get(xs, 0))\n\
         \x20 RETURN 1\n\
         END FUNC\n\n\
         FUNC main() AS Integer\n\
         \x20 MUT n AS Integer = 0\n\
         \x20 FOR i = 1 TO count()\n\
         \x20   n = n + stash(mkList())\n\
         \x20 NEXT\n\
         \x20 FOR EACH k IN kept\n\
         \x20   fs::writeAll(k, \"still open\")\n\
         \x20   n = n + 100\n\
         \x20 NEXT\n\
         \x20 io::print(\"n=\" & toString(n) & \" kept=\" & toString(len(kept)))\n\
         \x20 RETURN 0\n\
         END FUNC\n"
    );
    let exe = build_debug(name, &source);
    let run = run_at(name, &exe, 10);
    assert_eq!(
        run.double_free_skips, 0,
        "{name}: the arena skipped a double free"
    );
    assert_eq!(run.stdout, "n=1010 kept=10\n", "{name}");
    // 40 files stay open (the kept ones); the other 40 must have been closed.
    let (ok, stdout, stderr) = run_limited(&exe, 40);
    assert!(
        ok && stdout == "n=4040 kept=40\n",
        "{name}: the files nothing kept must be closed; stdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

/// The positions outside the class, where a value that outlives the statement
/// keeps the aggregate: whatever it keeps is still open and usable afterwards, and
/// nothing is freed twice. (An earlier attempt at bug-999 closed these at the end
/// of the statement and read freed memory.)
#[test]
fn an_aggregate_a_longer_lived_value_keeps_stays_usable() {
    let name = "bug999_kept";
    let body = "    LET back AS Layer = Layer[items := mkList()]\n    \
                LET items AS List OF RES fs::File = mkLayer().items\n    \
                n = n + written(back.items) + written(items)\n    \
                FOR EACH h IN mkList()\n      fs::writeAll(h, \"x\")\n      n = n + 1\n    NEXT";
    let exe = build_debug(name, &program("", body));
    let run = run_at(name, &exe, 5);
    assert_eq!(
        run.double_free_skips, 0,
        "{name}: the arena skipped a double free"
    );
    assert!(
        run.frees <= run.allocs,
        "{name}: free_calls {} exceeds alloc_calls {}",
        run.frees,
        run.allocs
    );
    assert_eq!(run.stdout, "n=30\n", "{name}");
}

/// The declarations the positions below add to `PRELUDE`: a resource union and a
/// list of it, a record with a `RES` field built by a call, a reader of one
/// plain field of such a record, and a receiver that fails.
const POSITIONS: &str = r#"IMPORT tcp

UNION Handle
  fs::File
  tcp::Socket
END UNION

FUNC mkHandles() AS List OF RES Handle
  MUT xs AS List OF RES Handle = []
  RES f AS Handle = fs::createTempFile()
  xs = collections::append(xs, f)
  RETURN xs
END FUNC

FUNC mkHolder() AS Holder
  RES f AS fs::File = fs::createTempFile()
  RETURN Holder[f, 1]
END FUNC

FUNC useH(h AS Holder) AS Integer
  RETURN h.n
END FUNC

FUNC countFail(xs AS List OF RES fs::File) AS Integer
  IF len(xs) > 0 THEN
    FAIL error(77, "boom")
  END IF
  RETURN 0
END FUNC
"#;

/// bug-999, position 1: a record takes the list a constructor argument built, and
/// releases it with its own scope.
#[test]
fn a_constructor_argument_is_released_with_the_record_that_took_it() {
    assert_statement_owns_it(
        "bug999_ctor",
        POSITIONS,
        "    LET back AS Layer = Layer[items := mkList()]\n    n = n + len(back.items)",
        2,
    );
}

/// bug-999, position 2: a member read out of a record a call returned. Bound, the
/// binding takes the list; unbound, the statement does.
#[test]
fn a_member_read_of_a_call_result_is_released_by_whatever_took_it() {
    assert_statement_owns_it(
        "bug999_member_bound",
        POSITIONS,
        "    LET items AS List OF RES fs::File = mkLayer().items\n    n = n + len(items)",
        2,
    );
    assert_statement_owns_it(
        "bug999_member_unbound",
        POSITIONS,
        "    n = n + len(mkLayer().items)",
        2,
    );
}

/// bug-999, position 3: a loop takes its iterable and releases it when it ends.
#[test]
fn a_for_each_iterable_is_released_when_the_loop_ends() {
    assert_statement_owns_it(
        "bug999_foreach",
        POSITIONS,
        "    FOR EACH h IN mkList()\n      fs::writeAll(h, \"x\")\n      n = n + 1\n    NEXT",
        2,
    );
}

/// bug-999, position 4: the receiving call answers with more than a plain scalar
/// (an element, another list), and nothing takes that answer either.
#[test]
fn an_argument_of_a_call_that_answers_with_a_reference_is_dropped_with_the_statement() {
    assert_statement_owns_it(
        "bug999_get",
        POSITIONS,
        "    fs::writeAll(collections::get(mkList(), 0), \"x\")\n    n = n + 1",
        1,
    );
    assert_statement_owns_it(
        "bug999_take",
        POSITIONS,
        "    n = n + len(collections::take(mkList(), 1))",
        1,
    );
}

/// bug-999, position 5: the result reaches a resource-union box.
#[test]
fn a_result_reaching_a_resource_union_is_dropped_with_the_statement() {
    assert_statement_owns_it("bug999_union", POSITIONS, "    n = n + len(mkHandles())", 1);
}

/// bug-999, position 6: the receiving function reads a plain field of its record
/// argument and keeps nothing.
#[test]
fn a_receiver_that_reads_a_field_of_its_record_argument_keeps_nothing() {
    assert_statement_owns_it(
        "bug999_field_reader",
        POSITIONS,
        "    n = n + useH(mkHolder())",
        1,
    );
}

/// bug-999, position 7: an inline `TRAP` on the receiving call. The handler also
/// catches a `mkList` that runs out of descriptors, so the second statement has a
/// receiver that succeeds: its `n` is right only while every `mkList` succeeded.
#[test]
fn an_inline_trap_on_the_receiving_call_releases_the_argument() {
    assert_statement_owns_it(
        "bug999_trap",
        POSITIONS,
        "    LET k AS Integer = countFail(mkList()) TRAP(e)\n      RECOVER 5\n    END TRAP\n    n = n + k",
        5,
    );
    assert_statement_owns_it(
        "bug999_trap_ok",
        POSITIONS,
        "    LET k AS Integer = written(mkList()) TRAP(e)\n      RECOVER 100\n    END TRAP\n    n = n + k",
        2,
    );
}

/// bug-999: the loop takes its iterable, and an element the body puts in a list
/// declared outside the loop takes that list's scope. The kept files are written
/// after the loop; everything else the iterable held is closed and freed by it.
#[test]
fn an_element_a_loop_body_stores_further_out_takes_that_scope() {
    let name = "bug999_foreach_store";
    let source = format!(
        "{PRELUDE}\nFUNC run() AS Integer\n\
         \x20 MUT n AS Integer = 0\n\
         \x20 MUT keep AS List OF RES fs::File = []\n\
         \x20 FOR i = 1 TO count()\n\
         \x20   FOR EACH h IN mkList()\n\
         \x20     IF i <= 2 THEN\n\
         \x20       keep = collections::append(keep, h)\n\
         \x20     END IF\n\
         \x20     n = n + 1\n\
         \x20   NEXT\n\
         \x20 NEXT\n\
         \x20 FOR EACH k IN keep\n\
         \x20   fs::writeAll(k, \"still open\")\n\
         \x20   n = n + 1000\n\
         \x20 NEXT\n\
         \x20 RETURN n\n\
         END FUNC\n\n\
         FUNC main() AS Integer\n\
         \x20 io::print(\"n=\" & toString(run()))\n\
         \x20 RETURN 0\n\
         END FUNC\n"
    );
    let exe = build_debug(name, &source);
    for n in [10, 20] {
        let run = run_at(name, &exe, n);
        assert_eq!(run.double_free_skips, 0, "{name} N={n}");
        assert_eq!(
            (run.allocs, run.live),
            (run.frees, 0),
            "{name} N={n}: alloc_calls {} free_calls {} live_bytes {}",
            run.allocs,
            run.frees,
            run.live
        );
        assert_eq!(run.stdout, format!("n={}\n", 4000 + 2 * n), "{name} N={n}");
    }
    let (ok, stdout, stderr) = run_limited(&exe, 300);
    assert!(
        ok && stdout == "n=4600\n",
        "{name}: stdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

/// bug-999: a `RES` binding takes the one element or field it names; the rest of
/// the aggregate is the statement's.
#[test]
fn a_res_binding_takes_the_one_resource_it_names() {
    assert_statement_owns_it(
        "bug999_res_get",
        POSITIONS,
        "    RES h AS fs::File = collections::get(mkList(), 0)\n    fs::writeAll(h, \"x\")\n    n = n + 1",
        1,
    );
    assert_statement_owns_it(
        "bug999_res_field",
        POSITIONS,
        "    RES h AS fs::File = mkHolder().h\n    fs::writeAll(h, \"x\")\n    n = n + 1",
        1,
    );
}

/// bug-999: a binding of a call's answer over a nested list frees the inner
/// blocks it took, and the statement frees the ones it did not.
#[test]
fn a_binding_through_a_call_over_a_nested_list_ends_balanced() {
    let nested = "FUNC mkNested() AS List OF List OF RES fs::File\n\
         \x20 MUT outer AS List OF List OF RES fs::File = []\n\
         \x20 outer = collections::append(outer, mkList())\n\
         \x20 outer = collections::append(outer, mkList())\n\
         \x20 RETURN outer\n\
         END FUNC\n";
    assert_statement_owns_it(
        "bug999_take_nested",
        nested,
        "    LET x AS List OF List OF RES fs::File = collections::take(mkNested(), 1)\n    n = n + len(x)",
        1,
    );
}

/// bug-999: a record built around a call and then returned by name is the
/// caller's, with no block left behind.
#[test]
fn a_record_built_around_a_call_and_returned_by_name_ends_balanced() {
    let mk2 = "FUNC mk2() AS Layer\n\
         \x20 LET back AS Layer = Layer[items := mkList()]\n\
         \x20 RETURN back\n\
         END FUNC\n";
    assert_statement_owns_it(
        "bug999_return_named",
        mk2,
        "    LET b AS Layer = mk2()\n    n = n + len(b.items)",
        2,
    );
}

/// bug-999, position 7 over the other shapes: the trapped argument is a record
/// that holds a list by pointer, and a nested list. The binding the desugar makes
/// for it reads into the `Result` wrapper, and still owns the blocks its value
/// points at.
#[test]
fn an_inline_trap_argument_that_holds_blocks_by_pointer_frees_them() {
    let extra = "FUNC layerLen(l AS Layer) AS Integer\n\
         \x20 RETURN len(l.items)\n\
         END FUNC\n\n\
         FUNC mkNested() AS List OF List OF RES fs::File\n\
         \x20 MUT outer AS List OF List OF RES fs::File = []\n\
         \x20 outer = collections::append(outer, mkList())\n\
         \x20 outer = collections::append(outer, mkList())\n\
         \x20 RETURN outer\n\
         END FUNC\n\n\
         FUNC nestedLen(xs AS List OF List OF RES fs::File) AS Integer\n\
         \x20 RETURN len(xs)\n\
         END FUNC\n";
    assert_statement_owns_it(
        "bug999_trap_record",
        extra,
        "    LET k AS Integer = layerLen(mkLayer()) TRAP(e)\n      RECOVER 100\n    END TRAP\n    n = n + k",
        2,
    );
    assert_statement_owns_it(
        "bug999_trap_nested",
        extra,
        "    LET k AS Integer = nestedLen(mkNested()) TRAP(e)\n      RECOVER 100\n    END TRAP\n    n = n + k",
        2,
    );
}

/// bug-999: the takers the sweep added. A `RETURN` takes for the caller what a
/// binding would take; a `MATCH` takes its subject; an element stored by an
/// in-place append is its container's; and a record or a list a statement builds
/// around such a call and drops is freed with it.
#[test]
fn every_other_taker_releases_what_it_took() {
    let extra = "FUNC retTake() AS List OF RES fs::File\n\
         \x20 RETURN collections::take(mkList(), 1)\n\
         END FUNC\n\n\
         FUNC retGet() AS RES fs::File\n\
         \x20 RETURN collections::get(mkList(), 0)\n\
         END FUNC\n\n\
         FUNC retField() AS List OF RES fs::File\n\
         \x20 RETURN mkLayer().items\n\
         END FUNC\n";
    let both = format!("{POSITIONS}\n{extra}");
    assert_statement_owns_it(
        "bug999_return_take",
        &both,
        "    LET x AS List OF RES fs::File = retTake()\n    n = n + len(x) + len(retTake())",
        2,
    );
    assert_statement_owns_it(
        "bug999_return_get",
        &both,
        "    RES h AS fs::File = retGet()\n    fs::writeAll(h, \"x\")\n    n = n + 1",
        1,
    );
    assert_statement_owns_it(
        "bug999_return_field",
        &both,
        "    LET x AS List OF RES fs::File = retField()\n    n = n + len(x)",
        2,
    );
    assert_statement_owns_it(
        "bug999_match_subject",
        &both,
        "    MATCH collections::get(mkHandles(), 0)\n      CASE fs::File(f)\n        \
         fs::writeAll(f, \"x\")\n        n = n + 1\n      CASE ELSE\n        n = n + 100\n    END MATCH",
        1,
    );
    assert_statement_owns_it(
        "bug999_append_call",
        &both,
        "    MUT ls AS List OF Layer = []\n    ls = collections::append(ls, mkLayer())\n    \
         ls = collections::append(ls, Layer[items := mkList()])\n    n = n + len(ls)",
        2,
    );
    assert_statement_owns_it(
        "bug999_unbound_built",
        &both,
        "    n = n + len(Layer[items := mkList()].items) + len([mkLayer(), Layer[items := mkList()]])",
        4,
    );
    assert_statement_owns_it(
        "bug999_list_of_records",
        &both,
        "    LET xs AS List OF Layer = [Layer[items := mkList()], mkLayer()]\n    n = n + len(xs)",
        2,
    );
}
