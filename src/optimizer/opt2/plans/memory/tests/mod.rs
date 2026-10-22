//! The slot-availability dataflow, driven through its consumer
//! (`opt2::stldfwd::forward`, which runs both memory rows and the Store PRE /
//! Load PRE row on this one analysis).

use crate::arch::aarch64::regmodel::Aarch64RegisterModel;
use crate::arch::ops::CodeOp;
use crate::codegen::engine::types::CodeInstruction;
use crate::optimizer::opt2::stldfwd::forward;
use crate::optimizer::{with_opt_level, OptLevel};

fn ci(op: &str, fields: &[(&'static str, &str)]) -> CodeInstruction {
    let mut inst = CodeInstruction::new(op);
    for (k, v) in fields {
        inst = inst.physical_field(k, v);
    }
    inst
}

fn run(stream: &mut Vec<CodeInstruction>) {
    with_opt_level(OptLevel(3), || forward(stream, &Aarch64RegisterModel));
}

fn store(src: &str, offset: &str) -> CodeInstruction {
    ci(
        "str_u64",
        &[("src", src), ("base", "sp"), ("offset", offset)],
    )
}

fn load(dst: &str, offset: &str) -> CodeInstruction {
    ci(
        "ldr_u64",
        &[("dst", dst), ("base", "sp"), ("offset", offset)],
    )
}

/// Every instruction that writes `reg` through its `dst` field.
fn writers_of(stream: &[CodeInstruction], reg: &str) -> Vec<usize> {
    stream
        .iter()
        .enumerate()
        .filter(|(_, inst)| inst.get("dst").as_deref() == Some(reg))
        .map(|(index, _)| index)
        .collect()
}

/// Store PRE / Load PRE must not complete a load into a holder register
/// whose own value is still needed on the gap path. Here `%v1` is loaded
/// ABOVE the branch, so it is live on both arms and read after the join; the
/// `other` arm overwrites the slot from a physical register (untracked, so
/// the slot is a gap there). Reloading the slot into `%v1` at the end of
/// `other` would make the join's `str %v1` store the call's result instead of
/// the value `%v1` was loaded with.
#[test]
fn a_partial_load_never_redefines_a_holder_live_on_the_gap_path() {
    let mut stream = vec![
        load("%v1", "8"),
        ci("cmp_imm", &[("lhs", "%v9"), ("rhs", "0")]),
        ci("b.eq", &[("target", "other")]),
        ci("b", &[("target", "join")]),
        ci("label", &[("name", "other")]),
        ci("bl", &[("target", "_mfb_fn_callee")]),
        store("x0", "8"),
        ci("b", &[("target", "join")]),
        ci("label", &[("name", "join")]),
        load("%v2", "8"),
        store("%v1", "16"),
        store("%v2", "24"),
        ci("ret", &[]),
    ];
    run(&mut stream);
    assert_eq!(
        writers_of(&stream, "%v1"),
        vec![0],
        "%v1 keeps its single definition: {:?}",
        stream.iter().map(|i| i.op).collect::<Vec<_>>()
    );
    let join_load = stream
        .iter()
        .position(|inst| inst.get("dst").as_deref() == Some("%v2"))
        .expect("the join still defines %v2");
    assert_eq!(
        stream[join_load].op,
        CodeOp::LdrU64,
        "the join keeps its own load"
    );
}

/// A loop that never touches the slot keeps it available: the value stored
/// before the loop forwards to the load after it. The loop header's back-edge
/// predecessor has no facts on the first visit; it must count as "not yet
/// known", not as "nothing known" — otherwise every block at or after any
/// loop is left with no facts and nothing there is ever forwarded.
#[test]
fn a_slot_untouched_by_a_loop_forwards_past_it() {
    let mut stream = vec![
        ci(
            "mov_imm",
            &[("dst", "%v1"), ("type", "Integer"), ("value", "5")],
        ),
        store("%v1", "8"),
        ci("label", &[("name", "head")]),
        ci("cmp_imm", &[("lhs", "%v9"), ("rhs", "0")]),
        ci("b.ne", &[("target", "head")]),
        load("%v2", "8"),
        store("%v2", "16"),
        ci("ret", &[]),
    ];
    run(&mut stream);
    assert_eq!(
        stream[5].op,
        CodeOp::Mov,
        "the post-loop reload became a copy"
    );
    assert_eq!(stream[5].get("src").as_deref(), Some("%v1"));
    assert_eq!(stream[5].get("dst").as_deref(), Some("%v2"));
}

/// The other half of the loop contract: the header's first, optimistic visit
/// sees only the entry edge, and a store later in the loop body must take the
/// slot back out when the back edge is met. The slot holds `%v3`'s value, not
/// `%v1`'s, on the second trip through the header, so the load after the
/// loop is not forwarded from either.
#[test]
fn a_slot_rewritten_inside_a_loop_is_not_forwarded_past_it() {
    let mut stream = vec![
        ci(
            "mov_imm",
            &[("dst", "%v1"), ("type", "Integer"), ("value", "5")],
        ),
        store("%v1", "8"),
        ci("label", &[("name", "head")]),
        ci("cmp_imm", &[("lhs", "%v9"), ("rhs", "0")]),
        ci("b.eq", &[("target", "done")]),
        ci(
            "mov_imm",
            &[("dst", "%v3"), ("type", "Integer"), ("value", "7")],
        ),
        store("%v3", "8"),
        ci("b", &[("target", "head")]),
        ci("label", &[("name", "done")]),
        load("%v2", "8"),
        store("%v2", "16"),
        ci("ret", &[]),
    ];
    run(&mut stream);
    assert_eq!(
        stream[9].op,
        CodeOp::LdrU64,
        "the slot differs on the back edge"
    );
}

/// A block one of whose predecessors the CFG walk never reaches (here `dead`,
/// which nothing branches to) gets no facts: that predecessor is entered, if
/// at all, by control flow the CFG does not model.
#[test]
fn a_predecessor_outside_the_cfg_walk_blocks_forwarding() {
    let mut stream = vec![
        ci(
            "mov_imm",
            &[("dst", "%v1"), ("type", "Integer"), ("value", "5")],
        ),
        store("%v1", "8"),
        ci("b", &[("target", "join")]),
        ci("label", &[("name", "dead")]),
        ci("b", &[("target", "join")]),
        ci("label", &[("name", "join")]),
        load("%v2", "8"),
        store("%v2", "16"),
        ci("ret", &[]),
    ];
    run(&mut stream);
    assert_eq!(stream[6].op, CodeOp::LdrU64);
}

/// A block with a predecessor outside the CFG walk has no facts on entry,
/// but it still runs: its own stores must reach its successors. Here `j` is
/// entered from the entry block and from the dead block, and rewrites the
/// slot with `%v3`; the join `s` must see that, not just the entry block's
/// `%v1`.
#[test]
fn a_block_entered_from_outside_the_cfg_walk_still_passes_its_stores_on() {
    let mut stream = vec![
        ci(
            "mov_imm",
            &[("dst", "%v1"), ("type", "Integer"), ("value", "5")],
        ),
        store("%v1", "8"),
        ci("cmp_imm", &[("lhs", "%v9"), ("rhs", "0")]),
        ci("b.eq", &[("target", "s")]),
        ci("label", &[("name", "j")]),
        ci(
            "mov_imm",
            &[("dst", "%v3"), ("type", "Integer"), ("value", "7")],
        ),
        store("%v3", "8"),
        ci("label", &[("name", "s")]),
        load("%v2", "8"),
        store("%v2", "16"),
        ci("ret", &[]),
        ci("label", &[("name", "dead")]),
        ci("b", &[("target", "j")]),
    ];
    run(&mut stream);
    assert_eq!(stream[8].op, CodeOp::LdrU64, "the slot is %v1 or %v3 at s");
}

/// The three-way join the partial-load row is built for: two predecessors
/// leave the slot in the same single-def register (`%v1`, loaded in a block
/// that cannot reach the third), the third lacks it and falls straight into
/// the join. The load is placed on the gap edge and the join's own load
/// becomes a copy of `%v1`.
#[test]
fn a_load_available_on_all_but_one_of_three_edges_is_completed() {
    let mut stream = vec![
        ci("cmp_imm", &[("lhs", "%v8"), ("rhs", "0")]),
        ci("b.eq", &[("target", "gap")]),
        load("%v1", "8"),
        ci("cmp_imm", &[("lhs", "%v9"), ("rhs", "0")]),
        ci("b.eq", &[("target", "second")]),
        ci("b", &[("target", "join")]),
        ci("label", &[("name", "second")]),
        ci("b", &[("target", "join")]),
        ci("label", &[("name", "gap")]),
        ci("bl", &[("target", "_mfb_fn_callee")]),
        ci("b", &[("target", "join")]),
        ci("label", &[("name", "join")]),
        load("%v2", "8"),
        store("%v2", "16"),
        ci("ret", &[]),
    ];
    run(&mut stream);
    assert_eq!(
        writers_of(&stream, "%v1"),
        vec![2, 10],
        "the gap edge reloads into %v1 just before its branch"
    );
    assert_eq!(stream[10].op, CodeOp::LdrU64);
    assert_eq!(stream[13].op, CodeOp::Mov, "the join's load became a copy");
    assert_eq!(stream[13].get("src").as_deref(), Some("%v1"));
}

/// The partial-load row's refusals, one per guard: two predecessors holding
/// DIFFERENT values, two predecessors lacking the value (completing both
/// would add a load, not move one), and a holder with two definitions.
#[test]
fn a_partial_load_needs_one_gap_and_one_single_def_holder() {
    // Two different values reach the join (`%v1` and `%v3`); the third edge
    // has none.
    let disagree = vec![
        ci("cmp_imm", &[("lhs", "%v8"), ("rhs", "0")]),
        ci("b.eq", &[("target", "gap")]),
        ci("cmp_imm", &[("lhs", "%v9"), ("rhs", "0")]),
        ci("b.eq", &[("target", "second")]),
        load("%v1", "8"),
        ci("b", &[("target", "join")]),
        ci("label", &[("name", "second")]),
        load("%v3", "8"),
        ci("b", &[("target", "join")]),
        ci("label", &[("name", "gap")]),
        ci("bl", &[("target", "_mfb_fn_callee")]),
        ci("b", &[("target", "join")]),
        ci("label", &[("name", "join")]),
        load("%v2", "8"),
        store("%v2", "16"),
        ci("ret", &[]),
    ];
    // One edge has `%v1`; the other two both lost the slot to a call.
    let two_gaps = vec![
        ci("cmp_imm", &[("lhs", "%v8"), ("rhs", "0")]),
        ci("b.eq", &[("target", "gap")]),
        ci("cmp_imm", &[("lhs", "%v9"), ("rhs", "0")]),
        ci("b.eq", &[("target", "second")]),
        load("%v1", "8"),
        ci("b", &[("target", "join")]),
        ci("label", &[("name", "second")]),
        ci("bl", &[("target", "_mfb_fn_callee")]),
        ci("b", &[("target", "join")]),
        ci("label", &[("name", "gap")]),
        ci("bl", &[("target", "_mfb_fn_callee")]),
        ci("b", &[("target", "join")]),
        ci("label", &[("name", "join")]),
        load("%v2", "8"),
        store("%v2", "16"),
        ci("ret", &[]),
    ];
    // `%v1` holds the slot on one edge but is defined twice in the function.
    let multi_def = vec![
        ci("cmp_imm", &[("lhs", "%v9"), ("rhs", "0")]),
        ci("b.eq", &[("target", "gap")]),
        load("%v1", "8"),
        ci("b", &[("target", "join")]),
        ci("label", &[("name", "gap")]),
        ci("bl", &[("target", "_mfb_fn_callee")]),
        ci("b", &[("target", "join")]),
        ci("label", &[("name", "join")]),
        load("%v2", "8"),
        store("%v2", "16"),
        ci(
            "mov_imm",
            &[("dst", "%v1"), ("type", "Integer"), ("value", "0")],
        ),
        store("%v1", "24"),
        ci("ret", &[]),
    ];
    for (name, mut stream) in [
        ("disagree", disagree),
        ("two_gaps", two_gaps),
        ("multi_def", multi_def),
    ] {
        let before = stream.len();
        run(&mut stream);
        assert_eq!(stream.len(), before, "{name}: no load was placed");
        let join_load = stream
            .iter()
            .position(|inst| inst.get("dst").as_deref() == Some("%v2"))
            .expect("the join still defines %v2");
        assert_eq!(stream[join_load].op, CodeOp::LdrU64, "{name}");
    }
}

/// What the dataflow does not track ends availability instead of being
/// guessed at: a load into a physical register is no candidate and leaves the
/// slot unknown, and an `sp` access whose offset is not a plain number is a
/// barrier for every slot.
#[test]
fn untrackable_slot_accesses_end_availability() {
    let physical_dst = vec![
        ci(
            "mov_imm",
            &[("dst", "%v1"), ("type", "Integer"), ("value", "5")],
        ),
        store("%v1", "8"),
        load("x0", "8"),
        load("%v2", "8"),
        store("%v2", "16"),
        ci("ret", &[]),
    ];
    let symbolic_load = vec![
        ci(
            "mov_imm",
            &[("dst", "%v1"), ("type", "Integer"), ("value", "5")],
        ),
        store("%v1", "8"),
        load("%v5", "#frame"),
        load("%v2", "8"),
        store("%v2", "16"),
        ci("ret", &[]),
    ];
    let symbolic_store = vec![
        ci(
            "mov_imm",
            &[("dst", "%v1"), ("type", "Integer"), ("value", "5")],
        ),
        store("%v1", "8"),
        store("%v1", "#frame"),
        load("%v2", "8"),
        store("%v2", "16"),
        ci("ret", &[]),
    ];
    for (name, mut stream) in [
        ("physical_dst", physical_dst),
        ("symbolic_load", symbolic_load),
        ("symbolic_store", symbolic_store),
    ] {
        let untracked = stream[2].op;
        run(&mut stream);
        assert_eq!(stream[2].op, untracked, "{name}: left as written");
        assert_eq!(stream[3].op, CodeOp::LdrU64, "{name}: %v2 still loads");
    }
}
