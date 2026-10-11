//! A resource union, and a collection of resource unions, give back every block
//! they own.
//!
//! Each case runs `once()` `N` times in one `--debug` build, at two values of `N`,
//! and reads the main arena's counters at exit: as many frees as allocations, no
//! live bytes, and no double free the arena had to skip. A block kept per
//! iteration shows as a `live_bytes` that grows with `N`; a drop that frees one
//! twice shows as `double_free_skips`. `which` reads from the file it is handed,
//! so a handle closed or freed too early fails the run on the value it prints,
//! and each case also runs 300 iterations under a 64-descriptor limit, so a
//! handle never closed fails it too.
#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

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
        common::mfb_path_literal(data)
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

/// The main arena's counters after `n` iterations.
struct Counters {
    live_bytes: u64,
    double_free_skips: u64,
    alloc_calls: u64,
    free_calls: u64,
}

fn measure(case: &str, exe: &Path, n: u64, per_iteration: u64) -> Counters {
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
    Counters {
        live_bytes: read("live_bytes"),
        double_free_skips: read("double_free_skips"),
        alloc_calls: read("alloc_calls"),
        free_calls: read("free_calls"),
    }
}

/// What a case is held to.
#[derive(Clone, Copy, PartialEq)]
enum Expect {
    /// Every block is freed exactly once.
    Balanced,
    /// The program hands a reference on past the collection it was read from, so
    /// the collection is not the one owner and may keep blocks; nothing may be
    /// freed twice or read after its free.
    HeldElsewhere,
}

/// Build `body` (the declarations after the prelude, defining `once`, which must
/// return `per_iteration`) and hold it to `expect` at `N` = 10 and 40, then to a
/// 64-descriptor limit over 300 iterations.
fn check(case: &str, body: &str, per_iteration: u64, expect: Expect) {
    let nonce = common::unique_nonce();
    let dir = std::env::temp_dir().join(format!("mfb_union_reclaim_{case}_{nonce}"));
    fs::create_dir_all(&dir).expect("create data dir");
    let data: PathBuf = dir.join("data.txt");
    fs::write(&data, "hello\n").expect("write data");
    let source = format!("{}{}", prelude(&data), body);
    let exe = common::debug_report::build_debug(&format!("union_reclaim_{case}"), &source);
    for n in [10, 40] {
        let counters = measure(case, &exe, n, per_iteration);
        assert_eq!(
            counters.double_free_skips, 0,
            "{case} N={n}: a block was freed twice"
        );
        assert!(
            counters.free_calls <= counters.alloc_calls,
            "{case} N={n}: {} frees for {} allocations",
            counters.free_calls,
            counters.alloc_calls
        );
        if expect == Expect::Balanced {
            assert_eq!(
                (
                    counters.alloc_calls - counters.free_calls,
                    counters.live_bytes
                ),
                (0, 0),
                "{case} N={n}: blocks never freed and bytes still live at exit \
                 ({} allocations, {} frees)",
                counters.alloc_calls,
                counters.free_calls
            );
        }
    }
    let limited = run(&exe, 300, true);
    assert!(
        limited.ok && limited.stdout.trim() == (300 * per_iteration).to_string(),
        "{case}: 300 iterations under a 64-descriptor limit failed:\n{}\n{}",
        limited.stdout,
        limited.stderr
    );
}

/// bug-1028: a `List OF RES <Union>` whose elements a `FOR EACH` visits frees its
/// block. The loop variable is handed to a function that reads it with `MATCH`
/// (the only way a resource union can be read), read by a `MATCH` in the loop
/// body, or not read at all.
#[test]
fn an_iterated_list_of_resource_unions_frees_its_block() {
    check(
        "foreach_callee",
        "FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 RES c AS fs::File = openOne()\n\
         \x20 LET hs AS List OF RES Handle = [a, c]\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR EACH h IN hs\n\
         \x20   t = t + which(\"e\", h)\n\
         \x20 NEXT\n\
         \x20 RETURN t - 1\n\
         END FUNC\n",
        1,
        Expect::Balanced,
    );
    check(
        "foreach_inline_match",
        "FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 RES c AS fs::File = openOne()\n\
         \x20 LET hs AS List OF RES Handle = [a, c]\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR EACH h IN hs\n\
         \x20   MATCH h\n\
         \x20     CASE fs::File(f)\n\
         \x20       t = t + len(fs::readAll(f)) - 5\n\
         \x20     CASE tcp::Socket(s)\n\
         \x20       t = t + 100\n\
         \x20   END MATCH\n\
         \x20 NEXT\n\
         \x20 RETURN t - 1\n\
         END FUNC\n",
        1,
        Expect::Balanced,
    );
    check(
        "foreach_unread",
        "FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 RES c AS fs::File = openOne()\n\
         \x20 LET hs AS List OF RES Handle = [a, c]\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR EACH h IN hs\n\
         \x20   t = t + 1\n\
         \x20 NEXT\n\
         \x20 RETURN t - 1\n\
         END FUNC\n",
        1,
        Expect::Balanced,
    );
    // The loop is left by a `RETURN` from inside it.
    check(
        "foreach_early_return",
        "FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 RES c AS fs::File = openOne()\n\
         \x20 LET hs AS List OF RES Handle = [a, c]\n\
         \x20 FOR EACH h IN hs\n\
         \x20   IF which(\"e\", h) = 1 THEN\n\
         \x20     RETURN 1\n\
         \x20   END IF\n\
         \x20 NEXT\n\
         \x20 RETURN 0\n\
         END FUNC\n",
        1,
        Expect::Balanced,
    );
}

/// bug-1028: a `List`/`Map OF RES <Union>` read through `collections::get`, the
/// element written straight into a call's argument or into a `MATCH`'s scrutinee,
/// frees its block.
#[test]
fn a_collection_of_resource_unions_read_through_get_frees_its_block() {
    check(
        "append_get_callee",
        "FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 MUT hs AS List OF RES Handle = []\n\
         \x20 hs = collections::append(hs, a)\n\
         \x20 RETURN len(hs) + which(\"g\", collections::get(hs, 0)) - 1\n\
         END FUNC\n",
        1,
        Expect::Balanced,
    );
    check(
        "map_get_callee",
        "FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 LET m AS Map OF String TO RES Handle = Map OF String TO RES Handle { \"x\" := a }\n\
         \x20 RETURN which(\"m\", collections::get(m, \"x\"))\n\
         END FUNC\n",
        1,
        Expect::Balanced,
    );
    check(
        "match_of_get",
        "FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 MUT hs AS List OF RES Handle = []\n\
         \x20 hs = collections::append(hs, a)\n\
         \x20 MATCH collections::get(hs, 0)\n\
         \x20   CASE fs::File(g)\n\
         \x20     LET text AS String = fs::readAll(g)\n\
         \x20     RETURN 1\n\
         \x20   CASE tcp::Socket(s)\n\
         \x20     RETURN 2\n\
         \x20 END MATCH\n\
         \x20 RETURN 0\n\
         END FUNC\n",
        1,
        Expect::Balanced,
    );
}

/// bug-1028: the same list when its elements floated into it from a loop body
/// (§15.6), so the list's drain owns the records and the boxes' copies as well as
/// the block.
#[test]
fn a_list_of_floated_resource_unions_frees_its_block_and_records() {
    check(
        "floated_foreach",
        "FUNC once() AS Integer\n\
         \x20 MUT hs AS List OF RES Handle = []\n\
         \x20 FOR j = 1 TO 3\n\
         \x20   RES s AS Handle = openOne()\n\
         \x20   hs = collections::append(hs, s)\n\
         \x20 NEXT\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR EACH h IN hs\n\
         \x20   t = t + which(\"e\", h)\n\
         \x20 NEXT\n\
         \x20 RETURN t - 2\n\
         END FUNC\n",
        1,
        Expect::Balanced,
    );
    check(
        "floated_get",
        "FUNC once() AS Integer\n\
         \x20 MUT hs AS List OF RES Handle = []\n\
         \x20 FOR j = 1 TO 3\n\
         \x20   RES s AS Handle = openOne()\n\
         \x20   hs = collections::append(hs, s)\n\
         \x20 NEXT\n\
         \x20 RETURN which(\"a\", collections::get(hs, 0)) + which(\"b\", collections::get(hs, 2)) - 1\n\
         END FUNC\n",
        1,
        Expect::Balanced,
    );
}

/// bug-1028's other direction: a case binding that goes somewhere is a second
/// holder of the record, so the list it was read out of is not the record's one
/// owner and must not free it. `keep` outlives `hs` and reads every file after
/// `hs` has left scope, with two fresh blocks allocated in between; a list that
/// freed its records would hand those reads a reused block.
#[test]
fn a_case_binding_that_leaves_keeps_its_list_from_freeing_the_record() {
    check(
        "case_binding_returned",
        "FUNC pick(h AS Handle) AS fs::File\n\
         \x20 MATCH h\n\
         \x20   CASE fs::File(f)\n\
         \x20     RETURN f\n\
         \x20   CASE tcp::Socket(s)\n\
         \x20     FAIL error(9, \"no\")\n\
         \x20 END MATCH\n\
         \x20 FAIL error(9, \"no\")\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 MUT keep AS List OF RES fs::File = []\n\
         \x20 IF len(\"x\") = 1 THEN\n\
         \x20   MUT hs AS List OF RES Handle = []\n\
         \x20   FOR j = 1 TO 2\n\
         \x20     RES s AS Handle = openOne()\n\
         \x20     hs = collections::append(hs, s)\n\
         \x20   NEXT\n\
         \x20   FOR EACH h IN hs\n\
         \x20     keep = collections::append(keep, pick(h))\n\
         \x20   NEXT\n\
         \x20 END IF\n\
         \x20 LET junk1 AS List OF Integer = [7, 7, 7, 7, 7, 7, 7, 7]\n\
         \x20 LET junk2 AS List OF Integer = [9, 9, 9, 9, 9, 9, 9, 9]\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR EACH k IN keep\n\
         \x20   t = t + len(fs::readAll(k)) - 5\n\
         \x20 NEXT\n\
         \x20 RETURN t - 1\n\
         END FUNC\n",
        1,
        Expect::HeldElsewhere,
    );
}

/// The declarations the inline-`TRAP` cases share: a function that hands back a
/// resource union, and one that fails when asked to.
const MAKERS: &str = "FUNC mkH AS RES Handle\n\
     \x20 RETURN openOne()\n\
     END FUNC\n\
     FUNC mkBad(n AS Integer) AS RES Handle\n\
     \x20 IF n > 0 THEN\n\
     \x20   FAIL error(77, \"bad\")\n\
     \x20 END IF\n\
     \x20 RETURN openOne()\n\
     END FUNC\n";

fn check_trap(case: &str, body: &str, per_iteration: u64) {
    check(
        case,
        &format!("{MAKERS}{body}"),
        per_iteration,
        Expect::Balanced,
    );
}

/// bug-1037: `RES v AS Handle = mkH() TRAP …` keeps nothing. The desugar binds a
/// temporary to a closed default (a 16 B box and its 112 B record) and then
/// assigns the call's result over it; the default was never released.
#[test]
fn a_trapped_resource_union_binding_releases_its_default() {
    check_trap(
        "trap_return",
        "FUNC once() AS Integer\n\
         \x20 RES v AS Handle = mkH() TRAP(e)\n\
         \x20   RETURN 5\n\
         \x20 END TRAP\n\
         \x20 RETURN which(\"v\", v)\n\
         END FUNC\n",
        1,
    );
    // The handler is taken, and leaves by `FAIL` through the caller's own `TRAP`.
    check_trap(
        "trap_fail",
        "FUNC inner(n AS Integer) AS Integer\n\
         \x20 RES v AS Handle = mkBad(n) TRAP(e)\n\
         \x20   FAIL error(78, \"again\")\n\
         \x20 END TRAP\n\
         \x20 RETURN which(\"v\", v)\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 MUT r AS Integer = 0\n\
         \x20 r = inner(1) TRAP(e)\n\
         \x20   RECOVER 0\n\
         \x20 END TRAP\n\
         \x20 RETURN r + inner(0)\n\
         END FUNC\n",
        1,
    );
}

/// bug-1037: the handler's `RECOVER` is a second store into the temporary. An
/// alias (`RECOVER a`) lends it `a`'s record and a copy of `a`'s box; a call
/// hands it a record of its own. Either way the default goes first, and `a` is
/// still open afterwards.
#[test]
fn a_trapped_resource_union_binding_recovered_releases_its_default() {
    check_trap(
        "recover_alias_unused",
        "FUNC once() AS Integer\n\
         \x20 RES a AS Handle = openOne()\n\
         \x20 RES v AS Handle = mkH() TRAP(e)\n\
         \x20   RECOVER a\n\
         \x20 END TRAP\n\
         \x20 RETURN which(\"v\", v) + which(\"a\", a) - 1\n\
         END FUNC\n",
        1,
    );
    check_trap(
        "recover_alias_taken",
        "FUNC once() AS Integer\n\
         \x20 RES a AS Handle = openOne()\n\
         \x20 RES v AS Handle = mkBad(1) TRAP(e)\n\
         \x20   RECOVER a\n\
         \x20 END TRAP\n\
         \x20 RETURN which(\"v\", v) + which(\"a\", a) - 1\n\
         END FUNC\n",
        1,
    );
    check_trap(
        "recover_call_taken",
        "FUNC once() AS Integer\n\
         \x20 RES v AS Handle = mkBad(1) TRAP(e)\n\
         \x20   RECOVER openOne()\n\
         \x20 END TRAP\n\
         \x20 RETURN which(\"v\", v)\n\
         END FUNC\n",
        1,
    );
}

/// bug-1037 in a loop body: the temporary is bound again on every pass, the
/// handler leaves by `CONTINUE FOR` on one of them, and in the second case the
/// binding floats into a list that outlives the loop (§15.6).
#[test]
fn a_trapped_resource_union_binding_in_a_loop_releases_its_default() {
    check_trap(
        "loop_continue",
        "FUNC once() AS Integer\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR j = 1 TO 3\n\
         \x20   RES v AS Handle = mkBad(j - 2) TRAP(e)\n\
         \x20     CONTINUE FOR\n\
         \x20   END TRAP\n\
         \x20   t = t + which(\"v\", v)\n\
         \x20 NEXT\n\
         \x20 RETURN t - 1\n\
         END FUNC\n",
        1,
    );
    check_trap(
        "loop_floated",
        "FUNC once() AS Integer\n\
         \x20 MUT hs AS List OF RES Handle = []\n\
         \x20 FOR j = 1 TO 3\n\
         \x20   RES s AS Handle = mkBad(j - 2) TRAP(e)\n\
         \x20     CONTINUE FOR\n\
         \x20   END TRAP\n\
         \x20   hs = collections::append(hs, s)\n\
         \x20 NEXT\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR EACH h IN hs\n\
         \x20   t = t + which(\"e\", h)\n\
         \x20 NEXT\n\
         \x20 RETURN t - 1\n\
         END FUNC\n",
        1,
    );
}

/// The declarations the returned-record cases share: a record holding a file, a
/// function that builds one around its parameter, and one that opens the file
/// and returns that record.
const HOLDERS: &str = "TYPE HB2\n\
     \x20 h AS RES fs::File\n\
     END TYPE\n\
     FUNC keep2(h AS fs::File) AS HB2\n\
     \x20 RETURN HB2[h]\n\
     END FUNC\n\
     FUNC mk2 AS HB2\n\
     \x20 RES u AS fs::File = openOne()\n\
     \x20 RETURN keep2(u)\n\
     END FUNC\n\
     FUNC rd(f AS fs::File) AS Integer\n\
     \x20 RETURN len(fs::readAll(f)) - 5\n\
     END FUNC\n";

fn check_holder(case: &str, body: &str, per_iteration: u64, expect: Expect) {
    check(case, &format!("{HOLDERS}{body}"), per_iteration, expect);
}

/// bug-1025: a record a callee built around a resource its caller's callee opened
/// is adopted by the binding of the call's result, which closes the file and
/// frees its record. The record was closed and never freed: a record binding was
/// never the proven owner of what it adopted.
#[test]
fn a_record_binding_frees_the_resource_record_it_adopted() {
    check_holder(
        "read_through_field",
        "FUNC once() AS Integer\n\
         \x20 LET k AS HB2 = mk2()\n\
         \x20 RETURN len(fs::readAll(k.h)) - 5\n\
         END FUNC\n",
        1,
        Expect::Balanced,
    );
    // A `RETURN` the record's scope can be left by, and a copy of the record bound
    // after it. The second read is at the end of the file, so `rd` gives -5.
    check_holder(
        "early_return_and_copy",
        "FUNC once() AS Integer\n\
         \x20 LET k AS HB2 = mk2()\n\
         \x20 IF rd(k.h) = 7 THEN\n\
         \x20   RETURN 7\n\
         \x20 END IF\n\
         \x20 LET k2 AS HB2 = k\n\
         \x20 RETURN rd(k2.h) + 6\n\
         END FUNC\n",
        1,
        Expect::Balanced,
    );
    // Bound in a loop body that one pass leaves by `CONTINUE FOR`.
    check_holder(
        "loop_body",
        "FUNC once() AS Integer\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR j = 1 TO 3\n\
         \x20   LET k AS HB2 = mk2()\n\
         \x20   IF j = 2 THEN\n\
         \x20     CONTINUE FOR\n\
         \x20   END IF\n\
         \x20   t = t + rd(k.h)\n\
         \x20 NEXT\n\
         \x20 RETURN t - 1\n\
         END FUNC\n",
        1,
        Expect::Balanced,
    );
}

/// bug-1025, the second reproduction: a four-field record, read with
/// `fs::readLine` (so the file owns a 16 KiB read buffer), copied with `WITH` and
/// by a plain bind, and compared field by field. The copies hold the same handle
/// and stay in the scope of the record they were made from.
#[test]
fn a_copied_record_binding_frees_the_record_and_its_read_buffer() {
    check(
        "copies_and_comparisons",
        "TYPE HB\n\
         \x20 name AS String\n\
         \x20 h AS RES fs::File\n\
         \x20 n AS Integer\n\
         \x20 tail AS String\n\
         END TYPE\n\
         FUNC keep(nm AS String, h AS fs::File, n AS Integer) AS HB\n\
         \x20 RETURN HB[nm, h, n, \"tail\" & toString(n)]\n\
         END FUNC\n\
         FUNC mk(n AS Integer) AS HB\n\
         \x20 RES u AS fs::File = openOne()\n\
         \x20 RETURN keep(\"name\" & toString(n), u, n)\n\
         END FUNC\n\
         FUNC readOne(f AS fs::File) AS Integer\n\
         \x20 RETURN len(fs::readLine(f))\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 LET k AS HB = mk(3)\n\
         \x20 LET junk1 AS List OF Integer = [7, 7, 7, 7]\n\
         \x20 LET k2 AS HB = WITH k { n := 9 }\n\
         \x20 LET junk2 AS String = \"abcdefghijklmnopqrstuvwxyz\" & toString(k2.n)\n\
         \x20 LET k3 AS HB = k2\n\
         \x20 MUT r AS Integer = 0\n\
         \x20 IF k.name = \"name3\" AND k.tail = \"tail3\" AND k.n = 3 AND k3.n = 9 AND k3.tail = \"tail3\" AND k2.name = \"name3\" THEN\n\
         \x20   r = 1\n\
         \x20 END IF\n\
         \x20 RETURN r * 100 + readOne(k3.h)\n\
         END FUNC\n",
        105,
        Expect::Balanced,
    );
}

/// bug-1025's other direction: a handle that leaves the record binding is a
/// second holder, so the binding must not free the record. In the first case the
/// record is handed on to a binding in an outer scope, which reads the file after
/// the inner scope has ended and two fresh blocks have been allocated; in the
/// second the handle is bound on its own.
#[test]
fn a_record_binding_whose_handle_leaves_does_not_free_the_record() {
    check_holder(
        "record_handed_outward",
        "FUNC stash(b AS HB2) AS HB2\n\
         \x20 RETURN b\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 RES z AS fs::File = openOne()\n\
         \x20 MUT outer AS HB2 = keep2(z)\n\
         \x20 IF len(\"x\") = 1 THEN\n\
         \x20   LET k AS HB2 = mk2()\n\
         \x20   outer = stash(k)\n\
         \x20 END IF\n\
         \x20 LET junk1 AS List OF Integer = [7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7]\n\
         \x20 LET junk2 AS List OF Integer = [9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9]\n\
         \x20 RETURN rd(outer.h)\n\
         END FUNC\n",
        1,
        Expect::HeldElsewhere,
    );
    check_holder(
        "handle_bound",
        "FUNC once() AS Integer\n\
         \x20 LET k AS HB2 = mk2()\n\
         \x20 RES f AS fs::File = k.h\n\
         \x20 RETURN rd(f)\n\
         END FUNC\n",
        1,
        Expect::HeldElsewhere,
    );
}
