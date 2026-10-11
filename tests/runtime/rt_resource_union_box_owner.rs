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
        0,
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
        0,
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
