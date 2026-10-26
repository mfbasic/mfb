//! plan-171-B: final-stream register liveness leans live at every boundary the
//! allocator's own liveness does not model.

use super::{analyze, index_of};
use crate::arch::aarch64::regmodel::Aarch64RegisterModel;
use crate::codegen::engine::types::CodeInstruction;

fn mov_imm(dst: &str, value: &str) -> CodeInstruction {
    CodeInstruction::new("mov_imm")
        .physical_field("dst", dst)
        .field("value", value)
}

fn bit(name: &str) -> u64 {
    1 << index_of(name).expect("a register")
}

fn live_after(stream: &[CodeInstruction], at: usize, register: &str) -> bool {
    analyze(stream, &Aarch64RegisterModel, false).live_out[at] & bit(register) != 0
}

#[test]
fn a_register_overwritten_before_any_read_is_dead() {
    let stream = vec![
        mov_imm("x9", "1"),
        mov_imm("x9", "2"),
        CodeInstruction::new("ret"),
    ];
    assert!(!live_after(&stream, 0, "x9"));
    assert!(live_after(&stream, 1, "x9"), "ret reads every register");
}

/// A compiled function reads its argument bank, not the allocatable pool.
#[test]
fn a_compiled_call_reads_arguments_but_not_the_pool() {
    let stream = vec![
        mov_imm("x0", "1"),
        mov_imm("x9", "1"),
        CodeInstruction::new("bl").field("target", "_mfb_fn_g"),
        mov_imm("x0", "0"),
        mov_imm("x9", "0"),
        CodeInstruction::new("ret"),
    ];
    assert!(live_after(&stream, 1, "x0"));
    assert!(!live_after(&stream, 1, "x9"));
}

/// A runtime helper or C function may read anything; so may `blr` and `svc`.
#[test]
fn any_other_call_reads_every_register() {
    for call in [
        CodeInstruction::new("bl").field("target", "_mfb_rt_helper"),
        CodeInstruction::new("bl").field("target", "_write"),
        CodeInstruction::new("blr").physical_field("register", "x16"),
        CodeInstruction::new("svc"),
    ] {
        let stream = vec![
            mov_imm("x9", "1"),
            call,
            mov_imm("x9", "0"),
            CodeInstruction::new("ret"),
        ];
        assert!(live_after(&stream, 0, "x9"), "{:?}", stream[1].op);
    }
}

/// A branch out of the function, and an unrealized token, are unknown readers.
#[test]
fn unknown_code_and_unknown_operands_read_everything() {
    let tail = vec![
        mov_imm("x9", "1"),
        CodeInstruction::new("b").field("target", "_elsewhere"),
    ];
    assert!(live_after(&tail, 0, "x9"));
    let token = vec![
        mov_imm("x9", "1"),
        CodeInstruction::new("mov")
            .physical_field("dst", "x10")
            .physical_field("src", "%scratch3"),
        mov_imm("x9", "0"),
        CodeInstruction::new("ret"),
    ];
    assert!(live_after(&token, 0, "x9"));
}

/// Both edges of a conditional branch count.
#[test]
fn a_read_on_either_edge_keeps_a_register_live() {
    let stream = vec![
        mov_imm("x9", "1"),
        CodeInstruction::new("cbz")
            .physical_field("register", "x10")
            .field("target", "a"),
        mov_imm("x9", "0"),
        CodeInstruction::new("ret"),
        CodeInstruction::new("label").field("name", "a"),
        CodeInstruction::new("mov")
            .physical_field("dst", "x0")
            .physical_field("src", "x9"),
        CodeInstruction::new("ret"),
    ];
    assert!(live_after(&stream, 1, "x9"));
    assert!(live_after(&stream, 0, "x10"));
}

/// A compiled function's callers treat x8–x14 as destroyed: its `ret` does not
/// read them, but still reads results and callee-saved registers.
#[test]
fn a_compiled_function_s_return_does_not_read_the_caller_saved_pool() {
    let stream = vec![
        mov_imm("x9", "1"),
        mov_imm("x21", "1"),
        mov_imm("x1", "1"),
        CodeInstruction::new("ret"),
    ];
    let live = analyze(&stream, &Aarch64RegisterModel, true).live_out[2];
    assert_eq!(live & bit("x9"), 0);
    assert_ne!(live & bit("x21"), 0);
    assert_ne!(live & bit("x1"), 0);
    let hand_written = analyze(&stream, &Aarch64RegisterModel, false).live_out[2];
    assert_ne!(hand_written & bit("x9"), 0);
}

/// `lr` in an epilogue `ldp` is `x30`, not an unknown operand that reads
/// everything (which kept every register live into a compiled `ret`).
#[test]
fn lr_is_x30_not_an_unknown_operand() {
    let stream = vec![
        mov_imm("x9", "1"),
        CodeInstruction::new("ldp")
            .physical_field("dst", "x25")
            .physical_field("dst2", "lr")
            .physical_field("base", "sp")
            .field("offset", "16")
            .field("addressing", "offset")
            .field("width", "8"),
        CodeInstruction::new("ret"),
    ];
    let liveness = analyze(&stream, &Aarch64RegisterModel, true);
    assert_eq!(liveness.live_out[0] & bit("x9"), 0);
    assert_eq!(liveness.defs[1] & bit("lr"), bit("x30"));
    assert!(!liveness.opaque[1]);
}
