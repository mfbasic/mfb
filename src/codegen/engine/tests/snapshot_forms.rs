//! Operand snapshots (`engine/value/operand_snapshot.rs`) for shapes the
//! runtime suites run but no in-process suite lowered.
//!
//! plan-142-G: `collections::forEach(acc, LAMBDA … -> acc = …)` hands the callee
//! `acc`'s block AND a lambda holding `acc` by reference, through which the
//! callback reassigns `acc` — freeing the block the callee is still walking. So
//! the call walks a snapshot (a stack object named `operand_snapshot`, freed with
//! the statement). Without the by-reference sibling there is nothing to snapshot.
//!
//! bug-666: a `FOR EACH` over a module-level global walks the global's live
//! block, which a store to the global in the body frees mid-loop — so a body
//! that can store it walks a snapshot, and a body that only reads borrows.

use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_src_cached, code_function, CodeTarget};

fn main_slots(body: &str, target: CodeTarget) -> Vec<String> {
    let source = format!(
        "IMPORT collections\nIMPORT io\n\nTYPE P\n  a AS Integer\n  name AS String\nEND TYPE\n\nMUT g AS List OF Integer = [1, 2, 3]\n\nFUNC main() AS Integer\n{body}\n  RETURN 0\nEND FUNC\n"
    );
    code_function(code_for_src_cached(&source, target, Console), "main")
        .stack_slots
        .iter()
        .map(|slot| slot.name.clone())
        .collect()
}

fn snapshots(body: &str, target: CodeTarget) -> bool {
    main_slots(body, target)
        .iter()
        .any(|name| name.starts_with("operand_snapshot"))
}

#[test]
fn a_call_given_a_by_reference_way_to_reassign_its_argument_walks_a_snapshot() {
    let reassigning = "  MUT acc AS List OF Integer = [1, 2, 3]\n  collections::forEach(acc, LAMBDA(v AS Integer) -> acc = collections::append(acc, v))\n  io::print(toString(len(acc)))";
    for target in CodeTarget::ALL {
        assert!(
            snapshots(reassigning, target),
            "{}: forEach(acc, LAMBDA -> acc = …) walks acc's own block",
            target.name()
        );
    }
}

#[test]
fn without_a_by_reference_sibling_the_argument_is_borrowed() {
    let reading = "  LET acc AS List OF Integer = [1, 2, 3]\n  collections::forEach(acc, LAMBDA(v AS Integer) -> io::print(toString(v + len(acc))))";
    assert!(!snapshots(reading, CodeTarget::LinuxX86_64));
    // An empty `WITH` has no later operand to free its target.
    let empty_with =
        "  LET p AS P = P[a := 1, name := \"n\"]\n  LET q AS P = WITH p { }\n  io::print(q.name)";
    assert!(!snapshots(empty_with, CodeTarget::LinuxX86_64));
}

#[test]
fn a_for_each_over_a_global_its_body_stores_walks_a_snapshot() {
    let storing = "  FOR EACH x IN g\n    IF x < 3 THEN\n      g = collections::append(g, x)\n    END IF\n  NEXT\n  io::print(toString(len(g)))";
    for target in CodeTarget::ALL {
        assert!(snapshots(storing, target), "{}", target.name());
    }
    let reading = "  FOR EACH x IN g\n    io::print(toString(x))\n  NEXT";
    assert!(!snapshots(reading, CodeTarget::LinuxX86_64));
}

/// Every function of `source`'s plan that reserves an operand snapshot.
fn snapshotting_functions(source: &str) -> Vec<String> {
    code_for_src_cached(source, CodeTarget::LinuxX86_64, Console)
        .functions
        .iter()
        .filter(|function| {
            function
                .stack_slots
                .iter()
                .any(|slot| slot.name.starts_with("operand_snapshot"))
        })
        .map(|function| function.name.clone())
        .collect()
}

/// Inside a lambda, a `String` captured by reference is read through the
/// parent's slot; a later operand that calls user code (twice, here) could
/// reassign it through another reference, so the read is snapshotted first. A
/// later operand that runs no user code leaves it borrowed.
#[test]
fn a_by_reference_read_before_a_user_call_is_snapshotted() {
    let program = |later: &str| {
        format!(
            "IMPORT collections\nIMPORT io\n\nFUNC say(v AS Integer) AS Integer\n  io::print(\"say\")\n  RETURN v\nEND FUNC\n\n\
             FUNC main() AS Integer\n  MUT acc AS String = \"a\"\n  \
             collections::forEach([1, 2], LAMBDA(v AS Integer) -> io::print(acc & {later}))\n  \
             acc = \"b\"\n  io::print(acc)\n  RETURN 0\nEND FUNC\n"
        )
    };
    assert!(!snapshotting_functions(&program("toString(say(v) + say(v))")).is_empty());
    assert!(snapshotting_functions(&program("toString(v + v)")).is_empty());
}

/// An address-taken local (a callback holds it by reference) read before a
/// later operand that calls user code through a function VALUE — a `FUNC` local
/// or a `FUNC` global — is snapshotted like one read before a named call; a later
/// operand that runs no user code leaves it borrowed.
#[test]
fn a_call_through_a_function_value_counts_as_user_code() {
    let program = |later: &str| {
        format!(
            "IMPORT collections\nIMPORT io\n\nFUNC say(v AS Integer) AS Integer\n  io::print(\"say\")\n  RETURN v\nEND FUNC\n\n\
             MUT gf AS FUNC(Integer) AS Integer = say\n\n\
             FUNC main() AS Integer\n  MUT acc AS String = \"a\"\n  \
             collections::forEach([1, 2], LAMBDA(v AS Integer) -> acc = \"b\")\n  \
             LET f AS FUNC(Integer) AS Integer = say\n  io::print(acc & {later})\n  RETURN 0\nEND FUNC\n"
        )
    };
    let main_snapshots = |later: &str| {
        code_function(
            code_for_src_cached(&program(later), CodeTarget::LinuxX86_64, Console),
            "main",
        )
        .stack_slots
        .iter()
        .any(|slot| slot.name.starts_with("operand_snapshot"))
    };
    assert!(main_snapshots("toString(f(1))"), "through a FUNC local");
    assert!(main_snapshots("toString(gf(1))"), "through a FUNC global");
    assert!(!main_snapshots("toString(len(acc))"), "no user code");
}
