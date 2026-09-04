//! bug-495: the plan-86 G1 bounds-check elision must not trust a `LET n = len(L)`
//! fact across an ENCLOSING loop's back edge.
//!
//! `recognize_provable_index` proves `FOR i = 0 TO n - k` indexes `L` in range
//! when `n = len(L)` and neither `i`, `L` nor `n` is reassigned in the `FOR`'s own
//! body. But the `n -> L` fact (`len_of_local`) can be established OUTSIDE the
//! loop and still be live when an enclosing loop reassigns `L` to a shorter list
//! AFTER the inner `FOR` and then re-enters it: `n` still holds the old length,
//! `L` is shorter, and `collections::get` runs unchecked — a heap out-of-bounds
//! read visible to the program (MEM-11: 24 elements from an 8 -> 1 list, heap
//! words leaked into a list).
//!
//! The fix declines the `Local(n)` bound whenever `L` or `n` is reassigned
//! anywhere in an enclosing loop body — while keeping the elision for the
//! straight-line `LET n = len(L)` shape the plan-86-G benchmark (`listchurn`)
//! relies on, and for a nested loop whose enclosing bodies leave `L`/`n` alone.
//! The controls below pin both halves: the checked shape carries the two
//! `list_get_invalid` guard branches, the elided shapes carry none.

mod common;
use common::{build_ncode, build_project, run_capture_with_env, temp_project};

/// MEM-11: `n = len(xs)` outside; the OUTER loop reassigns `xs` after the inner FOR.
const BACK_EDGE: &str = "\
IMPORT io\n\
IMPORT collections\n\
FUNC main() AS Integer\n\
  MUT xs AS List OF Integer = [1, 2, 3, 4, 5, 6, 7, 8]\n\
  LET n AS Integer = len(xs)\n\
  MUT out AS List OF Integer = []\n\
  FOR r = 0 TO 2\n\
    FOR i = 0 TO n - 1\n\
      out = collections::append(out, collections::get(xs, i))\n\
    NEXT\n\
    xs = [99]\n\
  NEXT\n\
  io::print(\"out=\" & toString(len(out)))\n\
  RETURN 0\n\
END FUNC\n";

/// The plan-86-G `listchurn` shape: straight-line `LET n = len(L)`, no enclosing loop.
const STRAIGHT_LINE: &str = "\
IMPORT io\n\
IMPORT collections\n\
FUNC main() AS Integer\n\
  LET xs AS List OF Integer = [1, 2, 3, 4, 5, 6, 7, 8]\n\
  LET n AS Integer = len(xs)\n\
  MUT s AS Integer = 0\n\
  FOR i = 0 TO n - 2\n\
    s = s + collections::get(xs, i) + collections::get(xs, i + 1)\n\
  NEXT\n\
  io::print(\"s=\" & toString(s))\n\
  RETURN 0\n\
END FUNC\n";

/// Nested in an enclosing loop that never reassigns `xs` or `n`: still provable.
const NESTED_STABLE: &str = "\
IMPORT io\n\
IMPORT collections\n\
FUNC main() AS Integer\n\
  LET xs AS List OF Integer = [1, 2, 3, 4, 5, 6, 7, 8]\n\
  LET n AS Integer = len(xs)\n\
  MUT s AS Integer = 0\n\
  FOR r = 0 TO 2\n\
    FOR i = 0 TO n - 1\n\
      s = s + collections::get(xs, i)\n\
    NEXT\n\
    s = s + r\n\
  NEXT\n\
  io::print(\"s=\" & toString(s))\n\
  RETURN 0\n\
END FUNC\n";

/// Branches in `main` that guard a `collections::get` (`b.lt`/`b.ge` to
/// `list_get_invalid*`): 2 per checked `get`, 0 per elided one.
fn get_guard_branches(source: &str, name: &str) -> usize {
    let project = temp_project(name, source);
    let ncode = build_ncode(&project, "macos-aarch64", name);
    let main = ncode["functions"]
        .as_array()
        .expect("functions")
        .iter()
        .find(|f| f["name"].as_str() == Some("main"))
        .expect("main function");
    let count = main["instructions"]
        .as_array()
        .expect("instructions")
        .iter()
        .filter(|inst| {
            matches!(inst["op"].as_str(), Some("b.lt") | Some("b.ge"))
                && inst["target"]
                    .as_str()
                    .is_some_and(|t| t.starts_with("list_get_invalid"))
        })
        .count();
    let _ = std::fs::remove_dir_all(&project);
    count
}

#[test]
fn back_edge_reassignment_keeps_the_bounds_check_and_traps() {
    let project = temp_project("rt_bounds_elim_backedge", BACK_EDGE);
    let executable = build_project(&project);
    let (code, stdout, stderr) = run_capture_with_env(&executable, &[]);
    assert_ne!(
        code, 0,
        "get(xs, i) on the shortened list must trap, not read out of bounds\nstdout:\n{stdout}"
    );
    assert!(
        stderr.contains("7-705-0001"),
        "expected ErrIndexOutOfRange (7-705-0001); stderr:\n{stderr}\nstdout:\n{stdout}"
    );
    assert!(
        !stdout.contains("out="),
        "program ran to completion with an out-of-bounds read: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&project);
    // The check is really there (two guard branches per `get`).
    assert_eq!(
        get_guard_branches(BACK_EDGE, "codegen_bounds_elim_backedge_checked"),
        2
    );
}

#[test]
fn straight_line_len_local_still_elides() {
    assert_eq!(
        get_guard_branches(STRAIGHT_LINE, "codegen_bounds_elim_straight_line"),
        0
    );
}

#[test]
fn nested_loop_without_enclosing_reassignment_still_elides() {
    assert_eq!(
        get_guard_branches(NESTED_STABLE, "codegen_bounds_elim_nested_stable"),
        0
    );
}
