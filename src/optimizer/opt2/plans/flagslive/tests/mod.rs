//! plan-171-B Phase 3: the flag-liveness cases §4.2 names.

use super::dead_after;
use crate::codegen::engine::types::CodeInstruction;

fn label(name: &str) -> CodeInstruction {
    CodeInstruction::new("label").field("name", name)
}

fn branch(op: &str, target: &str) -> CodeInstruction {
    CodeInstruction::new(op).field("target", target)
}

fn cmp0(register: &str) -> CodeInstruction {
    CodeInstruction::new("cmp_imm")
        .physical_field("lhs", register)
        .field("rhs", "0")
}

fn ret() -> CodeInstruction {
    CodeInstruction::new("ret")
}

/// `cmp; b.eq A; b.lt B` — the second branch reads the first one's flags (a
/// `share` chain), so they are live after the `b.eq`.
#[test]
fn a_share_chain_keeps_the_flags_live() {
    let stream = vec![
        cmp0("x9"),
        branch("b.eq", "a"),
        branch("b.lt", "b"),
        ret(),
        label("a"),
        ret(),
        label("b"),
        ret(),
    ];
    let dead = dead_after(&stream);
    assert!(!dead[1], "b.lt reads what b.eq read");
    assert!(dead[2], "nothing reads after the last branch");
}

/// A dead pair: each successor sets its own flags before reading or returns.
#[test]
fn flags_reset_or_unread_on_both_edges_are_dead() {
    let stream = vec![
        cmp0("x9"),
        branch("b.eq", "a"),
        cmp0("x10"),
        branch("b.ne", "b"),
        ret(),
        label("a"),
        ret(),
        label("b"),
        ret(),
    ];
    assert!(dead_after(&stream)[1]);
}

/// A reader at the branch target, across a label, keeps them live.
#[test]
fn a_reader_across_a_label_keeps_the_flags_live() {
    let stream = vec![
        cmp0("x9"),
        branch("b.eq", "a"),
        ret(),
        label("a"),
        branch("b.lt", "b"),
        ret(),
        label("b"),
        ret(),
    ];
    assert!(!dead_after(&stream)[1]);
}

/// A setter on the fall-through path only: the taken path still reads.
#[test]
fn a_setter_on_one_path_only_keeps_the_flags_live() {
    let stream = vec![
        cmp0("x9"),
        branch("b.eq", "a"),
        cmp0("x10"),
        branch("b.ne", "b"),
        ret(),
        label("a"),
        branch("b.ge", "b"),
        ret(),
        label("b"),
        ret(),
    ];
    assert!(!dead_after(&stream)[1]);
}

/// A call is transparent: a reader after it still reads the compare's flags.
#[test]
fn a_call_between_is_transparent() {
    let stream = vec![
        cmp0("x9"),
        branch("b.eq", "a"),
        CodeInstruction::new("bl").field("target", "_mfb_fn_g"),
        branch("b.ne", "a"),
        ret(),
        label("a"),
        ret(),
    ];
    assert!(!dead_after(&stream)[1]);
}

/// A back edge carries a reader at the loop head back to the branch.
#[test]
fn a_back_edge_carries_a_reader_round_the_loop() {
    let stream = vec![
        label("top"),
        branch("b.lt", "out"),
        CodeInstruction::new("add_imm")
            .physical_field("dst", "x9")
            .physical_field("src", "x9")
            .field("imm", "1"),
        cmp0("x9"),
        branch("b.ne", "top"),
        label("out"),
        ret(),
    ];
    let dead = dead_after(&stream);
    assert!(!dead[4], "the loop head's b.lt reads the bottom compare");
    // Without the head reader the same loop leaves them dead.
    let mut no_reader = stream.clone();
    no_reader.remove(1);
    assert!(dead_after(&no_reader)[3]);
}

/// A branch out of the function, or falling off its end, is unknown code.
#[test]
fn leaving_the_function_keeps_the_flags_live() {
    let tail = vec![cmp0("x9"), branch("b.eq", "elsewhere"), ret()];
    assert!(!dead_after(&tail)[1]);
    let off_the_end = vec![cmp0("x9"), branch("b.eq", "a"), label("a")];
    assert!(!dead_after(&off_the_end)[1]);
}

/// The x86-64 float branches read flags too.
#[test]
fn the_x86_float_branches_are_readers() {
    let stream = vec![
        cmp0("r10"),
        branch("b.eq", "a"),
        branch("x86.jp", "a"),
        ret(),
        label("a"),
        ret(),
    ];
    assert!(!dead_after(&stream)[1]);
}
