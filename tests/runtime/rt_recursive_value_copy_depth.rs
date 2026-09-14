//! plan-134-B: deep-copying a recursive value succeeds at any depth.
//!
//! ## Why this exists
//!
//! A value of a recursive type (`TYPE Node / kids AS List OF Node`) is a chain of
//! separately allocated blocks. Its deep copy — run by `collections::get` of a
//! recursive element (bug-538) and by every thread transfer (bug-391) — used to be a
//! per-type function that called itself once per edge, on the native stack. A chain
//! 50 000 levels deep copied; at 70 000 the process died with SIGSEGV (measured,
//! plan-134-A §2.1), while building and holding a 1 000 000-deep chain without
//! copying it was fine. So the depth limit was the copy's recursion, and any input
//! that builds a deep value — a user tree, a regex backtracking chain (up to 500 000
//! deep) — could crash the program the moment it was copied.
//!
//! plan-134-B replaced the recursion with one walker that keeps its work stack in the
//! arena. These tests copy a 100 000-deep chain both ways it is copied today.

#[path = "../common/mod.rs"]
mod common;

use std::time::Duration;

const DEPTH: u64 = 100_000;

/// Build `source` (with `{N}` replaced by [`DEPTH`]), run it, and assert it exits 0
/// printing exactly `expected`.
fn assert_runs(name: &str, source: &str, expected: &str) {
    let program = source.replace("{N}", &DEPTH.to_string());
    let project = common::temp_project(name, &program);
    let exe = common::build_project(&project);
    let (status, stdout, _rss) = common::run_bounded_with_rss(
        &exe,
        Duration::from_secs(300),
        "the deep-copy probe did not finish",
    );
    assert!(
        status.success(),
        "{name}: copying a {DEPTH}-deep recursive value {} — the deep copy must not \
         recurse on the native stack.\nstdout:\n{stdout}",
        common::exit_description(&status)
    );
    assert_eq!(stdout.trim(), expected, "{name}: wrong output");
    let _ = std::fs::remove_dir_all(&project);
}

/// `collections::get` of a recursive element hands back an independent deep copy
/// (bug-538), so reading the chain back out of the list copies all of it.
const GET_COPY_SOURCE: &str = "IMPORT io
IMPORT collections

TYPE Node
  kids AS List OF Node
  tag AS Integer
END TYPE

FUNC main AS Integer
  MUT cur AS Node = Node[kids := [], tag := 0]
  MUT i AS Integer = 1
  WHILE i <= {N}
    cur = Node[kids := [cur], tag := i]
    i = i + 1
  END WHILE
  MUT xs AS List OF Node = []
  xs = collections::append(xs, cur)
  LET top AS Node = collections::get(xs, 0)
  io::print(\"top=\" & toString(top.tag))
  RETURN 0
END FUNC
";

/// A worker's result is deep-copied out of the worker arena by `thread::waitFor`.
/// The list is read with `FOR EACH`, which borrows its element, so the thread copy
/// is the only deep copy this program makes.
const THREAD_COPY_SOURCE: &str = "IMPORT io
IMPORT thread

TYPE Node
  kids AS List OF Node
  tag AS Integer
END TYPE

ISOLATED FUNC buildChain(w AS ThreadWorker OF Integer TO List OF Node, n AS Integer) AS List OF Node
  MUT cur AS Node = Node[kids := [], tag := 0]
  MUT i AS Integer = 1
  WHILE i <= n
    cur = Node[kids := [cur], tag := i]
    i = i + 1
  END WHILE
  LET out AS List OF Node = [cur]
  RETURN out
END FUNC

FUNC main AS Integer
  LET t AS Thread OF Integer TO List OF Node = thread::start(buildChain, {N})
  LET xs AS List OF Node = thread::waitFor(t)
  FOR EACH top IN xs
    io::print(\"top=\" & toString(top.tag) & \" len=\" & toString(len(xs)))
  NEXT
  RETURN 0
END FUNC
";

/// The walker's copy is the same value as its source, not merely a value of the right
/// depth: a three-level `json::Json` with arrays, objects, strings and literals copied
/// by `collections::get` stringifies exactly like the original, and stays whole after
/// the list slot it was copied from is overwritten in place.
const JSON_COPY_SOURCE: &str = "IMPORT io
IMPORT json
IMPORT collections

FUNC main AS Integer
  LET doc AS json::Json = json::parse(\"{\\u{22}a\\u{22}:[1,{\\u{22}b\\u{22}:[true,null,\\u{22}x\\u{22}]}],\\u{22}c\\u{22}:{\\u{22}d\\u{22}:{\\u{22}e\\u{22}:[2.5]}}}\")
  MUT xs AS List OF json::Json = [doc]
  LET copied AS json::Json = collections::get(xs, 0)
  LET empty AS json::Json = json::JsonNull[NOTHING]
  xs = collections::set(xs, 0, empty)
  io::print(\"copy=\" & json::stringify(copied))
  io::print(\"source=\" & json::stringify(doc))
  io::print(\"slot=\" & json::stringify(collections::get(xs, 0)))
  RETURN 0
END FUNC
";

#[cfg(unix)]
#[test]
fn a_copied_json_tree_equals_its_source_and_is_independent_of_the_list() {
    let project = common::temp_project("p134b_json_copy_value", JSON_COPY_SOURCE);
    let exe = common::build_project(&project);
    let (status, stdout, _rss) = common::run_bounded_with_rss(
        &exe,
        Duration::from_secs(60),
        "the json copy probe did not finish",
    );
    assert!(
        status.success(),
        "the json copy probe {}.\nstdout:\n{stdout}",
        common::exit_description(&status)
    );
    let tree = r#"{"a":[1,{"b":[true,null,"x"]}],"c":{"d":{"e":[2.5]}}}"#;
    assert_eq!(
        stdout.trim(),
        format!("copy={tree}\nsource={tree}\nslot=null"),
        "a deep copy must reproduce every level of its source"
    );
    let _ = std::fs::remove_dir_all(&project);
}

#[cfg(unix)]
#[test]
fn a_100000_deep_chain_copies_through_collections_get() {
    assert_runs(
        "p134b_get_copy_depth",
        GET_COPY_SOURCE,
        &format!("top={DEPTH}"),
    );
}

#[cfg(unix)]
#[test]
fn a_100000_deep_chain_copies_out_of_a_worker_thread() {
    assert_runs(
        "p134b_thread_copy_depth",
        THREAD_COPY_SOURCE,
        &format!("top={DEPTH} len=1"),
    );
}
