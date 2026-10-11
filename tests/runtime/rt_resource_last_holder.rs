//! bug-1053: a resource record is freed when its last holder is gone.
//!
//! A drain used to free the records it closed only for a container proved, from the
//! function's text, to be their one holder; any other reading of the container (a
//! handle bound on its own, a copy that leaves, a `LAMBDA` capture, a callee that
//! keeps an element) left a close-only drain, which closed every descriptor and
//! kept every record. The owner ruling (2026-10-11) is that no leak is intentional:
//! the record is freed by the scope that owns it, once every cleanup of that scope
//! has run, and a collection's own block by the binding that is its one holder.
//!
//! Every case runs `once()` `N` times and reads the `--debug` report at two values
//! of `N`: the total printed proves each resource is readable where the program
//! reads it, every allocation is freed, nothing is freed twice, and 300 iterations
//! under a 64-descriptor limit prove each descriptor is closed.

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
         \x20 h AS RES fs::File\n\
         END TYPE\n\
         \n\
         TYPE HU\n\
         \x20 h AS RES Handle\n\
         END TYPE\n\
         \n\
         FUNC openOne AS RES fs::File\n\
         \x20 RETURN fs::openFile(\"{}\")\n\
         END FUNC\n\
         \n\
         FUNC wrap(h AS fs::File) AS HB\n\
         \x20 RETURN HB[h]\n\
         END FUNC\n\
         \n\
         FUNC rd(f AS fs::File) AS Integer\n\
         \x20 RETURN len(fs::readAll(f))\n\
         END FUNC\n\
         \n\
         FUNC which(h AS Handle) AS Integer\n\
         \x20 MATCH h\n\
         \x20   CASE fs::File(f)\n\
         \x20     RETURN len(fs::readAll(f))\n\
         \x20   CASE tcp::Socket(s)\n\
         \x20     RETURN 2\n\
         \x20 END MATCH\n\
         \x20 RETURN 0\n\
         END FUNC\n\
         \n\
         FUNC main AS Integer\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR i = 1 TO toInt(os::getEnvOr(\"N\", \"10\"))\n\
         \x20   t = t + once()\n\
         \x20 NEXT\n\
         \x20 io::print(toString(t))\n\
         \x20 RETURN 0\n\
         END FUNC\n\
",
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
    let dir = std::env::temp_dir().join(format!("mfb_last_holder_{case}_{nonce}"));
    fs::create_dir_all(&dir).expect("create data dir");
    let data: PathBuf = dir.join("data.txt");
    fs::write(&data, "hello\n").expect("write data");
    let source = format!("{}{}", prelude(&data), body);
    let exe = common::debug_report::build_debug(&format!("last_holder_{case}"), &source);
    for n in [10, 20] {
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

/// `r3` as filed: the handle bound on its own (`RES f = k.h`) beside the record.
#[test]
fn a_handle_bound_on_its_own_leaves_the_record_to_its_owner() {
    check(
        "r3",
        "FUNC mk() AS HB\n\
         \x20 RETURN HB[openOne()]\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 LET k AS HB = mk()\n\
         \x20 LET a AS Integer = rd(k.h)\n\
         \x20 RES f AS fs::File = k.h\n\
         \x20 LET k2 AS HB = k\n\
         \x20 RETURN a + rd(f) + rd(k2.h) + 1\n\
         END FUNC\n",
        7,
    );
}

/// `r4` as filed: the record returned after a copy, bound by the caller.
#[test]
fn a_record_returned_after_a_copy_is_freed_by_the_caller() {
    check(
        "r4",
        "FUNC mk() AS HB\n\
         \x20 RETURN HB[openOne()]\n\
         END FUNC\n\
         FUNC pass() AS HB\n\
         \x20 LET k AS HB = mk()\n\
         \x20 LET a AS Integer = rd(k.h)\n\
         \x20 LET k2 AS HB = k\n\
         \x20 IF a = 6 THEN\n\
         \x20   RETURN k2\n\
         \x20 END IF\n\
         \x20 RETURN k\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 LET q AS HB = pass()\n\
         \x20 RETURN rd(q.h) + 1\n\
         END FUNC\n",
        1,
    );
}

/// `r6` as filed: the record captured by a `LAMBDA` that only reads it.
#[test]
fn a_record_captured_by_a_reading_lambda_is_freed() {
    check(
        "r6",
        "FUNC mk() AS HB\n\
         \x20 RETURN HB[openOne()]\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 LET k AS HB = mk()\n\
         \x20 LET a AS Integer = rd(k.h)\n\
         \x20 LET g = LAMBDA() -> rd(k.h)\n\
         \x20 RETURN a + g() + 1\n\
         END FUNC\n",
        7,
    );
}

/// `r7` as filed: a `MATCH` case binding a callee stores into another list. The
/// records are the caller's bindings'; what was kept is the block of the list the
/// callee read.
#[test]
fn a_list_whose_element_a_callee_stores_elsewhere_frees_its_block() {
    check(
        "r7",
        "FUNC pick(hs AS List OF RES Handle) AS List OF RES fs::File\n\
         \x20 MUT out AS List OF RES fs::File = []\n\
         \x20 FOR EACH h IN hs\n\
         \x20   MATCH h\n\
         \x20     CASE fs::File(f)\n\
         \x20       out = collections::append(out, f)\n\
         \x20     CASE tcp::Socket(s)\n\
         \x20       io::print(\"s\")\n\
         \x20   END MATCH\n\
         \x20 NEXT\n\
         \x20 RETURN out\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 RES c AS fs::File = openOne()\n\
         \x20 LET hs AS List OF RES Handle = [a, c]\n\
         \x20 LET fl AS List OF RES fs::File = pick(hs)\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR EACH f IN fl\n\
         \x20   t = t + rd(f) + 1\n\
         \x20 NEXT\n\
         \x20 FOR EACH h IN hs\n\
         \x20   t = t + which(h)\n\
         \x20 NEXT\n\
         \x20 RETURN t\n\
         END FUNC\n",
        14,
    );
}

/// `r9` as filed: a copy of the record appended to a list, over a resource union.
#[test]
fn a_record_whose_copy_is_appended_to_a_list_is_freed() {
    check(
        "r9",
        "FUNC mkU() AS HU\n\
         \x20 RES u AS fs::File = openOne()\n\
         \x20 RETURN HU[u]\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 LET k AS HU = mkU()\n\
         \x20 LET k2 AS HU = k\n\
         \x20 MUT ks AS List OF HU = []\n\
         \x20 ks = collections::append(ks, k2)\n\
         \x20 RETURN which(k.h) + which(collections::get(ks, 0).h)\n\
         END FUNC\n",
        6,
    );
}
