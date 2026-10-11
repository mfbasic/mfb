//! bug-1012: a resource-union `{tag, record-ptr}` box has exactly one owner.
//!
//! A resource-union value (`UNION Handle  fs::File  tcp::Socket`) is a 16-byte box.
//! Only a binding's drop used to free one, and every other holder kept the box by
//! pointer. So a box wrapped at a call argument, a list element or a record field
//! leaked 16 B per evaluation; a record whose `RES Handle` field held a binding's
//! box read freed memory once that binding left scope; and the drain of a list a
//! callee returned freed a 16-byte span inside the list's own block.
//!
//! Every case runs `once()` `N` times and reads the `--debug` report at two values
//! of `N`: the bytes still live at exit may grow by at most the closed record a
//! resource temporary leaves (112 B, bug-842), and nothing is freed twice. `which`
//! reads from the file it is handed, so a handle closed too early fails the run,
//! and each case also runs 300 iterations under a 64-descriptor limit, so a handle
//! never closed fails it too.

#[path = "../common/mod.rs"]
mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The bytes a resource temporary's closed record keeps (bug-842).
const TOMBSTONE: u64 = 112;

fn prelude(data: &Path) -> String {
    format!(
        "IMPORT io\n\
         IMPORT os\n\
         IMPORT fs\n\
         IMPORT tcp\n\
         IMPORT collections\n\
         UNION Handle\n\
         \x20 fs::File\n\
         \x20 tcp::Socket\n\
         END UNION\n\
         TYPE HB\n\
         \x20 h AS RES Handle\n\
         END TYPE\n\
         TYPE Outer\n\
         \x20 n AS Integer\n\
         \x20 inner AS HB\n\
         END TYPE\n\
         FUNC openOne AS RES fs::File\n\
         \x20 RETURN fs::openFile(\"{}\")\n\
         END FUNC\n\
         FUNC which(label AS String, h AS Handle) AS Integer\n\
         \x20 MATCH h\n\
         \x20   CASE fs::File(f)\n\
         \x20     LET text AS String = fs::readAll(f)\n\
         \x20     RETURN 1\n\
         \x20   CASE tcp::Socket(s)\n\
         \x20     RETURN 2\n\
         \x20 END MATCH\n\
         \x20 RETURN 0\n\
         END FUNC\n\
         FUNC main AS Integer\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR i = 1 TO toInt(os::getEnvOr(\"N\", \"10\"))\n\
         \x20   t = t + once()\n\
         \x20 NEXT\n\
         \x20 io::print(toString(t))\n\
         \x20 RETURN 0\n\
         END FUNC\n",
        data.display()
    )
}

struct Run {
    ok: bool,
    stdout: String,
    stderr: String,
}

fn run(exe: &Path, n: u64, limit_descriptors: bool) -> Run {
    let out = if limit_descriptors {
        Command::new("sh")
            .arg("-c")
            .arg("ulimit -n 64 && exec \"$0\"")
            .arg(exe)
            .env("N", n.to_string())
            .output()
    } else {
        Command::new(exe).env("N", n.to_string()).output()
    }
    .expect("run the program");
    Run {
        ok: out.status.success(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// One measured run: `(live_bytes, double_free_skips, alloc_calls, free_calls)`.
fn measure(case: &str, exe: &Path, n: u64, per_iteration: u64) -> (u64, u64, u64, u64) {
    let result = run(exe, n, false);
    assert!(
        result.ok,
        "{case} N={n} failed:\n{}\n{}",
        result.stdout, result.stderr
    );
    assert_eq!(
        result.stdout.trim(),
        (n * per_iteration).to_string(),
        "{case} N={n} printed the wrong total"
    );
    let lines = common::debug_report::arena_lines(case, &result.stderr);
    let read = |name: &str| common::debug_report::counter(case, &lines, 0, name);
    (
        read("live_bytes"),
        read("double_free_skips"),
        read("alloc_calls"),
        read("free_calls"),
    )
}

/// Build `body` (the declarations after the prelude, defining `once`) and hold it
/// to the box rule: at most `slope` live bytes per iteration, no double free, no
/// free without an allocation, and no descriptor left open or closed early.
fn check(case: &str, body: &str, per_iteration: u64, slope: u64) {
    let nonce = common::unique_nonce();
    let dir = std::env::temp_dir().join(format!("mfb_box_owner_{case}_{nonce}"));
    fs::create_dir_all(&dir).expect("create data dir");
    let data: PathBuf = dir.join("data.txt");
    fs::write(&data, "hello\n").expect("write data");
    let source = format!("{}{}", prelude(&data), body);
    let exe = common::debug_report::build_debug(&format!("box_owner_{case}"), &source);
    let (live_a, skips_a, allocs_a, frees_a) = measure(case, &exe, 10, per_iteration);
    let (live_b, skips_b, allocs_b, frees_b) = measure(case, &exe, 40, per_iteration);
    assert_eq!(
        (skips_a, skips_b),
        (0, 0),
        "{case}: a block was freed twice (double_free_skips at N=10, N=40)"
    );
    assert!(
        frees_a <= allocs_a && frees_b <= allocs_b,
        "{case}: more frees than allocations ({frees_a}/{allocs_a} at N=10, \
         {frees_b}/{allocs_b} at N=40)"
    );
    let growth = live_b.saturating_sub(live_a);
    assert!(
        growth <= slope * 30,
        "{case}: live bytes grew {growth} over 30 iterations ({live_a} at N=10, {live_b} at \
         N=40), more than {slope} per iteration"
    );
    let limited = run(&exe, 300, true);
    assert!(
        limited.ok && limited.stdout.trim() == (300 * per_iteration).to_string(),
        "{case}: 300 iterations under a 64-descriptor limit failed:\n{}\n{}",
        limited.stdout,
        limited.stderr
    );
}

/// Shape (a): a record whose `RES Handle` field was built from a callee's
/// parameter, returned past the binding that wrapped the box.
///
/// These two may keep the closed file record (112 B) per iteration: a record a
/// callee hands back inside a record is closed by the caller's binding and not
/// freed, with a concrete `RES fs::File` field as well. That is the record, not a
/// box, and the box rule is what is held here.
#[test]
fn a_record_field_outlives_the_binding_that_wrapped_its_box() {
    check(
        "kept",
        "FUNC keep(h AS Handle) AS HB\n\
         \x20 RETURN HB[h]\n\
         END FUNC\n\
         FUNC mkKept AS HB\n\
         \x20 RES u AS Handle = openOne()\n\
         \x20 RETURN keep(u)\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 LET k AS HB = mkKept()\n\
         \x20 LET junk1 AS List OF Integer = [7, 7]\n\
         \x20 LET junk2 AS List OF Integer = [9, 9]\n\
         \x20 RETURN which(\"kept\", k.h)\n\
         END FUNC\n",
        1,
        TOMBSTONE,
    );
    check(
        "kept_two_frames",
        "FUNC keep(h AS Handle) AS HB\n\
         \x20 RETURN HB[h]\n\
         END FUNC\n\
         FUNC mid(h AS Handle) AS HB\n\
         \x20 RETURN keep(h)\n\
         END FUNC\n\
         FUNC mkKept AS HB\n\
         \x20 RES u AS Handle = openOne()\n\
         \x20 RETURN mid(u)\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 LET k AS HB = mkKept()\n\
         \x20 LET junk1 AS List OF Integer = [7, 7]\n\
         \x20 RETURN which(\"kept\", k.h)\n\
         END FUNC\n",
        1,
        TOMBSTONE,
    );
}

/// Shape (b): a box wrapped at a call argument.
#[test]
fn a_box_wrapped_at_a_call_argument_is_freed() {
    check(
        "arg_var",
        "FUNC once() AS Integer\n\
         \x20 RES a0 AS fs::File = openOne()\n\
         \x20 RETURN which(\"var\", a0)\n\
         END FUNC\n",
        1,
        0,
    );
    check(
        "arg_call",
        "FUNC once() AS Integer\n\
         \x20 RETURN which(\"call\", openOne())\n\
         END FUNC\n",
        1,
        TOMBSTONE,
    );
}

/// Shape (b): a box wrapped as a `List OF RES Handle` literal element.
#[test]
fn a_box_wrapped_at_a_list_element_is_freed() {
    check(
        "lit_var",
        "FUNC once() AS Integer\n\
         \x20 RES a0 AS fs::File = openOne()\n\
         \x20 LET hs AS List OF RES Handle = [a0]\n\
         \x20 RETURN len(hs)\n\
         END FUNC\n",
        1,
        0,
    );
    check(
        "lit_call",
        "FUNC once() AS Integer\n\
         \x20 LET hs AS List OF RES Handle = [openOne()]\n\
         \x20 RETURN len(hs)\n\
         END FUNC\n",
        1,
        0,
    );
}

/// Shape (b): a box wrapped as a `RES Handle` record field.
#[test]
fn a_box_wrapped_at_a_record_field_is_freed() {
    check(
        "field_var",
        "FUNC once() AS Integer\n\
         \x20 RES a0 AS fs::File = openOne()\n\
         \x20 LET b AS HB = HB[a0]\n\
         \x20 RETURN which(\"f\", b.h)\n\
         END FUNC\n",
        1,
        0,
    );
    check(
        "field_nested",
        "FUNC once() AS Integer\n\
         \x20 RES a0 AS fs::File = openOne()\n\
         \x20 LET o AS Outer = Outer[3, HB[a0]]\n\
         \x20 LET p AS Outer = o\n\
         \x20 RETURN which(\"o\", o.inner.h) + which(\"p\", p.inner.h)\n\
         END FUNC\n",
        2,
        0,
    );
}

/// Shape (c): a `List OF RES Handle` literal returned around a binding.
#[test]
fn a_returned_list_of_boxes_is_not_freed_twice() {
    check(
        "returned_literal",
        "FUNC mkStreams() AS List OF RES Handle\n\
         \x20 RES s AS Handle = openOne()\n\
         \x20 RETURN [s]\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 LET xs = mkStreams()\n\
         \x20 RETURN len(xs)\n\
         END FUNC\n",
        1,
        0,
    );
}

/// Shape (d): the same list, its element floated into it in the callee.
#[test]
fn a_returned_list_with_a_floated_box_is_not_freed_twice() {
    check(
        "returned_floated",
        "FUNC mkStreams() AS List OF RES Handle\n\
         \x20 MUT xs AS List OF RES Handle = []\n\
         \x20 IF len(\"x\") = 1 THEN\n\
         \x20   RES s AS Handle = openOne()\n\
         \x20   xs = collections::append(xs, s)\n\
         \x20 END IF\n\
         \x20 RETURN xs\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 LET xs = mkStreams()\n\
         \x20 RETURN len(xs)\n\
         END FUNC\n",
        1,
        0,
    );
}

/// The forms that were balanced before the fix stay balanced.
#[test]
fn a_binding_that_owns_its_box_stays_balanced() {
    check(
        "bind",
        "FUNC once() AS Integer\n\
         \x20 RES one AS Handle = openOne()\n\
         \x20 RETURN which(\"b\", one)\n\
         END FUNC\n",
        1,
        0,
    );
    check(
        "bind_literal",
        "FUNC once() AS Integer\n\
         \x20 RES a AS Handle = openOne()\n\
         \x20 LET hs AS List OF RES Handle = [a]\n\
         \x20 RETURN len(hs)\n\
         END FUNC\n",
        1,
        0,
    );
    check(
        "bind_append",
        "FUNC once() AS Integer\n\
         \x20 RES a AS Handle = openOne()\n\
         \x20 MUT hs AS List OF RES Handle = []\n\
         \x20 hs = collections::append(hs, a)\n\
         \x20 RETURN len(hs)\n\
         END FUNC\n",
        1,
        0,
    );
}

/// A binding initialized from a record field owns a copy of the field's box, so
/// replacing the record does not take the box away from under it.
#[test]
fn a_binding_of_a_field_outlives_the_record_it_was_read_from() {
    check(
        "field_bind_reassign",
        "FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 RES c AS fs::File = openOne()\n\
         \x20 MUT b AS HB = HB[a]\n\
         \x20 RES v AS Handle = b.h\n\
         \x20 b = HB[c]\n\
         \x20 LET junk1 AS List OF Integer = [7, 7]\n\
         \x20 LET junk2 AS List OF Integer = [9, 9]\n\
         \x20 RETURN which(\"v\", v) + which(\"b\", b.h) - 1\n\
         END FUNC\n",
        1,
        0,
    );
}

/// The other places a record with a `RES Handle` field is built, copied or read.
#[test]
fn a_record_holding_a_box_is_copied_and_dropped_whole() {
    check(
        "with_update",
        "FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 RES c AS fs::File = openOne()\n\
         \x20 MUT b AS HB = HB[a]\n\
         \x20 LET b2 AS HB = WITH b { h := c }\n\
         \x20 b = WITH b { h := c }\n\
         \x20 MUT o AS Outer = Outer[3, HB[a]]\n\
         \x20 o = WITH o { inner := b2 }\n\
         \x20 o = WITH o { n := 4 }\n\
         \x20 RETURN which(\"b\", b.h) + which(\"b2\", b2.h) + which(\"o\", o.inner.h) + o.n - 6\n\
         END FUNC\n",
        1,
        0,
    );
    check(
        "match_field_and_capture",
        "FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 LET b AS HB = HB[a]\n\
         \x20 MUT r AS Integer = 0\n\
         \x20 MATCH b.h\n\
         \x20   CASE fs::File(f)\n\
         \x20     r = 1\n\
         \x20   CASE tcp::Socket(s)\n\
         \x20     r = 2\n\
         \x20 END MATCH\n\
         \x20 LET g = LAMBDA() -> which(\"c\", b.h)\n\
         \x20 RETURN r + g() + g() - 2\n\
         END FUNC\n",
        1,
        0,
    );
    check(
        "records_in_collections",
        "FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 MUT bs AS List OF HB = []\n\
         \x20 FOR j = 1 TO 3\n\
         \x20   bs = collections::append(bs, HB[a])\n\
         \x20 NEXT\n\
         \x20 LET m AS Map OF String TO HB = Map OF String TO HB { \"x\" := HB[a] }\n\
         \x20 MUT t AS Integer = which(\"m\", collections::get(m, \"x\").h)\n\
         \x20 FOR EACH b IN bs\n\
         \x20   t = t + which(\"e\", b.h)\n\
         \x20 NEXT\n\
         \x20 RETURN t - 3\n\
         END FUNC\n",
        1,
        0,
    );
}

/// A statement that is left early — a failing call, a `FAIL`, an `EXIT`, a
/// `RETURN` — still frees the boxes it had built.
#[test]
fn a_statement_left_early_frees_its_boxes() {
    check(
        "unwind",
        "FUNC boom(h AS Handle, n AS Integer) AS Integer\n\
         \x20 IF n > 0 THEN\n\
         \x20   FAIL error(77, \"boom\")\n\
         \x20 END IF\n\
         \x20 RETURN 1\n\
         END FUNC\n\
         FUNC inner(n AS Integer) AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 LET b AS HB = HB[a]\n\
         \x20 LET x AS Integer = which(\"a\", a) + boom(b.h, 0) + boom(a, n)\n\
         \x20 IF which(\"b\", a) = 1 THEN\n\
         \x20   FAIL error(78, \"direct\")\n\
         \x20 END IF\n\
         \x20 RETURN x\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 MUT r AS Integer = 0\n\
         \x20 r = inner(1) TRAP(e)\n\
         \x20   RECOVER 0\n\
         \x20 END TRAP\n\
         \x20 MUT q AS Integer = 0\n\
         \x20 q = inner(0) TRAP(e)\n\
         \x20   RECOVER 1\n\
         \x20 END TRAP\n\
         \x20 RES c AS fs::File = openOne()\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR j = 1 TO 5\n\
         \x20   IF which(\"c\", c) + j = 3 THEN\n\
         \x20     EXIT FOR\n\
         \x20   END IF\n\
         \x20   t = t + 1\n\
         \x20 NEXT\n\
         \x20 IF which(\"x\", c) = 1 THEN\n\
         \x20   RETURN r + q\n\
         \x20 END IF\n\
         \x20 RETURN t\n\
         END FUNC\n",
        1,
        0,
    );
}

/// A function that returns a resource union hands its caller a box the caller
/// owns: a binding frees it at scope exit, an unbound call at statement end.
#[test]
fn a_returned_box_is_freed_by_its_caller() {
    check(
        "returned_box",
        "FUNC mkH AS RES Handle\n\
         \x20 RETURN openOne()\n\
         END FUNC\n\
         FUNC pick(b AS HB) AS Handle\n\
         \x20 RETURN b.h\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 RES h AS Handle = mkH()\n\
         \x20 LET b AS HB = HB[h]\n\
         \x20 RETURN which(\"x\", mkH()) + which(\"h\", h) + which(\"p\", pick(b)) - 2\n\
         END FUNC\n",
        1,
        TOMBSTONE,
    );
}

/// bug-1026: a binding initialized from `collections::get`/`getOr` owns a copy of
/// the box it was handed. The call yields the address of the element's two words
/// inside the collection's block, and the block is freed when the collection
/// grows; `v` used to keep that address, and `MATCH v` then read a freed block
/// (`which` returned 0 because no case matched the fill pattern).
///
/// The slope each case allows is the collection's own final block and nothing
/// else. A `List OF RES <Union>` read through `collections::get` keeps its block
/// (`record_ownership.rs:block_held_elsewhere_strict`, a documented bounded leak
/// this bug does not touch): 6,192 B for the 41-element list and 5,168 B for the
/// 41-entry map, the same figures the compiler without this fix reports for these
/// programs. A leaked box would add 16 B per iteration to them.
#[test]
fn a_binding_of_a_list_element_outlives_the_block_it_was_read_from() {
    check(
        "list_get_grow",
        "FUNC once() AS Integer\n\
         \x20 RES a AS Handle = openOne()\n\
         \x20 RES c AS Handle = openOne()\n\
         \x20 MUT hs AS List OF RES Handle = []\n\
         \x20 hs = collections::append(hs, a)\n\
         \x20 RES v AS Handle = collections::get(hs, 0)\n\
         \x20 FOR j = 1 TO 40\n\
         \x20   hs = collections::append(hs, c)\n\
         \x20 NEXT\n\
         \x20 LET junk1 AS List OF Integer = [7, 7, 7, 7, 7, 7, 7, 7]\n\
         \x20 LET junk2 AS List OF Integer = [9, 9, 9, 9, 9, 9, 9, 9]\n\
         \x20 RETURN which(\"v\", v)\n\
         END FUNC\n",
        1,
        6192,
    );
    check(
        "list_getor_grow",
        "FUNC once() AS Integer\n\
         \x20 RES a AS Handle = openOne()\n\
         \x20 RES c AS Handle = openOne()\n\
         \x20 MUT hs AS List OF RES Handle = []\n\
         \x20 hs = collections::append(hs, a)\n\
         \x20 RES v AS Handle = collections::getOr(hs, 0, c)\n\
         \x20 FOR j = 1 TO 40\n\
         \x20   hs = collections::append(hs, c)\n\
         \x20 NEXT\n\
         \x20 LET junk1 AS List OF Integer = [7, 7, 7, 7, 7, 7, 7, 7]\n\
         \x20 LET junk2 AS List OF Integer = [9, 9, 9, 9, 9, 9, 9, 9]\n\
         \x20 RETURN which(\"v\", v)\n\
         END FUNC\n",
        1,
        6192,
    );
}

/// bug-1026, a `Map` value: the same read, one container kind over.
#[test]
fn a_binding_of_a_map_value_outlives_the_block_it_was_read_from() {
    check(
        "map_get_grow",
        "FUNC once() AS Integer\n\
         \x20 RES a AS Handle = openOne()\n\
         \x20 RES c AS Handle = openOne()\n\
         \x20 MUT m AS Map OF Integer TO RES Handle = Map OF Integer TO RES Handle { 0 := a }\n\
         \x20 RES v AS Handle = collections::get(m, 0)\n\
         \x20 FOR j = 1 TO 40\n\
         \x20   m = collections::set(m, j, c)\n\
         \x20 NEXT\n\
         \x20 LET junk1 AS List OF Integer = [7, 7, 7, 7, 7, 7, 7, 7]\n\
         \x20 LET junk2 AS List OF Integer = [9, 9, 9, 9, 9, 9, 9, 9]\n\
         \x20 RETURN which(\"v\", v)\n\
         END FUNC\n",
        1,
        5168,
    );
}

/// bug-1026, the default of `getOr`: a variant written as the default is wrapped
/// in a box the statement owns and frees at its end, so the binding that received
/// it kept the address of a freed box (exit 139 inside `fs::readAll`). The slope is
/// the one-element list's block, 240 B.
#[test]
fn a_binding_of_a_getor_default_outlives_the_statement_that_wrapped_it() {
    check(
        "list_getor_default",
        "FUNC once() AS Integer\n\
         \x20 RES a AS Handle = openOne()\n\
         \x20 RES f AS fs::File = openOne()\n\
         \x20 MUT hs AS List OF RES Handle = []\n\
         \x20 hs = collections::append(hs, a)\n\
         \x20 RES v AS Handle = collections::getOr(hs, 7, f)\n\
         \x20 LET junk0 AS List OF Integer = [5, 5]\n\
         \x20 LET junk1 AS List OF Integer = [7, 7]\n\
         \x20 LET junk2 AS List OF Integer = [9, 9]\n\
         \x20 RETURN which(\"v\", v)\n\
         END FUNC\n",
        1,
        240,
    );
    // The same default as a `MATCH` scrutinee: the temporary the scrutinee is
    // bound to is read after the statement that wrapped the default has ended.
    check(
        "match_getor_default",
        "FUNC once() AS Integer\n\
         \x20 RES a AS Handle = openOne()\n\
         \x20 RES f AS fs::File = openOne()\n\
         \x20 MUT hs AS List OF RES Handle = []\n\
         \x20 hs = collections::append(hs, a)\n\
         \x20 MATCH collections::getOr(hs, 7, f)\n\
         \x20   CASE fs::File(g)\n\
         \x20     LET junk0 AS List OF Integer = [5, 5]\n\
         \x20     LET text AS String = fs::readAll(g)\n\
         \x20     RETURN 1\n\
         \x20   CASE tcp::Socket(s)\n\
         \x20     RETURN 2\n\
         \x20 END MATCH\n\
         \x20 RETURN 0\n\
         END FUNC\n",
        1,
        240,
    );
}

/// bug-1026 through the inline-`TRAP` desugar, which delivers the value through a
/// temp the user binding aliases. A borrowed element reaches the temp as the
/// trapped call's `resultValue`, and an aliasing `RECOVER` as a plain store; both
/// lend the temp a record and must hand it a copy of the box.
///
/// Each slope is the collection's kept block plus 128 B: the closed default the
/// temp's bind materializes (a 16 B box and its 112 B record), which no store into
/// a resource-union temp releases. `RES v AS Handle = mkH() TRAP …`, with no
/// collection in it, keeps the same 128 B per evaluation.
#[test]
fn a_trapped_binding_of_an_element_outlives_the_block_it_was_read_from() {
    check(
        "trap_list_get_grow",
        "FUNC once() AS Integer\n\
         \x20 RES a AS Handle = openOne()\n\
         \x20 RES c AS Handle = openOne()\n\
         \x20 MUT hs AS List OF RES Handle = []\n\
         \x20 hs = collections::append(hs, a)\n\
         \x20 RES v AS Handle = collections::get(hs, 0) TRAP(e)\n\
         \x20   RETURN 5\n\
         \x20 END TRAP\n\
         \x20 FOR j = 1 TO 40\n\
         \x20   hs = collections::append(hs, c)\n\
         \x20 NEXT\n\
         \x20 LET junk1 AS List OF Integer = [7, 7, 7, 7, 7, 7, 7, 7]\n\
         \x20 LET junk2 AS List OF Integer = [9, 9, 9, 9, 9, 9, 9, 9]\n\
         \x20 RETURN which(\"v\", v)\n\
         END FUNC\n",
        1,
        6192 + 128,
    );
    check(
        "trap_map_get_grow",
        "FUNC once() AS Integer\n\
         \x20 RES a AS Handle = openOne()\n\
         \x20 RES c AS Handle = openOne()\n\
         \x20 MUT m AS Map OF Integer TO RES Handle = Map OF Integer TO RES Handle { 0 := a }\n\
         \x20 RES v AS Handle = collections::get(m, 0) TRAP(e)\n\
         \x20   RETURN 5\n\
         \x20 END TRAP\n\
         \x20 FOR j = 1 TO 40\n\
         \x20   m = collections::set(m, j, c)\n\
         \x20 NEXT\n\
         \x20 LET junk1 AS List OF Integer = [7, 7, 7, 7, 7, 7, 7, 7]\n\
         \x20 LET junk2 AS List OF Integer = [9, 9, 9, 9, 9, 9, 9, 9]\n\
         \x20 RETURN which(\"v\", v)\n\
         END FUNC\n",
        1,
        5168 + 128,
    );
    check(
        "trap_getor_default",
        "FUNC once() AS Integer\n\
         \x20 RES a AS Handle = openOne()\n\
         \x20 RES f AS fs::File = openOne()\n\
         \x20 MUT hs AS List OF RES Handle = []\n\
         \x20 hs = collections::append(hs, a)\n\
         \x20 RES v AS Handle = collections::getOr(hs, 7, f) TRAP(e)\n\
         \x20   RETURN 5\n\
         \x20 END TRAP\n\
         \x20 LET junk0 AS List OF Integer = [5, 5]\n\
         \x20 RETURN which(\"v\", v)\n\
         END FUNC\n",
        1,
        240 + 128,
    );
}

/// bug-1026: the value an inline `TRAP` recovers with is lent the same way. A
/// field read names an address inside the record's block, which replacing the
/// record frees; a bare local names another binding's box.
#[test]
fn a_trapped_binding_recovered_with_an_alias_owns_its_box() {
    check(
        "trap_recover_field",
        "FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 RES c AS fs::File = openOne()\n\
         \x20 MUT b AS HB = HB[a]\n\
         \x20 MUT hs AS List OF RES Handle = []\n\
         \x20 RES v AS Handle = collections::get(hs, 3) TRAP(e)\n\
         \x20   RECOVER b.h\n\
         \x20 END TRAP\n\
         \x20 b = HB[c]\n\
         \x20 LET junk1 AS List OF Integer = [7, 7]\n\
         \x20 LET junk2 AS List OF Integer = [9, 9]\n\
         \x20 RETURN which(\"v\", v)\n\
         END FUNC\n",
        1,
        48 + 128,
    );
    check(
        "trap_recover_local",
        "FUNC once() AS Integer\n\
         \x20 RES a AS Handle = openOne()\n\
         \x20 MUT hs AS List OF RES Handle = []\n\
         \x20 RES v AS Handle = collections::get(hs, 3) TRAP(e)\n\
         \x20   RECOVER a\n\
         \x20 END TRAP\n\
         \x20 RETURN which(\"v\", v) + which(\"a\", a) - 1\n\
         END FUNC\n",
        1,
        48 + 128,
    );
}
