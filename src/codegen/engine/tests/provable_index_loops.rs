//! plan-86 G1: a `FOR i = 0 TO len(L) - k` loop (`k >= 1`, step 1) whose body
//! changes neither `i` nor `L` reads `collections::get(L, i)` without a bounds
//! check (`engine/control/builder_control/loops.rs`, `recognize_provable_index`).
//! The proof is all-or-nothing and fails closed: every other bound shape, and
//! every reassignment that could stale the bound, keeps the check — an unsound
//! elision is a silent out-of-bounds read.
//!
//! The instrument is the get's bounds check itself: an unsigned `count <= index`
//! compare and its `branch_ls` to the miss path. Each program differs from its
//! twin only in the bound, so a declined proof shows up as exactly one more
//! `branch_ls` in `main`.

use crate::arch::ops::CodeOp;
use crate::testutil::{code_for_src_on, code_function, CodeTarget};

/// The `branch_ls` instructions in `main` of a program running `body`.
fn unsigned_range_branches(body: &str) -> usize {
    let source = format!(
        "IMPORT collections\nIMPORT io\n\nFUNC main() AS Integer\n  \
         MUT xs AS List OF Integer = [1, 2, 3]\n  MUT k AS Integer = 1\n  \
         MUT s AS Integer = 0\n{body}  io::print(toString(s + k))\n  RETURN 0\nEND FUNC\n"
    );
    let plan = code_for_src_on(&source, CodeTarget::LinuxX86_64);
    code_function(&plan, "main")
        .instructions
        .iter()
        .filter(|instruction| instruction.op == CodeOp::BranchLs)
        .count()
}

fn loop_to(end: &str) -> String {
    format!("  FOR i = 0 TO {end}\n    s = s + collections::get(xs, i)\n  NEXT\n")
}

#[test]
fn only_len_minus_a_positive_constant_proves_the_index() {
    let proven = unsigned_range_branches(&loop_to("len(xs) - 1"));
    for end in ["len(xs) + 0", "len(xs) - k", "len(xs) - 0"] {
        assert_eq!(
            unsigned_range_branches(&loop_to(end)),
            proven + 1,
            "`FOR i = 0 TO {end}` must keep the get's bounds check"
        );
    }
}

/// A `len` fact bound to a local proves the index while the local holds it; a body
/// that reassigns the local does not.
#[test]
fn a_reassigned_length_local_keeps_the_check() {
    let fact = "  LET n AS Integer = len(xs)\n  FOR i = 0 TO n - 1\n    \
                s = s + collections::get(xs, i)\n  NEXT\n";
    let reassigned = "  MUT n AS Integer = len(xs)\n  FOR i = 0 TO n - 1\n    \
                      s = s + collections::get(xs, i)\n    n = 1\n  NEXT\n";
    assert_eq!(
        unsigned_range_branches(reassigned),
        unsigned_range_branches(fact) + 1
    );
}

/// bug-495: a `len` fact recorded BEFORE an enclosing loop whose body shrinks or
/// grows `L` is stale on that loop's second iteration, so the inner index is not
/// proven; the same fact re-recorded inside the outer loop is fresh each time and
/// proves it.
#[test]
fn a_length_fact_an_enclosing_loop_can_stale_keeps_the_check() {
    let outer = |inside: &str, before: &str| {
        format!(
            "{before}  FOR j = 1 TO 2\n{inside}    FOR i = 0 TO n - 1\n      \
             s = s + collections::get(xs, i)\n    NEXT\n    \
             xs = collections::append(xs, 4)\n  NEXT\n"
        )
    };
    let fresh = outer("    LET n AS Integer = len(xs)\n", "");
    let stale = outer("", "  LET n AS Integer = len(xs)\n");
    assert_eq!(
        unsigned_range_branches(&stale),
        unsigned_range_branches(&fresh) + 1
    );
}
