//! bug-1050: a `RES` binding with a `STATE` clause and an inline `TRAP` frees its
//! `STATE` block.
//!
//! `RES v AS fs::File STATE Cur = openOne() TRAP(e) … END TRAP` reaches codegen as
//! a delivery temporary typed by the PRODUCER (`$trap_valN : fs.File`, no `STATE`)
//! and `v` bound from it. The temporary carries the close obligation, so its
//! cleanup is the one that runs, and it knew of no state: the 32-byte block the
//! binding attached (and every block a write to a `String` field replaced it with)
//! was kept on every evaluation.
//!
//! Every case runs `once()` `N` times and reads the `--debug` report at two values
//! of `N`: the total printed, every allocation freed, nothing freed twice, and 300
//! iterations under a 64-descriptor limit.

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
         END TYPE\n\
         \n\
         TYPE Cur\n\
         \x20 pos AS Integer\n\
         \x20 note AS String\n\
         END TYPE\n\
         \n\
         FUNC openOne AS RES fs::File\n\
         \x20 RETURN fs::openFile(\"{}\")\n\
         END FUNC\n\
         \n\
         FUNC mkH() AS RES Handle\n\
         \x20 RES h AS Handle = openOne()\n\
         \x20 RETURN h\n\
         END FUNC\n\
         \n\
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

/// Build `body` (the declarations after the prelude, defining `once`; `{DATA}` is
/// the data file) and hold it to the rule: the total printed, every allocation
/// freed, nothing freed twice, and no descriptor left open.
fn check(case: &str, body: &str, per_iteration: u64) {
    let nonce = common::unique_nonce();
    let dir = std::env::temp_dir().join(format!("mfb_trap_state_{case}_{nonce}"));
    fs::create_dir_all(&dir).expect("create data dir");
    let data: PathBuf = dir.join("data.txt");
    fs::write(&data, "hello\n").expect("write data");
    let body = body.replace("{DATA}", &data.display().to_string());
    let source = format!("{}{}", prelude(&data), body);
    let exe = common::debug_report::build_debug(&format!("trap_state_{case}"), &source);
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

/// The reproduction as filed: a user function's file bound with a `STATE` clause
/// through an inline `TRAP`, its state written twice (the `String` field replaces
/// the block), and the same with a built-in opener written directly.
#[test]
fn a_stateful_binding_through_an_inline_trap_frees_its_state() {
    check(
        "as_filed",
        "FUNC once() AS Integer\n\
         \x20 RES v AS fs::File STATE Cur = openOne() TRAP(e)\n\
         \x20   RETURN 5\n\
         \x20 END TRAP\n\
         \x20 v.state.pos = 3\n\
         \x20 v.state.note = \"abc\" & toString(v.state.pos)\n\
         \x20 RETURN len(fs::readAll(v)) - 5 + v.state.pos - 3\n\
         END FUNC\n",
        1,
    );
    check(
        "builtin_opener",
        "FUNC once() AS Integer\n\
         \x20 RES v AS fs::File STATE Cur = fs::openFile(\"{DATA}\") TRAP(e)\n\
         \x20   RETURN 5\n\
         \x20 END TRAP\n\
         \x20 v.state.pos = 3\n\
         \x20 v.state.note = \"abc\"\n\
         \x20 RETURN len(fs::readAll(v)) - 5 + v.state.pos - 3\n\
         END FUNC\n",
        1,
    );
}

/// The union form the bug names: `RES v AS Handle STATE Cur = mkH() TRAP`.
#[test]
fn a_stateful_union_binding_through_an_inline_trap_frees_its_state() {
    check(
        "union",
        "FUNC once() AS Integer\n\
         \x20 RES v AS Handle STATE Cur = mkH() TRAP(e)\n\
         \x20   RETURN 5\n\
         \x20 END TRAP\n\
         \x20 v.state.pos = 3\n\
         \x20 v.state.note = \"abc\" & toString(v.state.pos)\n\
         \x20 RETURN which(\"u\", v) + v.state.pos - 3\n\
         END FUNC\n",
        1,
    );
}

/// The handler recovers with another file (the producer fails), the binding sits in
/// a loop body, and the function returns early after writing the state.
#[test]
fn the_state_is_freed_on_every_way_out_of_the_binding_s_scope() {
    check(
        "recover",
        "FUNC bad() AS RES fs::File\n\
         \x20 RETURN fs::openFile(\"{DATA}.missing\")\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 RES v AS fs::File STATE Cur = bad() TRAP(e)\n\
         \x20   RECOVER openOne()\n\
         \x20 END TRAP\n\
         \x20 v.state.pos = 3\n\
         \x20 RETURN len(fs::readAll(v)) - 5 + v.state.pos - 3\n\
         END FUNC\n",
        1,
    );
    check(
        "loop",
        "FUNC once() AS Integer\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR j = 1 TO 3\n\
         \x20   RES v AS fs::File STATE Cur = openOne() TRAP(e)\n\
         \x20     RETURN 5\n\
         \x20   END TRAP\n\
         \x20   v.state.pos = j\n\
         \x20   v.state.note = \"n\" & toString(j)\n\
         \x20   t = t + len(fs::readAll(v)) - 5\n\
         \x20 NEXT\n\
         \x20 RETURN t\n\
         END FUNC\n",
        3,
    );
    check(
        "early_return",
        "FUNC once() AS Integer\n\
         \x20 RES v AS fs::File STATE Cur = openOne() TRAP(e)\n\
         \x20   RETURN 5\n\
         \x20 END TRAP\n\
         \x20 v.state.note = \"early\"\n\
         \x20 IF len(v.state.note) = 5 THEN\n\
         \x20   RETURN 1\n\
         \x20 END IF\n\
         \x20 RETURN 0\n\
         END FUNC\n",
        1,
    );
}

/// The same binding with no `TRAP`, a stateful file a callee returns past its own
/// trapped binding (the caller's binding owns the state then), and a handler that
/// diverges every time (no state is ever attached).
#[test]
fn the_paths_that_were_right_stay_right() {
    check(
        "no_trap",
        "FUNC once() AS Integer\n\
         \x20 RES v AS fs::File STATE Cur = openOne()\n\
         \x20 v.state.pos = 3\n\
         \x20 v.state.note = \"abc\" & toString(v.state.pos)\n\
         \x20 RETURN len(fs::readAll(v)) - 5 + v.state.pos - 3\n\
         END FUNC\n",
        1,
    );
    check(
        "returned",
        "FUNC openS() AS RES fs::File STATE Cur\n\
         \x20 RES v AS fs::File STATE Cur = openOne() TRAP(e)\n\
         \x20   FAIL e\n\
         \x20 END TRAP\n\
         \x20 v.state.pos = 4\n\
         \x20 RETURN v\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 RES w AS fs::File STATE Cur = openS()\n\
         \x20 RETURN w.state.pos - 3\n\
         END FUNC\n",
        1,
    );
    check(
        "handler_diverges",
        "FUNC bad() AS RES fs::File\n\
         \x20 RETURN fs::openFile(\"{DATA}.missing\")\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 RES v AS fs::File STATE Cur = bad() TRAP(e)\n\
         \x20   RETURN 5\n\
         \x20 END TRAP\n\
         \x20 v.state.pos = 3\n\
         \x20 RETURN 0\n\
         END FUNC\n",
        5,
    );
}
