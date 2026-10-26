//! plan-171-A: the AArch64 save area is written with `stp` and read back with
//! `ldp` for every two adjacent same-kind slots, and a small frame whose first
//! pair is at its bottom moves `sp` with that pair. The slots — and so the frame
//! layout — never change; x86-64 and riscv64 keep one access per register.

use super::*;
use crate::codegen::engine::mir;

/// One instruction as `op field=value …`, in field order.
fn show(instruction: &CodeInstruction) -> String {
    let mut text = instruction.op.mnemonic().to_string();
    for (name, value) in &instruction.fields {
        text.push_str(&format!(" {name}={}", value.rendered()));
    }
    text
}

fn label() -> CodeInstruction {
    CodeInstruction::new("label").field("name", "entry")
}

fn call() -> CodeInstruction {
    CodeInstruction::new("bl").field("target", "_callee")
}

fn ret() -> CodeInstruction {
    CodeInstruction::new("ret")
}

/// Finalize `body` (between the entry label and one `ret` per `rets`) on
/// `backend`, saving `saved` (the link register is added when the body calls),
/// and return the frame size and the finalized stream, label and calls elided.
fn frame(
    backend: &'static dyn mir::Backend,
    saved: &[&str],
    locals: usize,
    body: Vec<CodeInstruction>,
    rets: usize,
) -> (usize, Vec<String>) {
    mir::set_backend(backend);
    let mut instructions = vec![label()];
    instructions.extend(body);
    for _ in 0..rets {
        instructions.push(ret());
    }
    let callee_saved = saved.iter().map(|r| r.to_string()).collect();
    let frame = finalize_frame(&mut instructions, &mut [], locals, callee_saved);
    let stream = instructions
        .iter()
        .filter(|i| !matches!(i.op, CodeOp::Label | CodeOp::BranchLink))
        .map(show)
        .collect();
    (frame.stack_size, stream)
}

fn aarch64(saved: &[&str], locals: usize, calls: bool) -> (usize, Vec<String>) {
    let body = if calls { vec![call()] } else { Vec::new() };
    frame(
        &crate::arch::aarch64::backend::AARCH64_BACKEND,
        saved,
        locals,
        body,
        1,
    )
}

fn stp(a: &str, b: &str, offset: i64, mode: &str, width: usize) -> String {
    format!("stp src={a} src2={b} base=sp offset={offset} addressing={mode} width={width}")
}

fn ldp(a: &str, b: &str, offset: i64, mode: &str, width: usize) -> String {
    format!("ldp dst={a} dst2={b} base=sp offset={offset} addressing={mode} width={width}")
}

const RET: &str = "ret";

#[test]
fn no_save_keeps_the_plain_allocation_and_no_frame_has_no_prologue() {
    assert_eq!(aarch64(&[], 0, false), (0, vec![RET.to_string()]));
    assert_eq!(
        aarch64(&[], 16, false),
        (
            16,
            vec!["sub_sp imm=16".into(), "add_sp imm=16".into(), RET.into()]
        )
    );
}

#[test]
fn one_save_is_a_single_store_and_load() {
    // A call saves only the link register: an odd save area of one slot.
    assert_eq!(
        aarch64(&[], 0, true).1,
        [
            "sub_sp imm=16",
            "str_u64 src=lr base=sp offset=0",
            "ldr_u64 dst=lr base=sp offset=0",
            "add_sp imm=16",
            RET
        ]
    );
}

#[test]
fn two_saves_at_the_bottom_of_a_small_frame_fold_the_allocation() {
    assert_eq!(
        aarch64(&["x21"], 0, true),
        (
            16,
            vec![
                stp("x21", "lr", -16, "pre", 8),
                ldp("x21", "lr", 16, "post", 8),
                RET.into()
            ]
        )
    );
}

#[test]
fn three_saves_pair_the_first_two_and_keep_the_odd_tail_single() {
    assert_eq!(
        aarch64(&["x21", "x22"], 0, true).1,
        [
            stp("x21", "x22", -32, "pre", 8),
            "str_u64 src=lr base=sp offset=16".into(),
            "ldr_u64 dst=lr base=sp offset=16".into(),
            ldp("x21", "x22", 32, "post", 8),
            RET.to_string()
        ]
    );
}

#[test]
fn a_q_slot_breaks_an_integer_run_and_q_slots_pair_with_each_other() {
    // x21 at 0, d8 at 16 (16-aligned, 16 wide), x22 at 32: no two adjacent of a kind.
    let (_, mixed) = aarch64(&["x21", "d8", "x22"], 0, false);
    assert_eq!(
        mixed[..4],
        [
            "sub_sp imm=48",
            "str_u64 src=x21 base=sp offset=0",
            "str_q src=d8 base=sp offset=16",
            "str_u64 src=x22 base=sp offset=32",
        ]
    );
    let (_, both) = aarch64(&["x21", "x22", "d8", "d9"], 0, false);
    assert_eq!(
        both,
        [
            stp("x21", "x22", -48, "pre", 8),
            stp("d8", "d9", 16, "offset", 16),
            ldp("d8", "d9", 16, "offset", 16),
            ldp("x21", "x22", 48, "post", 8),
            RET.into()
        ]
    );
}

#[test]
fn an_outgoing_argument_tail_keeps_the_allocation_and_pairs_above_it() {
    let body = vec![abi::outgoing_stack_arg_store(abi::ZERO, 0), call()];
    let (size, stream) = frame(
        &crate::arch::aarch64::backend::AARCH64_BACKEND,
        &["x21"],
        0,
        body,
        1,
    );
    assert_eq!(size, 32);
    assert_eq!(
        stream[..2],
        [
            "sub_sp imm=32".to_string(),
            stp("x21", "lr", 16, "offset", 8)
        ]
    );
    assert_eq!(
        stream[3..],
        [
            ldp("x21", "lr", 16, "offset", 8),
            "add_sp imm=32".into(),
            RET.into()
        ]
    );
}

#[test]
fn the_fold_stops_at_the_largest_frame_the_post_index_reaches() {
    // 16 bytes of saves + 480 of locals = 496: folded.
    let (size, folded) = aarch64(&["x21"], 480, true);
    assert_eq!(size, 496);
    assert_eq!(folded[0], stp("x21", "lr", -496, "pre", 8));
    assert_eq!(folded[1], ldp("x21", "lr", 496, "post", 8));
    // 512 is past +504: allocated and freed on its own, the pair at offset 0.
    let (size, plain) = aarch64(&["x21"], 496, true);
    assert_eq!(size, 512);
    assert_eq!(
        plain,
        [
            "sub_sp imm=512".to_string(),
            stp("x21", "lr", 0, "offset", 8),
            ldp("x21", "lr", 0, "offset", 8),
            "add_sp imm=512".into(),
            RET.into()
        ]
    );
}

#[test]
fn a_pair_past_the_imm7_reach_is_two_single_accesses() {
    // An outgoing tail of 496 bytes puts the saves at 496, 504, 512, 520, 528:
    // the first pair starts at 496 (in reach), the second would start at 512.
    let body = vec![abi::outgoing_stack_arg_store(abi::ZERO, 60), call()];
    let (_, stream) = frame(
        &crate::arch::aarch64::backend::AARCH64_BACKEND,
        &["x21", "x22", "x23", "x24"],
        0,
        body,
        1,
    );
    assert_eq!(
        stream[1..5],
        [
            stp("x21", "x22", 496, "offset", 8),
            "str_u64 src=x23 base=sp offset=512".into(),
            "str_u64 src=x24 base=sp offset=520".into(),
            "str_u64 src=lr base=sp offset=528".into(),
        ]
    );
}

#[test]
fn every_return_gets_the_paired_epilogue() {
    let (_, stream) = frame(
        &crate::arch::aarch64::backend::AARCH64_BACKEND,
        &["x21", "x22", "x23"],
        0,
        vec![call()],
        2,
    );
    let epilogue = [
        ldp("x23", "lr", 16, "offset", 8),
        ldp("x21", "x22", 32, "post", 8),
        RET.into(),
    ];
    assert_eq!(stream[2..5], epilogue);
    assert_eq!(stream[5..], epilogue);
}

#[test]
fn a_backend_without_pair_access_keeps_one_access_per_register() {
    let (_, x86) = frame(
        &crate::arch::x86_64::backend::X86_64_BACKEND,
        &["r12", "r14"],
        0,
        Vec::new(),
        1,
    );
    assert!(
        x86.iter()
            .all(|i| !i.starts_with("stp") && !i.starts_with("ldp")),
        "{x86:?}"
    );
    assert_eq!(x86[0], "sub_sp imm=16");
    let (_, rv) = frame(
        &crate::arch::riscv64::backend::RISCV64_BACKEND,
        &["s1", "s2"],
        0,
        Vec::new(),
        1,
    );
    assert!(
        rv.iter()
            .all(|i| !i.starts_with("stp") && !i.starts_with("ldp")),
        "{rv:?}"
    );
    assert_eq!(rv[0], "sub_sp imm=16");
}
