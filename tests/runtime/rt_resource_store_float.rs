//! bug-1045: a value from an inner scope stored into a container declared in an
//! enclosing one takes that container's scope.
//!
//! `mfb spec language resource-management` 15.6: adding a reference to a resource to
//! a collection raises the resource's owning scope to the collection's scope when
//! that scope outlives the current owner. The escape analysis does that for a `RES`
//! binding named as the element. A value that only REACHES a resource had no such
//! rule: a `LET k AS HB = mk()` in a loop body (`HB` a record with a `RES` field)
//! adopted the resource for the loop body's scope, and appending `k`, or `k.h`, to a
//! list declared before the loop left the list holding a handle the end of the
//! iteration closed. Reading it after the loop failed with `7-703-0004`.
//!
//! Every case runs `once()` `N` times and reads the `--debug` report at two values
//! of `N`: the total printed proves every stored resource is still readable where
//! the program reads it, every allocation is freed, nothing is freed twice, and 300
//! iterations under a 64-descriptor limit prove each descriptor is closed.

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
         \n\
         FUNC mk() AS HB\n\
         \x20 RETURN HB[openOne()]\n\
         END FUNC\n\
         \n\
         FUNC mkU() AS HU\n\
         \x20 RETURN HU[openOne()]\n\
         END FUNC\n\
         \n\
         FUNC sumHB(xs AS List OF HB) AS Integer\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR EACH x IN xs\n\
         \x20   t = t + rd(x.h) + 1\n\
         \x20 NEXT\n\
         \x20 RETURN t\n\
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
    let dir = std::env::temp_dir().join(format!("mfb_store_float_{case}_{nonce}"));
    fs::create_dir_all(&dir).expect("create data dir");
    let data: PathBuf = dir.join("data.txt");
    fs::write(&data, "hello\n").expect("write data");
    let source = format!("{}{}", prelude(&data), body);
    let exe = common::debug_report::build_debug(&format!("store_float_{case}"), &source);
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

/// The two reproductions as filed. `r2` appends the record, `r5` its field. Each
/// file is six bytes and the loop body reads it once before the store, so the read
/// after the loop finds it at its end: one per element, where a closed handle
/// raised `ErrResourceClosed`.
#[test]
fn a_record_from_a_loop_body_appended_to_an_outer_list_keeps_its_resource_open() {
    check(
        "record",
        "FUNC once() AS Integer\n\
         \x20 MUT xs AS List OF HB = []\n\
         \x20 FOR j = 1 TO 3\n\
         \x20   LET k AS HB = mk()\n\
         \x20   LET a AS Integer = rd(k.h)\n\
         \x20   xs = collections::append(xs, k)\n\
         \x20 NEXT\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR EACH x IN xs\n\
         \x20   t = t + rd(x.h) + 1\n\
         \x20 NEXT\n\
         \x20 RETURN t\n\
         END FUNC\n",
        3,
    );
    check(
        "field",
        "FUNC once() AS Integer\n\
         \x20 MUT fsl AS List OF RES fs::File = []\n\
         \x20 FOR j = 1 TO 2\n\
         \x20   LET k AS HB = mk()\n\
         \x20   LET a AS Integer = rd(k.h)\n\
         \x20   fsl = collections::append(fsl, k.h)\n\
         \x20 NEXT\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR EACH f IN fsl\n\
         \x20   t = t + rd(f) + 1\n\
         \x20 NEXT\n\
         \x20 RETURN t\n\
         END FUNC\n",
        2,
    );
}

/// The other stores that put a value into a container: `prepend` and `insert`,
/// `set` over an element, a list literal, a `Map` `set`, an assignment of the
/// record itself to an outer record, and a `WITH` that replaces an outer record's
/// field.
#[test]
fn every_inserting_store_raises_what_it_stores() {
    check(
        "prepend_insert",
        "FUNC once() AS Integer\n\
         \x20 MUT xs AS List OF HB = []\n\
         \x20 FOR j = 1 TO 2\n\
         \x20   LET k AS HB = mk()\n\
         \x20   LET k2 AS HB = mk()\n\
         \x20   xs = collections::prepend(xs, k)\n\
         \x20   xs = collections::insert(xs, 0, k2)\n\
         \x20 NEXT\n\
         \x20 RETURN sumHB(xs)\n\
         END FUNC\n",
        28,
    );
    check(
        "set",
        "FUNC once() AS Integer\n\
         \x20 MUT xs AS List OF HB = []\n\
         \x20 FOR j = 1 TO 3\n\
         \x20   LET k AS HB = mk()\n\
         \x20   IF j = 1 THEN\n\
         \x20     xs = collections::append(xs, k)\n\
         \x20   ELSE\n\
         \x20     xs = collections::set(xs, 0, k)\n\
         \x20   END IF\n\
         \x20 NEXT\n\
         \x20 RETURN sumHB(xs)\n\
         END FUNC\n",
        7,
    );
    check(
        "list_literal",
        "FUNC once() AS Integer\n\
         \x20 MUT xs AS List OF HB = []\n\
         \x20 FOR j = 1 TO 2\n\
         \x20   LET k AS HB = mk()\n\
         \x20   xs = [k]\n\
         \x20 NEXT\n\
         \x20 RETURN sumHB(xs)\n\
         END FUNC\n",
        7,
    );
    check(
        "map_set",
        "FUNC sumM(m AS Map OF String TO HB) AS Integer\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR EACH e IN m\n\
         \x20   t = t + rd(e.value.h) + 1\n\
         \x20 NEXT\n\
         \x20 RETURN t\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 MUT m AS Map OF String TO HB = Map OF String TO HB {}\n\
         \x20 FOR j = 1 TO 2\n\
         \x20   LET k AS HB = mk()\n\
         \x20   m = collections::set(m, toString(j), k)\n\
         \x20 NEXT\n\
         \x20 RETURN sumM(m)\n\
         END FUNC\n",
        14,
    );
    check(
        "record_assign",
        "FUNC once() AS Integer\n\
         \x20 MUT last AS HB = mk()\n\
         \x20 FOR j = 1 TO 2\n\
         \x20   LET k AS HB = mk()\n\
         \x20   last = k\n\
         \x20 NEXT\n\
         \x20 RETURN rd(last.h)\n\
         END FUNC\n",
        6,
    );
    check(
        "with_field",
        "FUNC once() AS Integer\n\
         \x20 MUT keep AS HB = mk()\n\
         \x20 FOR j = 1 TO 2\n\
         \x20   LET k AS HB = mk()\n\
         \x20   keep = WITH keep { h := k.h }\n\
         \x20 NEXT\n\
         \x20 RETURN rd(keep.h)\n\
         END FUNC\n",
        6,
    );
}

/// A record whose field is a `RES Handle` (a resource union), appended whole and by
/// its field (the field is the union's `{tag, record}` box held inline).
#[test]
fn a_resource_union_field_is_raised_like_a_concrete_one() {
    check(
        "union",
        "FUNC once() AS Integer\n\
         \x20 MUT xs AS List OF HU = []\n\
         \x20 MUT hs AS List OF RES Handle = []\n\
         \x20 FOR j = 1 TO 3\n\
         \x20   LET k AS HU = mkU()\n\
         \x20   xs = collections::append(xs, k)\n\
         \x20 NEXT\n\
         \x20 FOR j = 1 TO 2\n\
         \x20   LET k AS HU = mkU()\n\
         \x20   hs = collections::append(hs, k.h)\n\
         \x20 NEXT\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR EACH x IN xs\n\
         \x20   t = t + which(x.h)\n\
         \x20 NEXT\n\
         \x20 FOR EACH y IN hs\n\
         \x20   t = t + which(y)\n\
         \x20 NEXT\n\
         \x20 RETURN t\n\
         END FUNC\n",
        30,
    );
}

/// An `IF` body, a `WHILE` body and a loop two scopes inside the list's.
#[test]
fn the_store_may_sit_in_any_inner_scope_and_any_number_of_scopes_in() {
    check(
        "if_while",
        "FUNC once() AS Integer\n\
         \x20 MUT xs AS List OF HB = []\n\
         \x20 FOR j = 1 TO 3\n\
         \x20   IF j > 1 THEN\n\
         \x20     LET k AS HB = mk()\n\
         \x20     xs = collections::append(xs, k)\n\
         \x20   END IF\n\
         \x20 NEXT\n\
         \x20 MUT n AS Integer = 0\n\
         \x20 WHILE n < 2\n\
         \x20   LET w AS HB = mk()\n\
         \x20   xs = collections::append(xs, w)\n\
         \x20   n = n + 1\n\
         \x20 END WHILE\n\
         \x20 RETURN sumHB(xs)\n\
         END FUNC\n",
        28,
    );
    check(
        "nested",
        "FUNC once() AS Integer\n\
         \x20 MUT xs AS List OF HB = []\n\
         \x20 FOR j = 1 TO 2\n\
         \x20   FOR q = 1 TO 2\n\
         \x20     LET k AS HB = mk()\n\
         \x20     xs = collections::append(xs, k)\n\
         \x20   NEXT\n\
         \x20 NEXT\n\
         \x20 RETURN sumHB(xs)\n\
         END FUNC\n",
        28,
    );
}

/// An early `RETURN` out of the loop that stored (the function's scope closes what
/// the list took), and a list that is `RETURN`ed and bound by the caller.
#[test]
fn the_raised_resource_follows_the_container_out() {
    check(
        "early_return",
        "FUNC once() AS Integer\n\
         \x20 MUT xs AS List OF HB = []\n\
         \x20 FOR j = 1 TO 3\n\
         \x20   LET k AS HB = mk()\n\
         \x20   xs = collections::append(xs, k)\n\
         \x20   IF j = 2 THEN\n\
         \x20     RETURN sumHB(xs)\n\
         \x20   END IF\n\
         \x20 NEXT\n\
         \x20 RETURN 0\n\
         END FUNC\n",
        14,
    );
    check(
        "returned",
        "FUNC build() AS List OF HB\n\
         \x20 MUT xs AS List OF HB = []\n\
         \x20 FOR j = 1 TO 3\n\
         \x20   LET k AS HB = mk()\n\
         \x20   xs = collections::append(xs, k)\n\
         \x20 NEXT\n\
         \x20 RETURN xs\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 LET xs AS List OF HB = build()\n\
         \x20 RETURN sumHB(xs)\n\
         END FUNC\n",
        21,
    );
}

/// A store that runs on some iterations only (the others close at the end of the
/// loop body, as before), a record with two resources of which one is stored, a
/// read through the record after the store in the same iteration, and one record
/// stored into two outer lists.
#[test]
fn only_what_was_stored_is_raised() {
    check(
        "conditional",
        "FUNC once() AS Integer\n\
         \x20 MUT xs AS List OF HB = []\n\
         \x20 FOR j = 1 TO 4\n\
         \x20   LET k AS HB = mk()\n\
         \x20   IF j > 2 THEN\n\
         \x20     xs = collections::append(xs, k)\n\
         \x20   END IF\n\
         \x20 NEXT\n\
         \x20 RETURN sumHB(xs)\n\
         END FUNC\n",
        14,
    );
    check(
        "one_of_two_fields",
        "TYPE Two\n\
         \x20 a AS RES fs::File\n\
         \x20 b AS RES fs::File\n\
         END TYPE\n\
         FUNC mk2() AS Two\n\
         \x20 RETURN Two[openOne(), openOne()]\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 MUT fsl AS List OF RES fs::File = []\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR j = 1 TO 2\n\
         \x20   LET k AS Two = mk2()\n\
         \x20   fsl = collections::append(fsl, k.a)\n\
         \x20   t = t + rd(k.b)\n\
         \x20 NEXT\n\
         \x20 FOR EACH f IN fsl\n\
         \x20   t = t + rd(f) + 1\n\
         \x20 NEXT\n\
         \x20 RETURN t\n\
         END FUNC\n",
        26,
    );
    check(
        "read_after_store",
        "FUNC once() AS Integer\n\
         \x20 MUT xs AS List OF HB = []\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR j = 1 TO 3\n\
         \x20   LET k AS HB = mk()\n\
         \x20   xs = collections::append(xs, k)\n\
         \x20   t = t + rd(k.h)\n\
         \x20 NEXT\n\
         \x20 RETURN t + sumHB(xs)\n\
         END FUNC\n",
        21,
    );
    check(
        "two_lists",
        "FUNC once() AS Integer\n\
         \x20 MUT xs AS List OF HB = []\n\
         \x20 MUT ys AS List OF HB = []\n\
         \x20 FOR j = 1 TO 2\n\
         \x20   LET k AS HB = mk()\n\
         \x20   xs = collections::append(xs, k)\n\
         \x20   ys = collections::append(ys, k)\n\
         \x20 NEXT\n\
         \x20 RETURN sumHB(xs) + sumHB(ys)\n\
         END FUNC\n",
        16,
    );
}

/// A `FOR EACH` variable is a reference to an element of the list it iterates:
/// stored into a list declared further out than that list, what it reaches takes
/// the outer list's scope. A list of records, and a `List OF RES fs::File`.
#[test]
fn an_element_of_an_inner_container_is_raised_out_of_that_container() {
    check(
        "for_each_record",
        "FUNC once() AS Integer\n\
         \x20 MUT all AS List OF HB = []\n\
         \x20 FOR j = 1 TO 2\n\
         \x20   MUT mid AS List OF HB = []\n\
         \x20   FOR q = 1 TO 2\n\
         \x20     LET k AS HB = mk()\n\
         \x20     mid = collections::append(mid, k)\n\
         \x20   NEXT\n\
         \x20   FOR EACH m IN mid\n\
         \x20     all = collections::append(all, m)\n\
         \x20   NEXT\n\
         \x20 NEXT\n\
         \x20 RETURN sumHB(all)\n\
         END FUNC\n",
        28,
    );
    check(
        "for_each_handle",
        "FUNC once() AS Integer\n\
         \x20 MUT all AS List OF RES fs::File = []\n\
         \x20 FOR j = 1 TO 2\n\
         \x20   MUT mid AS List OF RES fs::File = []\n\
         \x20   FOR q = 1 TO 2\n\
         \x20     RES f AS fs::File = openOne()\n\
         \x20     mid = collections::append(mid, f)\n\
         \x20   NEXT\n\
         \x20   FOR EACH m IN mid\n\
         \x20     all = collections::append(all, m)\n\
         \x20   NEXT\n\
         \x20 NEXT\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR EACH g IN all\n\
         \x20   t = t + rd(g) + 1\n\
         \x20 NEXT\n\
         \x20 RETURN t\n\
         END FUNC\n",
        28,
    );
}

/// A parameter's handle wrapped in a record in a loop and appended to the
/// function's own list stays the caller's: the caller reads it after the call.
#[test]
fn a_handle_owned_further_out_is_left_with_its_owner() {
    check(
        "callers_handle",
        "FUNC viaParam(a AS fs::File) AS Integer\n\
         \x20 MUT xs AS List OF HB = []\n\
         \x20 FOR j = 1 TO 2\n\
         \x20   LET k AS HB = wrap(a)\n\
         \x20   xs = collections::append(xs, k)\n\
         \x20 NEXT\n\
         \x20 RETURN len(xs) * 7\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 LET n AS Integer = viaParam(a)\n\
         \x20 RETURN n + rd(a)\n\
         END FUNC\n",
        20,
    );
}
