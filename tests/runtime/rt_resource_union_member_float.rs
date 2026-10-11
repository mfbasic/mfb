//! bug-1046: a concrete resource stored into a collection of its resource union.
//!
//! `RES s AS fs::File`, then `hs = collections::append(hs, s)` on a `List OF RES
//! Handle` (`UNION Handle  fs::File  tcp::Socket`). When the binding is declared in
//! a scope inside the container's, its close obligation floats to the container
//! (language spec 15.6) and the container's owned-list takes the record. The
//! owned-list of a union container holds `{tag, record, next}` nodes, and the float
//! is decided at the binding, before the store wraps the value: the push read the
//! member's record as a `{tag, record}` box, took its handle word (a descriptor
//! number) for the record pointer, and the program died with SIGSEGV. With that
//! fixed, the list still closed the records and freed none of them, because the
//! wrap around the member at the store made the member look held elsewhere.
//!
//! Every case runs `once()` `N` times and reads the `--debug` report at two values
//! of `N`: the total printed proves each element is tagged as its member (the
//! reader takes the `fs::File` arm and reads the file through it), every
//! allocation is freed, nothing is freed twice, and 300 iterations under a
//! 64-descriptor limit prove each descriptor is closed.

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
         \n\
         UNION Handle\n\
         \x20 fs::File\n\
         \x20 tcp::Socket\n\
         END UNION\n\
         \n\
         TYPE HB\n\
         \x20 h AS RES Handle\n\
         \x20 n AS Integer\n\
         END TYPE\n\
         \n\
         FUNC openOne AS RES fs::File\n\
         \x20 RETURN fs::openFile(\"{}\")\n\
         END FUNC\n\
         \n\
         FUNC countH(hs AS List OF RES Handle) AS Integer\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR EACH h IN hs\n\
         \x20   MATCH h\n\
         \x20     CASE fs::File(f)\n\
         \x20       t = t + len(fs::readAll(f))\n\
         \x20     CASE tcp::Socket(s)\n\
         \x20       t = t + 100\n\
         \x20   END MATCH\n\
         \x20 NEXT\n\
         \x20 RETURN t\n\
         END FUNC\n\
         \n\
         FUNC countM(hs AS Map OF String TO RES Handle) AS Integer\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR EACH e IN hs\n\
         \x20   MATCH e.value\n\
         \x20     CASE fs::File(f)\n\
         \x20       t = t + len(fs::readAll(f))\n\
         \x20     CASE tcp::Socket(s)\n\
         \x20       t = t + 100\n\
         \x20   END MATCH\n\
         \x20 NEXT\n\
         \x20 RETURN t\n\
         END FUNC\n\
         \n\
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
/// to the rule: the total printed, every allocation freed, nothing freed twice,
/// and no descriptor left open.
fn check(case: &str, body: &str, per_iteration: u64) {
    let nonce = common::unique_nonce();
    let dir = std::env::temp_dir().join(format!("mfb_member_float_{case}_{nonce}"));
    fs::create_dir_all(&dir).expect("create data dir");
    let data: PathBuf = dir.join("data.txt");
    fs::write(&data, "hello\n").expect("write data");
    let source = format!("{}{}", prelude(&data), body);
    let exe = common::debug_report::build_debug(&format!("member_float_{case}"), &source);
    for n in [10, 40] {
        let (live, skips, allocs, frees) = measure(case, &exe, n, per_iteration);
        assert_eq!(skips, 0, "{case} N={n}: a block was freed twice");
        assert_eq!(
            (allocs, live),
            (frees, 0),
            "{case} N={n}: {allocs} allocations, {frees} frees, {live} bytes live at exit"
        );
    }
    let limited = run(&exe, 300, true);
    assert!(
        limited.ok && limited.stdout.trim() == (300 * per_iteration).to_string(),
        "{case}: 300 iterations under a 64-descriptor limit failed:\n{}\n{}",
        limited.stdout,
        limited.stderr
    );
}

/// The reproduction as filed (`g1`): a loop-scoped `RES s AS fs::File` appended to a
/// `List OF RES Handle` declared before the loop. `countH` takes the `fs::File` arm
/// for each element and reads the file through it.
#[test]
fn a_member_floated_from_a_loop_into_a_list_is_tagged_closed_and_freed() {
    check(
        "loop_append",
        "FUNC once() AS Integer\n\
         \x20 MUT hs AS List OF RES Handle = []\n\
         \x20 FOR j = 1 TO 3\n\
         \x20   RES s AS fs::File = openOne()\n\
         \x20   hs = collections::append(hs, s)\n\
         \x20 NEXT\n\
         \x20 RETURN countH(hs)\n\
         END FUNC\n",
        18,
    );
    check(
        "loop_prepend_insert",
        "FUNC once() AS Integer\n\
         \x20 MUT hs AS List OF RES Handle = []\n\
         \x20 FOR j = 1 TO 3\n\
         \x20   RES s AS fs::File = openOne()\n\
         \x20   RES u AS fs::File = openOne()\n\
         \x20   hs = collections::prepend(hs, s)\n\
         \x20   hs = collections::insert(hs, 0, u)\n\
         \x20 NEXT\n\
         \x20 RETURN countH(hs)\n\
         END FUNC\n",
        36,
    );
    check(
        "loop_set",
        "FUNC once() AS Integer\n\
         \x20 MUT hs AS List OF RES Handle = []\n\
         \x20 RES first AS fs::File = openOne()\n\
         \x20 hs = collections::append(hs, first)\n\
         \x20 FOR j = 1 TO 3\n\
         \x20   RES s AS fs::File = openOne()\n\
         \x20   hs = collections::set(hs, 0, s)\n\
         \x20 NEXT\n\
         \x20 RETURN countH(hs)\n\
         END FUNC\n",
        6,
    );
    check(
        "if_and_while_scopes",
        "FUNC once() AS Integer\n\
         \x20 MUT hs AS List OF RES Handle = []\n\
         \x20 FOR j = 1 TO 3\n\
         \x20   IF j > 1 THEN\n\
         \x20     RES s AS fs::File = openOne()\n\
         \x20     hs = collections::append(hs, s)\n\
         \x20   END IF\n\
         \x20 NEXT\n\
         \x20 MUT k AS Integer = 0\n\
         \x20 WHILE k < 2\n\
         \x20   RES w AS fs::File = openOne()\n\
         \x20   hs = collections::append(hs, w)\n\
         \x20   k = k + 1\n\
         \x20 END WHILE\n\
         \x20 RETURN countH(hs)\n\
         END FUNC\n",
        24,
    );
}

/// The second reproduction (`i1`) with the result bound: the callee floats the
/// member into the list it returns, and the caller's binding adopts the list.
#[test]
fn a_member_floated_into_a_returned_list_is_adopted_by_the_caller() {
    check(
        "returned_one",
        "FUNC mk1() AS List OF RES Handle\n\
         \x20 MUT xs AS List OF RES Handle = []\n\
         \x20 RES f AS fs::File = openOne()\n\
         \x20 xs = collections::append(xs, f)\n\
         \x20 RETURN xs\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 LET hs AS List OF RES Handle = mk1()\n\
         \x20 RETURN countH(hs)\n\
         END FUNC\n",
        6,
    );
    check(
        "returned_loop",
        "FUNC mk3() AS List OF RES Handle\n\
         \x20 MUT xs AS List OF RES Handle = []\n\
         \x20 FOR j = 1 TO 3\n\
         \x20   RES f AS fs::File = openOne()\n\
         \x20   xs = collections::append(xs, f)\n\
         \x20 NEXT\n\
         \x20 RETURN xs\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 LET hs AS List OF RES Handle = mk3()\n\
         \x20 RETURN countH(hs)\n\
         END FUNC\n",
        18,
    );
}

/// The exits a float has to survive: an early `RETURN` out of the loop that floated
/// it, and a producer bound through an inline `TRAP` (whose delivery temporary must
/// not keep a close of its own).
#[test]
fn a_member_floated_before_an_early_return_or_through_an_inline_trap() {
    check(
        "early_return",
        "FUNC once() AS Integer\n\
         \x20 MUT hs AS List OF RES Handle = []\n\
         \x20 FOR j = 1 TO 3\n\
         \x20   RES s AS fs::File = openOne()\n\
         \x20   hs = collections::append(hs, s)\n\
         \x20   IF j = 2 THEN\n\
         \x20     RETURN countH(hs)\n\
         \x20   END IF\n\
         \x20 NEXT\n\
         \x20 RETURN 0\n\
         END FUNC\n",
        12,
    );
    check(
        "inline_trap",
        "FUNC once() AS Integer\n\
         \x20 MUT hs AS List OF RES Handle = []\n\
         \x20 FOR j = 1 TO 3\n\
         \x20   RES s AS fs::File = fs::openFile(\"/tmp/g933/data.txt\") TRAP(e)\n\
         \x20     RETURN -1\n\
         \x20   END TRAP\n\
         \x20   hs = collections::append(hs, s)\n\
         \x20 NEXT\n\
         \x20 RETURN countH(hs)\n\
         END FUNC\n",
        18,
    );
}

/// A `Map OF String TO RES Handle`: `set` in a loop, and a map a callee returns.
/// The reader is `MATCH collections::get(hs, k)`, which takes the `fs::File` arm.
#[test]
fn a_member_floated_into_a_map_is_tagged_closed_and_freed() {
    check(
        "map_loop_set",
        "FUNC readAt(hs AS Map OF String TO RES Handle, k AS String) AS Integer\n\
         \x20 MATCH collections::get(hs, k)\n\
         \x20   CASE fs::File(f)\n\
         \x20     RETURN len(fs::readAll(f))\n\
         \x20   CASE tcp::Socket(s)\n\
         \x20     RETURN 100\n\
         \x20 END MATCH\n\
         \x20 RETURN 0\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 MUT hs AS Map OF String TO RES Handle = Map OF String TO RES Handle {}\n\
         \x20 FOR j = 1 TO 3\n\
         \x20   RES s AS fs::File = openOne()\n\
         \x20   hs = collections::set(hs, toString(j), s)\n\
         \x20 NEXT\n\
         \x20 RETURN readAt(hs, \"1\") + readAt(hs, \"3\")\n\
         END FUNC\n",
        12,
    );
    check(
        "map_returned",
        "FUNC readAt(hs AS Map OF String TO RES Handle, k AS String) AS Integer\n\
         \x20 MATCH collections::get(hs, k)\n\
         \x20   CASE fs::File(f)\n\
         \x20     RETURN len(fs::readAll(f))\n\
         \x20   CASE tcp::Socket(s)\n\
         \x20     RETURN 100\n\
         \x20 END MATCH\n\
         \x20 RETURN 0\n\
         END FUNC\n\
         FUNC mkM() AS Map OF String TO RES Handle\n\
         \x20 MUT xs AS Map OF String TO RES Handle = Map OF String TO RES Handle {}\n\
         \x20 RES f AS fs::File = openOne()\n\
         \x20 xs = collections::set(xs, \"k\", f)\n\
         \x20 RETURN xs\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 LET hs AS Map OF String TO RES Handle = mkM()\n\
         \x20 RETURN readAt(hs, \"k\")\n\
         END FUNC\n",
        6,
    );
}

/// Two unions that list `fs::File` at different positions, each with a list a
/// member floats into: the tag is the member's, whichever union holds it.
#[test]
fn a_member_gets_its_own_tag_in_every_union_that_lists_it() {
    check(
        "two_unions",
        "UNION Rev\n\
         \x20 tcp::Socket\n\
         \x20 fs::File\n\
         END UNION\n\
         FUNC countR(hs AS List OF RES Rev) AS Integer\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR EACH h IN hs\n\
         \x20   MATCH h\n\
         \x20     CASE fs::File(f)\n\
         \x20       t = t + len(fs::readAll(f))\n\
         \x20     CASE tcp::Socket(s)\n\
         \x20       t = t + 100\n\
         \x20   END MATCH\n\
         \x20 NEXT\n\
         \x20 RETURN t\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 MUT hs AS List OF RES Rev = []\n\
         \x20 MUT gs AS List OF RES Handle = []\n\
         \x20 FOR j = 1 TO 2\n\
         \x20   RES s AS fs::File = openOne()\n\
         \x20   hs = collections::append(hs, s)\n\
         \x20   RES u AS fs::File = openOne()\n\
         \x20   gs = collections::append(gs, u)\n\
         \x20 NEXT\n\
         \x20 RETURN countR(hs) + countH(gs)\n\
         END FUNC\n",
        24,
    );
}

/// The same stores where nothing floats (member and container in one scope), which
/// were right before the fix and must stay so: `append`, `prepend`/`insert`, `set`
/// over an element, a list literal, a map literal, and a `RES Handle` record field.
#[test]
fn a_member_stored_in_the_scope_that_declares_it_stays_balanced() {
    check(
        "same_scope_append",
        "FUNC once() AS Integer\n\
         \x20 MUT hs AS List OF RES Handle = []\n\
         \x20 RES s AS fs::File = openOne()\n\
         \x20 hs = collections::append(hs, s)\n\
         \x20 RETURN countH(hs)\n\
         END FUNC\n",
        6,
    );
    check(
        "same_scope_prepend_insert",
        "FUNC once() AS Integer\n\
         \x20 MUT hs AS List OF RES Handle = []\n\
         \x20 RES s AS fs::File = openOne()\n\
         \x20 RES u AS fs::File = openOne()\n\
         \x20 hs = collections::prepend(hs, s)\n\
         \x20 hs = collections::insert(hs, 0, u)\n\
         \x20 RETURN countH(hs)\n\
         END FUNC\n",
        12,
    );
    check(
        "same_scope_set",
        "FUNC once() AS Integer\n\
         \x20 MUT hs AS List OF RES Handle = []\n\
         \x20 RES s AS fs::File = openOne()\n\
         \x20 RES u AS fs::File = openOne()\n\
         \x20 hs = collections::append(hs, s)\n\
         \x20 hs = collections::set(hs, 0, u)\n\
         \x20 RETURN countH(hs) + len(hs)\n\
         END FUNC\n",
        7,
    );
    check(
        "list_literal",
        "FUNC once() AS Integer\n\
         \x20 RES s AS fs::File = openOne()\n\
         \x20 RES u AS fs::File = openOne()\n\
         \x20 LET hs AS List OF RES Handle = [s, u]\n\
         \x20 RETURN countH(hs)\n\
         END FUNC\n",
        12,
    );
    check(
        "map_literal",
        "FUNC once() AS Integer\n\
         \x20 RES s AS fs::File = openOne()\n\
         \x20 LET hs AS Map OF String TO RES Handle = Map OF String TO RES Handle { \"a\" := s }\n\
         \x20 RETURN len(hs)\n\
         END FUNC\n",
        1,
    );
    check(
        "record_field",
        "FUNC rd(k AS HB) AS Integer\n\
         \x20 MATCH k.h\n\
         \x20   CASE fs::File(f)\n\
         \x20     RETURN len(fs::readAll(f)) + k.n\n\
         \x20   CASE tcp::Socket(s)\n\
         \x20     RETURN 100\n\
         \x20 END MATCH\n\
         \x20 RETURN 0\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 RES s AS fs::File = openOne()\n\
         \x20 LET k = HB[s, 4]\n\
         \x20 RETURN rd(k)\n\
         END FUNC\n",
        10,
    );
}
