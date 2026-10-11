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
