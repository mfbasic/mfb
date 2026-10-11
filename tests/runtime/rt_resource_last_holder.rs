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
    check_built(case, body, per_iteration, None);
}

/// [`check`] at both the default optimization level and `-O0`. A copy the default
/// level folds away (`LET k3 = k`) is a real second binding at `-O0`, and the two
/// builds take different roads through the ownership analysis (bug-1052).
fn check_at_both_levels(case: &str, body: &str, per_iteration: u64) {
    check_built(case, body, per_iteration, None);
    check_built(&format!("{case}_o0"), body, per_iteration, Some("-O0"));
}

fn check_built(case: &str, body: &str, per_iteration: u64, level: Option<&str>) {
    let nonce = common::unique_nonce();
    let dir = std::env::temp_dir().join(format!("mfb_last_holder_{case}_{nonce}"));
    fs::create_dir_all(&dir).expect("create data dir");
    let data: PathBuf = dir.join("data.txt");
    fs::write(&data, "hello\n").expect("write data");
    let source = format!("{}{}", prelude(&data), body);
    let exe = match level {
        None => common::debug_report::build_debug(&format!("last_holder_{case}"), &source),
        Some(level) => {
            let name = format!("last_holder_{case}");
            let project = common::temp_project(&name, &source);
            let output = Command::new(common::mfb_exe())
                .arg("build")
                .arg("--debug")
                .arg(level)
                .arg(&project)
                .output()
                .expect("run mfb build --debug");
            assert!(
                output.status.success(),
                "{case} failed to build at {level}:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            project.join("build").join(format!("{name}.out"))
        }
    };
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

/// The filed shapes over a resource union and over a `Map`: a handle of a union
/// field bound on its own, a map whose value a callee stores in the list it
/// returns (the map's block is the binding's), a floated `List OF RES Handle` an
/// element of which is bound from `collections::get`, and a list literal around
/// two bindings with one element bound again.
#[test]
fn a_resource_union_and_a_map_follow_the_same_rule() {
    check(
        "union_field_handle",
        "FUNC mkU() AS HU\n\
         \x20 RES u AS fs::File = openOne()\n\
         \x20 RETURN HU[u]\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 LET k AS HU = mkU()\n\
         \x20 RES f AS Handle = k.h\n\
         \x20 LET k2 AS HU = k\n\
         \x20 RETURN which(f) + which(k2.h) + 1\n\
         END FUNC\n",
        7,
    );
    check(
        "map_value_kept_by_callee",
        "FUNC pickM(m AS Map OF String TO RES fs::File) AS List OF RES fs::File\n\
         \x20 MUT out AS List OF RES fs::File = []\n\
         \x20 FOR EACH e IN m\n\
         \x20   out = collections::append(out, e.value)\n\
         \x20 NEXT\n\
         \x20 RETURN out\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 RES c AS fs::File = openOne()\n\
         \x20 LET m AS Map OF String TO RES fs::File = Map OF String TO RES fs::File { \"a\" := a, \"c\" := c }\n\
         \x20 LET fl AS List OF RES fs::File = pickM(m)\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR EACH f IN fl\n\
         \x20   t = t + rd(f) + 1\n\
         \x20 NEXT\n\
         \x20 RETURN t + len(m)\n\
         END FUNC\n",
        16,
    );
    check(
        "union_list_get",
        "FUNC once() AS Integer\n\
         \x20 MUT xs AS List OF RES Handle = []\n\
         \x20 FOR j = 1 TO 2\n\
         \x20   RES a AS Handle = openOne()\n\
         \x20   xs = collections::append(xs, a)\n\
         \x20 NEXT\n\
         \x20 RES g AS Handle = collections::get(xs, 0)\n\
         \x20 MUT t AS Integer = which(g)\n\
         \x20 FOR EACH h IN xs\n\
         \x20   t = t + which(h) + 1\n\
         \x20 NEXT\n\
         \x20 RETURN t\n\
         END FUNC\n",
        14,
    );
    check(
        "union_literal_get",
        "FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 RES c AS fs::File = openOne()\n\
         \x20 LET hs AS List OF RES Handle = [a, c]\n\
         \x20 RES s AS Handle = collections::get(hs, 1)\n\
         \x20 MUT t AS Integer = which(s)\n\
         \x20 FOR EACH h IN hs\n\
         \x20   t = t + which(h) + 1\n\
         \x20 NEXT\n\
         \x20 RETURN t\n\
         END FUNC\n",
        14,
    );
}

/// Each iteration of a loop body frees what it took: at the end of the body, at a
/// `CONTINUE`, at an `EXIT` and at a `RETURN` out of the loop.
#[test]
fn a_loop_body_frees_on_every_way_out() {
    check(
        "loop_body",
        "FUNC mk() AS HB\n\
         \x20 RETURN HB[openOne()]\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR j = 1 TO 3\n\
         \x20   LET k AS HB = mk()\n\
         \x20   RES f AS fs::File = k.h\n\
         \x20   LET g = LAMBDA() -> rd(k.h)\n\
         \x20   t = t + rd(f) + g() + 1\n\
         \x20 NEXT\n\
         \x20 RETURN t\n\
         END FUNC\n",
        21,
    );
    check(
        "loop_return_continue",
        "FUNC mk() AS HB\n\
         \x20 RETURN HB[openOne()]\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR j = 1 TO 3\n\
         \x20   LET k AS HB = mk()\n\
         \x20   RES f AS fs::File = k.h\n\
         \x20   t = t + rd(f) + 1\n\
         \x20   IF j = 2 THEN\n\
         \x20     RETURN t\n\
         \x20   END IF\n\
         \x20   IF j = 1 THEN\n\
         \x20     CONTINUE FOR\n\
         \x20   END IF\n\
         \x20   t = t + 100\n\
         \x20 NEXT\n\
         \x20 RETURN t\n\
         END FUNC\n",
        14,
    );
    check(
        "while_exit_continue",
        "FUNC mk() AS HB\n\
         \x20 RETURN HB[openOne()]\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 MUT j AS Integer = 0\n\
         \x20 WHILE j < 5\n\
         \x20   j = j + 1\n\
         \x20   LET k AS HB = mk()\n\
         \x20   RES f AS fs::File = k.h\n\
         \x20   IF j = 2 THEN\n\
         \x20     CONTINUE WHILE\n\
         \x20   END IF\n\
         \x20   t = t + rd(f) + 1\n\
         \x20   IF j = 4 THEN\n\
         \x20     EXIT WHILE\n\
         \x20   END IF\n\
         \x20 END WHILE\n\
         \x20 RETURN t\n\
         END FUNC\n",
        21,
    );
}

/// An error leaves by other roads than the end of a scope: an inline `TRAP` in the
/// scope, an error return through the landing pads (the caller recovers), a
/// function-level `TRAP` reached from the function's own scope (the handler still
/// reads the container, so the route closes in place and the handler's exit
/// frees), and one reached from a loop body.
#[test]
fn an_error_exit_frees_what_the_scope_took() {
    check(
        "inline_trap",
        "FUNC mk() AS HB\n\
         \x20 RETURN HB[openOne()]\n\
         END FUNC\n\
         FUNC rdT(f AS fs::File, n AS Integer) AS Integer\n\
         \x20 IF n > 1 THEN\n\
         \x20   FAIL error(77, \"boom\")\n\
         \x20 END IF\n\
         \x20 RETURN len(fs::readAll(f))\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR j = 1 TO 3\n\
         \x20   LET k AS HB = mk()\n\
         \x20   RES f AS fs::File = k.h\n\
         \x20   LET n AS Integer = rdT(f, j) TRAP(e)\n\
         \x20     RECOVER 50\n\
         \x20   END TRAP\n\
         \x20   t = t + n + rdT(k.h, 0) TRAP(e)\n\
         \x20     RECOVER 1000\n\
         \x20   END TRAP\n\
         \x20 NEXT\n\
         \x20 RETURN t\n\
         END FUNC\n",
        118,
    );
    check(
        "error_return",
        "FUNC mk() AS HB\n\
         \x20 RETURN HB[openOne()]\n\
         END FUNC\n\
         FUNC boom(n AS Integer) AS Integer\n\
         \x20 IF n > 0 THEN\n\
         \x20   FAIL error(77, \"boom\")\n\
         \x20 END IF\n\
         \x20 RETURN 1\n\
         END FUNC\n\
         FUNC inner() AS Integer\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR j = 1 TO 2\n\
         \x20   LET k AS HB = mk()\n\
         \x20   RES f AS fs::File = k.h\n\
         \x20   t = t + rd(f)\n\
         \x20   t = t + boom(t)\n\
         \x20 NEXT\n\
         \x20 RETURN t\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 LET r AS Integer = inner() TRAP(e)\n\
         \x20   RECOVER 5\n\
         \x20 END TRAP\n\
         \x20 RETURN r\n\
         END FUNC\n",
        5,
    );
    check(
        "function_trap",
        "FUNC boom(n AS Integer) AS Integer\n\
         \x20 IF n > 0 THEN\n\
         \x20   FAIL error(77, \"boom\")\n\
         \x20 END IF\n\
         \x20 RETURN 1\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 MUT xs AS List OF RES fs::File = []\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 IF len(\"x\") = 1 THEN\n\
         \x20   RES a AS fs::File = openOne()\n\
         \x20   xs = collections::append(xs, a)\n\
         \x20 END IF\n\
         \x20 IF len(\"x\") = 1 THEN\n\
         \x20   RES g AS fs::File = collections::get(xs, 0)\n\
         \x20   t = t + len(fs::readLine(g))\n\
         \x20 END IF\n\
         \x20 t = t + boom(len(xs))\n\
         \x20 RETURN t\n\
         \x20 TRAP(e)\n\
         \x20   RETURN t + len(xs) + 100\n\
         \x20 END TRAP\n\
         END FUNC\n",
        106,
    );
    check(
        "function_trap_from_loop",
        "FUNC mk() AS HB\n\
         \x20 RETURN HB[openOne()]\n\
         END FUNC\n\
         FUNC boom(n AS Integer) AS Integer\n\
         \x20 IF n > 0 THEN\n\
         \x20   FAIL error(77, \"boom\")\n\
         \x20 END IF\n\
         \x20 RETURN 1\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR j = 1 TO 2\n\
         \x20   LET k AS HB = mk()\n\
         \x20   RES f AS fs::File = k.h\n\
         \x20   t = t + rd(f)\n\
         \x20   IF j = 2 THEN\n\
         \x20     t = t + boom(t)\n\
         \x20   END IF\n\
         \x20 NEXT\n\
         \x20 RETURN t\n\
         \x20 TRAP(e)\n\
         \x20   RETURN t + 100\n\
         \x20 END TRAP\n\
         END FUNC\n",
        112,
    );
}

/// Two cleanups of one scope that name the same record: the record's own
/// container and a list its handle floated into (declared before the record, and
/// after it), and a chain of calls that hand the same handle back. The first
/// drain to reach the record closes and takes it; every other one reads an owner
/// word that is no longer its token and leaves it alone.
#[test]
fn two_cleanups_that_name_one_record_free_it_once() {
    check(
        "list_then_record",
        "FUNC mk() AS HB\n\
         \x20 RETURN HB[openOne()]\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 MUT xs AS List OF RES fs::File = []\n\
         \x20 LET k AS HB = mk()\n\
         \x20 RES f AS fs::File = k.h\n\
         \x20 xs = collections::append(xs, f)\n\
         \x20 MUT t AS Integer = rd(k.h)\n\
         \x20 FOR EACH h IN xs\n\
         \x20   t = t + rd(h) + 1\n\
         \x20 NEXT\n\
         \x20 RETURN t\n\
         END FUNC\n",
        7,
    );
    check(
        "record_then_list",
        "FUNC mk() AS HB\n\
         \x20 RETURN HB[openOne()]\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 LET k AS HB = mk()\n\
         \x20 MUT xs AS List OF RES fs::File = []\n\
         \x20 RES f AS fs::File = k.h\n\
         \x20 xs = collections::append(xs, f)\n\
         \x20 MUT t AS Integer = rd(k.h)\n\
         \x20 FOR EACH h IN xs\n\
         \x20   t = t + rd(h) + 1\n\
         \x20 NEXT\n\
         \x20 RETURN t\n\
         END FUNC\n",
        7,
    );
    check(
        "handed_back",
        "FUNC mk() AS HB\n\
         \x20 RETURN HB[openOne()]\n\
         END FUNC\n\
         FUNC ident(f AS fs::File) AS RES fs::File\n\
         \x20 RETURN f\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 LET k AS HB = mk()\n\
         \x20 RES b AS fs::File = ident(k.h)\n\
         \x20 LET k2 AS HB = wrap(b)\n\
         \x20 LET k3 AS HB = wrap(k.h)\n\
         \x20 RETURN rd(b) + rd(k2.h) + rd(k3.h) + rd(k.h) + 1\n\
         END FUNC\n",
        7,
    );
}

/// A floated list an element of which is bound from `collections::get`, the same
/// list returned to a caller that reads it the same way, and a record a callee
/// copies into the list it returns.
#[test]
fn a_list_read_through_get_or_copied_by_a_callee_is_freed() {
    check(
        "float_get",
        "FUNC once() AS Integer\n\
         \x20 MUT xs AS List OF RES fs::File = []\n\
         \x20 FOR j = 1 TO 2\n\
         \x20   RES a AS fs::File = openOne()\n\
         \x20   xs = collections::append(xs, a)\n\
         \x20 NEXT\n\
         \x20 RES g AS fs::File = collections::get(xs, 0)\n\
         \x20 MUT t AS Integer = rd(g)\n\
         \x20 FOR EACH h IN xs\n\
         \x20   t = t + rd(h) + 1\n\
         \x20 NEXT\n\
         \x20 RETURN t\n\
         END FUNC\n",
        14,
    );
    check(
        "returned_get",
        "FUNC build() AS List OF RES fs::File\n\
         \x20 MUT xs AS List OF RES fs::File = []\n\
         \x20 FOR j = 1 TO 2\n\
         \x20   RES a AS fs::File = openOne()\n\
         \x20   xs = collections::append(xs, a)\n\
         \x20 NEXT\n\
         \x20 RES g AS fs::File = collections::get(xs, 0)\n\
         \x20 LET n AS Integer = rd(g)\n\
         \x20 RETURN xs\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 LET ys AS List OF RES fs::File = build()\n\
         \x20 RES g AS fs::File = collections::get(ys, 1)\n\
         \x20 MUT t AS Integer = rd(g)\n\
         \x20 FOR EACH h IN ys\n\
         \x20   t = t + rd(h) + 1\n\
         \x20 NEXT\n\
         \x20 RETURN t\n\
         END FUNC\n",
        8,
    );
    check(
        "callee_copies",
        "FUNC mk() AS HB\n\
         \x20 RETURN HB[openOne()]\n\
         END FUNC\n\
         FUNC keepIt(k AS HB) AS List OF HB\n\
         \x20 MUT out AS List OF HB = []\n\
         \x20 out = collections::append(out, k)\n\
         \x20 RETURN out\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 LET k AS HB = mk()\n\
         \x20 LET ks AS List OF HB = keepIt(k)\n\
         \x20 MUT t AS Integer = rd(k.h)\n\
         \x20 FOR EACH x IN ks\n\
         \x20   t = t + rd(x.h) + 1\n\
         \x20 NEXT\n\
         \x20 RETURN t\n\
         END FUNC\n",
        7,
    );
}

/// bug-1052: a callee that copies a record holding a resource and returns the copy
/// (`LET k3 AS HB2 = k` / `RETURN k3`). At `-O0` the copy is a binding of its own,
/// so the returning function's record is named outside an argument position and the
/// caller's binding is not bound from a provably fresh block: the caller's drain
/// closed the file and kept its 112-byte record on every call. The same over a
/// `WITH` copy of an enclosing record, a chain of copies on both sides of the call,
/// a copy returned out of a loop body and after it, a resource-union field, and a
/// callee that fails on some calls under an inline `TRAP`.
#[test]
fn a_copy_of_a_record_returned_by_a_callee_is_freed_at_every_level() {
    check_at_both_levels(
        "unfolded_copy",
        "TYPE HB2\n\
         \x20 h AS RES fs::File\n\
         END TYPE\n\
         TYPE Outer2\n\
         \x20 n AS Integer\n\
         \x20 inner AS HB2\n\
         END TYPE\n\
         FUNC keep2(h AS fs::File) AS HB2\n\
         \x20 RETURN HB2[h]\n\
         END FUNC\n\
         FUNC mk2 AS HB2\n\
         \x20 RES u AS fs::File = openOne()\n\
         \x20 RETURN keep2(u)\n\
         END FUNC\n\
         \n\
         FUNC mid AS HB2\n\
         \x20 LET k AS HB2 = mk2()\n\
         \x20 LET k3 AS HB2 = k\n\
         \x20 RETURN k3\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 LET q AS HB2 = mid()\n\
         \x20 LET junk1 AS List OF Integer = [7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7]\n\
         \x20 RETURN rd(q.h) - 5\n\
         END FUNC\n",
        1,
    );
    check_at_both_levels(
        "with_copy",
        "TYPE HB2\n\
         \x20 h AS RES fs::File\n\
         END TYPE\n\
         TYPE Outer2\n\
         \x20 n AS Integer\n\
         \x20 inner AS HB2\n\
         END TYPE\n\
         FUNC keep2(h AS fs::File) AS HB2\n\
         \x20 RETURN HB2[h]\n\
         END FUNC\n\
         FUNC mk2 AS HB2\n\
         \x20 RES u AS fs::File = openOne()\n\
         \x20 RETURN keep2(u)\n\
         END FUNC\n\
         \n\
         FUNC mid AS Outer2\n\
         \x20 LET k AS HB2 = mk2()\n\
         \x20 LET o AS Outer2 = Outer2[1, k]\n\
         \x20 LET o2 AS Outer2 = WITH o { n := 2 }\n\
         \x20 RETURN o2\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 LET q AS Outer2 = mid()\n\
         \x20 RETURN rd(q.inner.h) + q.n\n\
         END FUNC\n",
        8,
    );
    check_at_both_levels(
        "copy_chain",
        "TYPE HB2\n\
         \x20 h AS RES fs::File\n\
         END TYPE\n\
         TYPE Outer2\n\
         \x20 n AS Integer\n\
         \x20 inner AS HB2\n\
         END TYPE\n\
         FUNC keep2(h AS fs::File) AS HB2\n\
         \x20 RETURN HB2[h]\n\
         END FUNC\n\
         FUNC mk2 AS HB2\n\
         \x20 RES u AS fs::File = openOne()\n\
         \x20 RETURN keep2(u)\n\
         END FUNC\n\
         \n\
         FUNC mid AS HB2\n\
         \x20 LET k AS HB2 = mk2()\n\
         \x20 LET a AS HB2 = k\n\
         \x20 LET b AS HB2 = a\n\
         \x20 RETURN b\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 LET q AS HB2 = mid()\n\
         \x20 LET q2 AS HB2 = q\n\
         \x20 RETURN rd(q2.h) + rd(q.h) + 1\n\
         END FUNC\n",
        7,
    );
    check_at_both_levels(
        "copy_from_loop",
        "TYPE HB2\n\
         \x20 h AS RES fs::File\n\
         END TYPE\n\
         TYPE Outer2\n\
         \x20 n AS Integer\n\
         \x20 inner AS HB2\n\
         END TYPE\n\
         FUNC keep2(h AS fs::File) AS HB2\n\
         \x20 RETURN HB2[h]\n\
         END FUNC\n\
         FUNC mk2 AS HB2\n\
         \x20 RES u AS fs::File = openOne()\n\
         \x20 RETURN keep2(u)\n\
         END FUNC\n\
         \n\
         FUNC mid(n AS Integer) AS HB2\n\
         \x20 FOR j = 1 TO 3\n\
         \x20   LET k AS HB2 = mk2()\n\
         \x20   LET k3 AS HB2 = k\n\
         \x20   IF j = n THEN\n\
         \x20     RETURN k3\n\
         \x20   END IF\n\
         \x20 NEXT\n\
         \x20 LET z AS HB2 = mk2()\n\
         \x20 LET z2 AS HB2 = z\n\
         \x20 RETURN z2\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 LET q AS HB2 = mid(2)\n\
         \x20 LET r AS HB2 = mid(9)\n\
         \x20 RETURN rd(q.h) + rd(r.h) + 1\n\
         END FUNC\n",
        13,
    );
    check_at_both_levels(
        "union_field_copy",
        "FUNC mkU() AS HU\n\
         \x20 RES u AS fs::File = openOne()\n\
         \x20 RETURN HU[u]\n\
         END FUNC\n\
         FUNC mid AS HU\n\
         \x20 LET k AS HU = mkU()\n\
         \x20 LET k3 AS HU = k\n\
         \x20 RETURN k3\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 LET q AS HU = mid()\n\
         \x20 RETURN which(q.h) + 1\n\
         END FUNC\n",
        7,
    );
    check_at_both_levels(
        "copy_under_inline_trap",
        "TYPE HB2\n\
         \x20 h AS RES fs::File\n\
         END TYPE\n\
         TYPE Outer2\n\
         \x20 n AS Integer\n\
         \x20 inner AS HB2\n\
         END TYPE\n\
         FUNC keep2(h AS fs::File) AS HB2\n\
         \x20 RETURN HB2[h]\n\
         END FUNC\n\
         FUNC mk2 AS HB2\n\
         \x20 RES u AS fs::File = openOne()\n\
         \x20 RETURN keep2(u)\n\
         END FUNC\n\
         \n\
         FUNC mid(n AS Integer) AS HB2\n\
         \x20 LET k AS HB2 = mk2()\n\
         \x20 LET k3 AS HB2 = k\n\
         \x20 IF n > 1 THEN\n\
         \x20   FAIL error(77, \"boom\")\n\
         \x20 END IF\n\
         \x20 RETURN k3\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR j = 1 TO 2\n\
         \x20   LET q AS HB2 = mid(j) TRAP(e)\n\
         \x20     t = t + 100\n\
         \x20     CONTINUE FOR\n\
         \x20   END TRAP\n\
         \x20   t = t + rd(q.h) + 1\n\
         \x20 NEXT\n\
         \x20 RETURN t\n\
         END FUNC\n",
        107,
    );
}

/// bug-1051: `RES v AS fs::File = sameF(a) TRAP(e) … END TRAP`, where `sameF`
/// returns its argument. The desugar's temp is bound to a closed default record of
/// its own and the call's value is stored over it. The temp is not proved to own
/// what `sameF` hands back (it does not: the record is `a`'s), so its drop frees no
/// record, and that included the default: 112 bytes on every evaluation, on the
/// success path (the store's drop of the old value) and when the handler leaves
/// without storing (the scope's drop). The same for a resource union, whose default
/// is a box and a record; with the handler taken (`CONTINUE`, and `RECOVER` of
/// another binding); and with the argument read through a record field or
/// `collections::get`, beside a control whose producer the temp does own.
#[test]
fn a_trapped_call_that_returns_its_argument_frees_the_default_record() {
    check_at_both_levels(
        "same_file",
        "FUNC sameF(h AS fs::File) AS RES fs::File\n\
         \x20 RETURN h\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 RES v AS fs::File = sameF(a) TRAP(e)\n\
         \x20   RETURN 5\n\
         \x20 END TRAP\n\
         \x20 RETURN len(fs::readAll(v)) - 5\n\
         END FUNC\n",
        1,
    );
    check_at_both_levels(
        "same_union",
        "FUNC same(h AS Handle) AS RES Handle\n\
         \x20 RETURN h\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 RES a AS Handle = openOne()\n\
         \x20 RES v AS Handle = same(a) TRAP(e)\n\
         \x20   RETURN 5\n\
         \x20 END TRAP\n\
         \x20 RETURN which(v) - 5\n\
         END FUNC\n",
        1,
    );
    check_at_both_levels(
        "handler_continues",
        "FUNC sameF(h AS fs::File, n AS Integer) AS RES fs::File\n\
         \x20 IF n > 1 THEN\n\
         \x20   FAIL error(77, \"boom\")\n\
         \x20 END IF\n\
         \x20 RETURN h\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR j = 1 TO 2\n\
         \x20   RES v AS fs::File = sameF(a, j) TRAP(e)\n\
         \x20     t = t + 100\n\
         \x20     CONTINUE FOR\n\
         \x20   END TRAP\n\
         \x20   t = t + len(fs::readAll(v)) + 1\n\
         \x20 NEXT\n\
         \x20 RETURN t\n\
         END FUNC\n",
        107,
    );
    check_at_both_levels(
        "recover_alias",
        "FUNC sameF(h AS fs::File, n AS Integer) AS RES fs::File\n\
         \x20 IF n > 1 THEN\n\
         \x20   FAIL error(77, \"boom\")\n\
         \x20 END IF\n\
         \x20 RETURN h\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 RES b AS fs::File = openOne()\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR j = 1 TO 2\n\
         \x20   RES v AS fs::File = sameF(a, j) TRAP(e)\n\
         \x20     RECOVER b\n\
         \x20   END TRAP\n\
         \x20   t = t + len(fs::readAll(v)) + 1\n\
         \x20 NEXT\n\
         \x20 RETURN t\n\
         END FUNC\n",
        14,
    );
    check_at_both_levels(
        "union_recover_and_continue",
        "FUNC same(h AS Handle, n AS Integer) AS RES Handle\n\
         \x20 IF n > 1 THEN\n\
         \x20   FAIL error(77, \"boom\")\n\
         \x20 END IF\n\
         \x20 RETURN h\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 RES a AS Handle = openOne()\n\
         \x20 RES b AS Handle = openOne()\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR j = 1 TO 2\n\
         \x20   RES v AS Handle = same(a, j) TRAP(e)\n\
         \x20     RECOVER b\n\
         \x20   END TRAP\n\
         \x20   t = t + which(v) + 1\n\
         \x20 NEXT\n\
         \x20 FOR j = 1 TO 2\n\
         \x20   RES w AS Handle = same(a, j) TRAP(e)\n\
         \x20     t = t + 100\n\
         \x20     CONTINUE FOR\n\
         \x20   END TRAP\n\
         \x20   t = t + which(w) + 1\n\
         \x20 NEXT\n\
         \x20 RETURN t\n\
         END FUNC\n",
        115,
    );
    check_at_both_levels(
        "field_get_and_producer",
        "FUNC mk() AS HB\n\
         \x20 RETURN HB[openOne()]\n\
         END FUNC\n\
         FUNC sameF(h AS fs::File) AS RES fs::File\n\
         \x20 RETURN h\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 LET k AS HB = mk()\n\
         \x20 RES v AS fs::File = sameF(k.h) TRAP(e)\n\
         \x20   RETURN 5\n\
         \x20 END TRAP\n\
         \x20 MUT xs AS List OF RES fs::File = []\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 xs = collections::append(xs, a)\n\
         \x20 RES g AS fs::File = sameF(collections::get(xs, 0)) TRAP(e)\n\
         \x20   RETURN 6\n\
         \x20 END TRAP\n\
         \x20 RES p AS fs::File = openOne() TRAP(e)\n\
         \x20   RETURN 7\n\
         \x20 END TRAP\n\
         \x20 RETURN rd(v) + rd(g) + rd(p) + 1\n\
         END FUNC\n",
        19,
    );
}

/// bug-1047: `whichL("t", collections::get(hs, i)) TRAP(e) … END TRAP` over a `List
/// OF RES fs::File`. The inline `TRAP` lifts the fallible `get` out of the argument
/// into a temporary of its own (`bind $trap_resN = callResult collections.get(hs,
/// i)`), so the element read is no longer written straight into an argument: the
/// list counted as read back out of, lost the proof, and with it the free of its
/// own block. Either trapped read alone keeps the block (96 bytes for one element,
/// 192 for three), the failing one and the succeeding one alike. The same over a
/// resource union, a `Map`, a list floated in a loop and read in a loop with an
/// early `RETURN`, and a `RES` binding of the trapped read itself.
#[test]
fn a_trapped_element_read_does_not_keep_the_list_block() {
    check_at_both_levels(
        "two_reads",
        "FUNC whichL(label AS String, f AS fs::File) AS Integer\n\
         \x20 LET text AS String = fs::readAll(f)\n\
         \x20 RETURN 1\n\
         END FUNC\n\
         FUNC whichLH(label AS String, h AS Handle) AS Integer\n\
         \x20 MATCH h\n\
         \x20   CASE fs::File(f)\n\
         \x20     LET text AS String = fs::readAll(f)\n\
         \x20     RETURN 1\n\
         \x20   CASE tcp::Socket(s)\n\
         \x20     RETURN 2\n\
         \x20 END MATCH\n\
         \x20 RETURN 0\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 LET hs AS List OF RES fs::File = [a]\n\
         \x20 MUT r AS Integer = 0\n\
         \x20 r = whichL(\"t\", collections::get(hs, 5)) TRAP(e)\n\
         \x20   RECOVER 7\n\
         \x20 END TRAP\n\
         \x20 MUT q AS Integer = 0\n\
         \x20 q = whichL(\"t\", collections::get(hs, 0)) TRAP(e)\n\
         \x20   RECOVER 7\n\
         \x20 END TRAP\n\
         \x20 RETURN r + q - 7\n\
         END FUNC\n",
        1,
    );
    check_at_both_levels(
        "failing_read",
        "FUNC whichL(label AS String, f AS fs::File) AS Integer\n\
         \x20 LET text AS String = fs::readAll(f)\n\
         \x20 RETURN 1\n\
         END FUNC\n\
         FUNC whichLH(label AS String, h AS Handle) AS Integer\n\
         \x20 MATCH h\n\
         \x20   CASE fs::File(f)\n\
         \x20     LET text AS String = fs::readAll(f)\n\
         \x20     RETURN 1\n\
         \x20   CASE tcp::Socket(s)\n\
         \x20     RETURN 2\n\
         \x20 END MATCH\n\
         \x20 RETURN 0\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 LET hs AS List OF RES fs::File = [a]\n\
         \x20 MUT r AS Integer = 0\n\
         \x20 r = whichL(\"t\", collections::get(hs, 5)) TRAP(e)\n\
         \x20   RECOVER 7\n\
         \x20 END TRAP\n\
         \x20 RETURN r - 6\n\
         END FUNC\n",
        1,
    );
    check_at_both_levels(
        "succeeding_read",
        "FUNC whichL(label AS String, f AS fs::File) AS Integer\n\
         \x20 LET text AS String = fs::readAll(f)\n\
         \x20 RETURN 1\n\
         END FUNC\n\
         FUNC whichLH(label AS String, h AS Handle) AS Integer\n\
         \x20 MATCH h\n\
         \x20   CASE fs::File(f)\n\
         \x20     LET text AS String = fs::readAll(f)\n\
         \x20     RETURN 1\n\
         \x20   CASE tcp::Socket(s)\n\
         \x20     RETURN 2\n\
         \x20 END MATCH\n\
         \x20 RETURN 0\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 LET hs AS List OF RES fs::File = [a]\n\
         \x20 MUT q AS Integer = 0\n\
         \x20 q = whichL(\"t\", collections::get(hs, 0)) TRAP(e)\n\
         \x20   RECOVER 7\n\
         \x20 END TRAP\n\
         \x20 RETURN q\n\
         END FUNC\n",
        1,
    );
    check_at_both_levels(
        "three_elements",
        "FUNC whichL(label AS String, f AS fs::File) AS Integer\n\
         \x20 LET text AS String = fs::readAll(f)\n\
         \x20 RETURN 1\n\
         END FUNC\n\
         FUNC whichLH(label AS String, h AS Handle) AS Integer\n\
         \x20 MATCH h\n\
         \x20   CASE fs::File(f)\n\
         \x20     LET text AS String = fs::readAll(f)\n\
         \x20     RETURN 1\n\
         \x20   CASE tcp::Socket(s)\n\
         \x20     RETURN 2\n\
         \x20 END MATCH\n\
         \x20 RETURN 0\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 RES b AS fs::File = openOne()\n\
         \x20 RES c AS fs::File = openOne()\n\
         \x20 LET hs AS List OF RES fs::File = [a, b, c]\n\
         \x20 MUT q AS Integer = 0\n\
         \x20 q = whichL(\"t\", collections::get(hs, 0)) TRAP(e)\n\
         \x20   RECOVER 7\n\
         \x20 END TRAP\n\
         \x20 RETURN q\n\
         END FUNC\n",
        1,
    );
    check_at_both_levels(
        "union_list",
        "FUNC whichL(label AS String, f AS fs::File) AS Integer\n\
         \x20 LET text AS String = fs::readAll(f)\n\
         \x20 RETURN 1\n\
         END FUNC\n\
         FUNC whichLH(label AS String, h AS Handle) AS Integer\n\
         \x20 MATCH h\n\
         \x20   CASE fs::File(f)\n\
         \x20     LET text AS String = fs::readAll(f)\n\
         \x20     RETURN 1\n\
         \x20   CASE tcp::Socket(s)\n\
         \x20     RETURN 2\n\
         \x20 END MATCH\n\
         \x20 RETURN 0\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 RES a AS Handle = openOne()\n\
         \x20 LET hs AS List OF RES Handle = [a]\n\
         \x20 MUT r AS Integer = 0\n\
         \x20 r = whichLH(\"t\", collections::get(hs, 5)) TRAP(e)\n\
         \x20   RECOVER 7\n\
         \x20 END TRAP\n\
         \x20 MUT q AS Integer = 0\n\
         \x20 q = whichLH(\"t\", collections::get(hs, 0)) TRAP(e)\n\
         \x20   RECOVER 7\n\
         \x20 END TRAP\n\
         \x20 RETURN r + q - 7\n\
         END FUNC\n",
        1,
    );
    check_at_both_levels(
        "map",
        "FUNC whichL(label AS String, f AS fs::File) AS Integer\n\
         \x20 LET text AS String = fs::readAll(f)\n\
         \x20 RETURN 1\n\
         END FUNC\n\
         FUNC whichLH(label AS String, h AS Handle) AS Integer\n\
         \x20 MATCH h\n\
         \x20   CASE fs::File(f)\n\
         \x20     LET text AS String = fs::readAll(f)\n\
         \x20     RETURN 1\n\
         \x20   CASE tcp::Socket(s)\n\
         \x20     RETURN 2\n\
         \x20 END MATCH\n\
         \x20 RETURN 0\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 LET m AS Map OF String TO RES fs::File = Map OF String TO RES fs::File { \"a\" := a }\n\
         \x20 MUT r AS Integer = 0\n\
         \x20 r = whichL(\"t\", collections::get(m, \"zz\")) TRAP(e)\n\
         \x20   RECOVER 7\n\
         \x20 END TRAP\n\
         \x20 MUT q AS Integer = 0\n\
         \x20 q = whichL(\"t\", collections::get(m, \"a\")) TRAP(e)\n\
         \x20   RECOVER 7\n\
         \x20 END TRAP\n\
         \x20 RETURN r + q - 7\n\
         END FUNC\n",
        1,
    );
    check_at_both_levels(
        "floated_loop_return",
        "FUNC whichL(label AS String, f AS fs::File) AS Integer\n\
         \x20 LET text AS String = fs::readAll(f)\n\
         \x20 RETURN 1\n\
         END FUNC\n\
         FUNC whichLH(label AS String, h AS Handle) AS Integer\n\
         \x20 MATCH h\n\
         \x20   CASE fs::File(f)\n\
         \x20     LET text AS String = fs::readAll(f)\n\
         \x20     RETURN 1\n\
         \x20   CASE tcp::Socket(s)\n\
         \x20     RETURN 2\n\
         \x20 END MATCH\n\
         \x20 RETURN 0\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 MUT hs AS List OF RES fs::File = []\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR j = 1 TO 2\n\
         \x20   RES a AS fs::File = openOne()\n\
         \x20   hs = collections::append(hs, a)\n\
         \x20 NEXT\n\
         \x20 FOR j = 0 TO 2\n\
         \x20   MUT q AS Integer = 0\n\
         \x20   q = whichL(\"t\", collections::get(hs, j)) TRAP(e)\n\
         \x20     RECOVER 7\n\
         \x20   END TRAP\n\
         \x20   t = t + q\n\
         \x20   IF j = 2 THEN\n\
         \x20     RETURN t - 8\n\
         \x20   END IF\n\
         \x20 NEXT\n\
         \x20 RETURN 0\n\
         END FUNC\n",
        1,
    );
    check_at_both_levels(
        "bound_trapped_read",
        "FUNC whichL(label AS String, f AS fs::File) AS Integer\n\
         \x20 LET text AS String = fs::readAll(f)\n\
         \x20 RETURN 1\n\
         END FUNC\n\
         FUNC whichLH(label AS String, h AS Handle) AS Integer\n\
         \x20 MATCH h\n\
         \x20   CASE fs::File(f)\n\
         \x20     LET text AS String = fs::readAll(f)\n\
         \x20     RETURN 1\n\
         \x20   CASE tcp::Socket(s)\n\
         \x20     RETURN 2\n\
         \x20 END MATCH\n\
         \x20 RETURN 0\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 LET hs AS List OF RES fs::File = [a]\n\
         \x20 RES g AS fs::File = collections::get(hs, 0) TRAP(e)\n\
         \x20   RETURN 9\n\
         \x20 END TRAP\n\
         \x20 RES g2 AS fs::File = collections::get(hs, 4) TRAP(e)\n\
         \x20   RECOVER a\n\
         \x20 END TRAP\n\
         \x20 RETURN whichL(\"t\", g) + whichL(\"u\", g2) - 1\n\
         END FUNC\n",
        1,
    );
}

/// bug-1048: `FOR EACH e IN m` over a `Map OF String TO RES fs::File`, read through
/// `e.value`. As filed (the value written straight into an argument) it is balanced
/// since a field read in an argument position became a use of what it reads
/// through; the first seven cases pin that: the map literal, a map built by a
/// call, one built by `collections::set` in a loop, one iterated in a callee,
/// `e.key` alone, an inline `TRAP` in the loop body, and an early `RETURN` out of a
/// nested loop. The last two are the same shape with the value bound first (`RES f
/// = e.value`) and with a `collections::get` binding after the loop: there the map
/// kept its block (192 bytes a call) until its block was judged on its own.
#[test]
fn a_map_of_resources_iterated_through_its_values_frees_its_block() {
    check_at_both_levels(
        "literal",
        "FUNC whichL(label AS String, f AS fs::File) AS Integer\n\
         \x20 LET text AS String = fs::readAll(f)\n\
         \x20 RETURN 1\n\
         END FUNC\n\
         FUNC whichLH(label AS String, h AS Handle) AS Integer\n\
         \x20 MATCH h\n\
         \x20   CASE fs::File(f)\n\
         \x20     LET text AS String = fs::readAll(f)\n\
         \x20     RETURN 1\n\
         \x20   CASE tcp::Socket(s)\n\
         \x20     RETURN 2\n\
         \x20 END MATCH\n\
         \x20 RETURN 0\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 RES c AS fs::File = openOne()\n\
         \x20 LET m AS Map OF String TO RES fs::File = Map OF String TO RES fs::File { \"x\" := a, \"y\" := c }\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR EACH e IN m\n\
         \x20   t = t + whichL(\"m\", e.value)\n\
         \x20 NEXT\n\
         \x20 RETURN t - 1\n\
         END FUNC\n",
        1,
    );
    check_at_both_levels(
        "built_by_call",
        "FUNC whichL(label AS String, f AS fs::File) AS Integer\n\
         \x20 LET text AS String = fs::readAll(f)\n\
         \x20 RETURN 1\n\
         END FUNC\n\
         FUNC whichLH(label AS String, h AS Handle) AS Integer\n\
         \x20 MATCH h\n\
         \x20   CASE fs::File(f)\n\
         \x20     LET text AS String = fs::readAll(f)\n\
         \x20     RETURN 1\n\
         \x20   CASE tcp::Socket(s)\n\
         \x20     RETURN 2\n\
         \x20 END MATCH\n\
         \x20 RETURN 0\n\
         END FUNC\n\
         FUNC mkMap() AS Map OF String TO RES fs::File\n\
         \x20 MUT m AS Map OF String TO RES fs::File = Map OF String TO RES fs::File {}\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 RES c AS fs::File = openOne()\n\
         \x20 m = collections::set(m, \"x\", a)\n\
         \x20 m = collections::set(m, \"y\", c)\n\
         \x20 RETURN m\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 LET m AS Map OF String TO RES fs::File = mkMap()\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR EACH e IN m\n\
         \x20   t = t + whichL(\"m\", e.value)\n\
         \x20 NEXT\n\
         \x20 RETURN t - 1\n\
         END FUNC\n",
        1,
    );
    check_at_both_levels(
        "built_by_set",
        "FUNC whichL(label AS String, f AS fs::File) AS Integer\n\
         \x20 LET text AS String = fs::readAll(f)\n\
         \x20 RETURN 1\n\
         END FUNC\n\
         FUNC whichLH(label AS String, h AS Handle) AS Integer\n\
         \x20 MATCH h\n\
         \x20   CASE fs::File(f)\n\
         \x20     LET text AS String = fs::readAll(f)\n\
         \x20     RETURN 1\n\
         \x20   CASE tcp::Socket(s)\n\
         \x20     RETURN 2\n\
         \x20 END MATCH\n\
         \x20 RETURN 0\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 MUT m AS Map OF String TO RES fs::File = Map OF String TO RES fs::File {}\n\
         \x20 FOR j = 1 TO 3\n\
         \x20   RES a AS fs::File = openOne()\n\
         \x20   m = collections::set(m, \"k\" & toString(j), a)\n\
         \x20 NEXT\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR EACH e IN m\n\
         \x20   t = t + whichL(\"m\", e.value)\n\
         \x20 NEXT\n\
         \x20 RETURN t - 2\n\
         END FUNC\n",
        1,
    );
    check_at_both_levels(
        "iterated_in_callee",
        "FUNC whichL(label AS String, f AS fs::File) AS Integer\n\
         \x20 LET text AS String = fs::readAll(f)\n\
         \x20 RETURN 1\n\
         END FUNC\n\
         FUNC whichLH(label AS String, h AS Handle) AS Integer\n\
         \x20 MATCH h\n\
         \x20   CASE fs::File(f)\n\
         \x20     LET text AS String = fs::readAll(f)\n\
         \x20     RETURN 1\n\
         \x20   CASE tcp::Socket(s)\n\
         \x20     RETURN 2\n\
         \x20 END MATCH\n\
         \x20 RETURN 0\n\
         END FUNC\n\
         FUNC sumM(m AS Map OF String TO RES fs::File) AS Integer\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR EACH e IN m\n\
         \x20   t = t + whichL(\"m\", e.value)\n\
         \x20 NEXT\n\
         \x20 RETURN t\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 RES c AS fs::File = openOne()\n\
         \x20 LET m AS Map OF String TO RES fs::File = Map OF String TO RES fs::File { \"x\" := a, \"y\" := c }\n\
         \x20 RETURN sumM(m) - 1\n\
         END FUNC\n",
        1,
    );
    check_at_both_levels(
        "key_only",
        "FUNC whichL(label AS String, f AS fs::File) AS Integer\n\
         \x20 LET text AS String = fs::readAll(f)\n\
         \x20 RETURN 1\n\
         END FUNC\n\
         FUNC whichLH(label AS String, h AS Handle) AS Integer\n\
         \x20 MATCH h\n\
         \x20   CASE fs::File(f)\n\
         \x20     LET text AS String = fs::readAll(f)\n\
         \x20     RETURN 1\n\
         \x20   CASE tcp::Socket(s)\n\
         \x20     RETURN 2\n\
         \x20 END MATCH\n\
         \x20 RETURN 0\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 RES c AS fs::File = openOne()\n\
         \x20 LET m AS Map OF String TO RES fs::File = Map OF String TO RES fs::File { \"x\" := a, \"y\" := c }\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR EACH e IN m\n\
         \x20   t = t + len(e.key)\n\
         \x20 NEXT\n\
         \x20 RETURN t - 1\n\
         END FUNC\n",
        1,
    );
    check_at_both_levels(
        "inline_trap",
        "FUNC whichL(label AS String, f AS fs::File) AS Integer\n\
         \x20 LET text AS String = fs::readAll(f)\n\
         \x20 RETURN 1\n\
         END FUNC\n\
         FUNC whichLH(label AS String, h AS Handle) AS Integer\n\
         \x20 MATCH h\n\
         \x20   CASE fs::File(f)\n\
         \x20     LET text AS String = fs::readAll(f)\n\
         \x20     RETURN 1\n\
         \x20   CASE tcp::Socket(s)\n\
         \x20     RETURN 2\n\
         \x20 END MATCH\n\
         \x20 RETURN 0\n\
         END FUNC\n\
         FUNC whichT(label AS String, f AS fs::File, n AS Integer) AS Integer\n\
         \x20 IF n > 1 THEN\n\
         \x20   FAIL error(77, \"boom\")\n\
         \x20 END IF\n\
         \x20 LET text AS String = fs::readAll(f)\n\
         \x20 RETURN 1\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 RES c AS fs::File = openOne()\n\
         \x20 LET m AS Map OF String TO RES fs::File = Map OF String TO RES fs::File { \"x\" := a, \"y\" := c }\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 MUT n AS Integer = 0\n\
         \x20 FOR EACH e IN m\n\
         \x20   n = n + 1\n\
         \x20   t = t + whichT(\"m\", e.value, n) TRAP(err)\n\
         \x20     RECOVER 10\n\
         \x20   END TRAP\n\
         \x20 NEXT\n\
         \x20 RETURN t - 10\n\
         END FUNC\n",
        0,
    );
    check_at_both_levels(
        "nested_return",
        "FUNC whichL(label AS String, f AS fs::File) AS Integer\n\
         \x20 LET text AS String = fs::readAll(f)\n\
         \x20 RETURN 1\n\
         END FUNC\n\
         FUNC whichLH(label AS String, h AS Handle) AS Integer\n\
         \x20 MATCH h\n\
         \x20   CASE fs::File(f)\n\
         \x20     LET text AS String = fs::readAll(f)\n\
         \x20     RETURN 1\n\
         \x20   CASE tcp::Socket(s)\n\
         \x20     RETURN 2\n\
         \x20 END MATCH\n\
         \x20 RETURN 0\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR j = 1 TO 2\n\
         \x20   RES a AS fs::File = openOne()\n\
         \x20   RES c AS fs::File = openOne()\n\
         \x20   LET m AS Map OF String TO RES fs::File = Map OF String TO RES fs::File { \"x\" := a, \"y\" := c }\n\
         \x20   FOR EACH e IN m\n\
         \x20     t = t + whichL(\"m\", e.value)\n\
         \x20     IF t = 3 THEN\n\
         \x20       RETURN t - 2\n\
         \x20     END IF\n\
         \x20   NEXT\n\
         \x20 NEXT\n\
         \x20 RETURN t\n\
         END FUNC\n",
        1,
    );
    check_at_both_levels(
        "value_bound",
        "FUNC whichL(label AS String, f AS fs::File) AS Integer\n\
         \x20 LET text AS String = fs::readAll(f)\n\
         \x20 RETURN 1\n\
         END FUNC\n\
         FUNC whichLH(label AS String, h AS Handle) AS Integer\n\
         \x20 MATCH h\n\
         \x20   CASE fs::File(f)\n\
         \x20     LET text AS String = fs::readAll(f)\n\
         \x20     RETURN 1\n\
         \x20   CASE tcp::Socket(s)\n\
         \x20     RETURN 2\n\
         \x20 END MATCH\n\
         \x20 RETURN 0\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 RES c AS fs::File = openOne()\n\
         \x20 LET m AS Map OF String TO RES fs::File = Map OF String TO RES fs::File { \"x\" := a, \"y\" := c }\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR EACH e IN m\n\
         \x20   RES f AS fs::File = e.value\n\
         \x20   t = t + whichL(\"m\", f)\n\
         \x20 NEXT\n\
         \x20 RETURN t - 1\n\
         END FUNC\n",
        1,
    );
    check_at_both_levels(
        "get_after_loop",
        "FUNC whichL(label AS String, f AS fs::File) AS Integer\n\
         \x20 LET text AS String = fs::readAll(f)\n\
         \x20 RETURN 1\n\
         END FUNC\n\
         FUNC whichLH(label AS String, h AS Handle) AS Integer\n\
         \x20 MATCH h\n\
         \x20   CASE fs::File(f)\n\
         \x20     LET text AS String = fs::readAll(f)\n\
         \x20     RETURN 1\n\
         \x20   CASE tcp::Socket(s)\n\
         \x20     RETURN 2\n\
         \x20 END MATCH\n\
         \x20 RETURN 0\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 RES c AS fs::File = openOne()\n\
         \x20 LET m AS Map OF String TO RES fs::File = Map OF String TO RES fs::File { \"x\" := a, \"y\" := c }\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR EACH e IN m\n\
         \x20   t = t + whichL(\"m\", e.value)\n\
         \x20 NEXT\n\
         \x20 RES g AS fs::File = collections::get(m, \"y\")\n\
         \x20 RETURN t - 2 + whichL(\"g\", g)\n\
         END FUNC\n",
        1,
    );
}
