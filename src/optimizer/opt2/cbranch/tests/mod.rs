//! plan-171-B Phase 4: the compare-and-branch row, per backend.

use super::{form, Formed};
use crate::arch::aarch64::regmodel::Aarch64RegisterModel;
use crate::arch::ops::CodeOp;
use crate::arch::riscv64::regmodel::Riscv64RegisterModel;
use crate::arch::x86_64::regmodel::X86_64RegisterModel;
use crate::codegen::engine::mir::CompareBranchForms;
use crate::codegen::engine::types::CodeInstruction;
use crate::target::shared::regmodel::RegisterModel;

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
fn cmp(lhs: &str, rhs: &str) -> CodeInstruction {
    CodeInstruction::new("cmp")
        .physical_field("lhs", lhs)
        .physical_field("rhs", rhs)
}
fn mov_imm(dst: &str, value: &str) -> CodeInstruction {
    CodeInstruction::new("mov_imm")
        .physical_field("dst", dst)
        .field("type", "Integer")
        .field("value", value)
}
fn and(dst: &str, lhs: &str, rhs: &str) -> CodeInstruction {
    CodeInstruction::new("and")
        .physical_field("dst", dst)
        .physical_field("lhs", lhs)
        .physical_field("rhs", rhs)
}
fn mov(dst: &str, src: &str) -> CodeInstruction {
    CodeInstruction::new("mov")
        .physical_field("dst", dst)
        .physical_field("src", src)
}
fn ret() -> CodeInstruction {
    CodeInstruction::new("ret")
}
fn rv_br(lhs: &str, rhs: &str, cond: &str, target: &str) -> CodeInstruction {
    CodeInstruction::new("rv.br")
        .physical_field("lhs", lhs)
        .physical_field("rhs", rhs)
        .field("cond", cond)
        .field("target", target)
}

/// Each successor overwrites the scratch registers before returning, so
/// `x9`–`x11` (and the flags) are dead after the branch.
fn tail(scratch: &[&str]) -> Vec<CodeInstruction> {
    let mut out = Vec::new();
    for register in scratch {
        out.push(mov_imm(register, "0"));
    }
    out.push(ret());
    out.push(label("l"));
    for register in scratch {
        out.push(mov_imm(register, "0"));
    }
    out.push(ret());
    out
}

fn run(
    mut stream: Vec<CodeInstruction>,
    forms: CompareBranchForms,
    model: &dyn RegisterModel,
) -> (Vec<CodeInstruction>, Formed) {
    let formed = form(&mut stream, forms, model, false);
    (stream, formed)
}

fn spelled(stream: &[CodeInstruction]) -> Vec<String> {
    stream
        .iter()
        .map(|i| {
            let fields: Vec<String> = i
                .fields
                .iter()
                .map(|(k, v)| format!("{k}={}", v.rendered()))
                .collect();
            format!("{} {}", i.op.mnemonic(), fields.join(" "))
        })
        .collect()
}

#[test]
fn aarch64_compare_with_zero_becomes_cbz_cbnz_and_sign_tests() {
    for (op, formed) in [
        ("b.eq", "cbz register=x9 target=l"),
        ("b.ne", "cbnz register=x9 target=l"),
        ("b.lt", "tbnz register=x9 bit=63 target=l"),
        ("b.ge", "tbz register=x9 bit=63 target=l"),
    ] {
        let mut stream = vec![cmp0("x9"), branch(op, "l")];
        stream.extend(tail(&[]));
        let (out, counts) = run(stream, CompareBranchForms::Aarch64, &Aarch64RegisterModel);
        assert_eq!(spelled(&out)[0], formed, "{op}");
        assert_eq!(counts.zero_branches, 1);
    }
}

#[test]
fn live_flags_keep_the_compare() {
    // A share chain: `b.lt` reads the flags after `b.eq`.
    let mut stream = vec![cmp0("x9"), branch("b.eq", "l"), branch("b.lt", "l")];
    stream.extend(tail(&[]));
    let (out, counts) = run(stream, CompareBranchForms::Aarch64, &Aarch64RegisterModel);
    assert_eq!(counts, Formed::default());
    assert_eq!(out[0].op, CodeOp::CmpImm);
    assert_eq!(out[1].op, CodeOp::BranchEq);
    assert_eq!(out[2].op, CodeOp::BranchLt);
}

#[test]
fn a_zero_in_a_register_becomes_the_immediate_and_its_move_goes() {
    let mut stream = vec![mov_imm("x10", "0"), cmp("x9", "x10"), branch("b.ne", "l")];
    stream.extend(tail(&["x10"]));
    let (out, counts) = run(stream, CompareBranchForms::Aarch64, &Aarch64RegisterModel);
    assert_eq!(spelled(&out)[0], "cbnz register=x9 target=l");
    assert_eq!(counts.zero_compares, 1);
    // x86-64: the compare becomes `cmp_imm r, 0` (encoded `test r, r`).
    let mut stream = vec![mov_imm("r12", "0"), cmp("r11", "r12"), branch("b.ne", "l")];
    stream.extend(tail(&["r12"]));
    let (out, _) = run(stream, CompareBranchForms::X86, &X86_64RegisterModel);
    assert_eq!(spelled(&out)[0], "cmp_imm lhs=r11 rhs=0");
    assert_eq!(out[1].op, CodeOp::BranchNe);
}

#[test]
fn a_live_zero_register_keeps_its_move() {
    let stream = vec![
        mov_imm("x10", "0"),
        cmp("x9", "x10"),
        branch("b.ne", "l"),
        mov("x0", "x10"),
        ret(),
        label("l"),
        ret(),
    ];
    let (out, _) = run(stream, CompareBranchForms::Aarch64, &Aarch64RegisterModel);
    assert_eq!(spelled(&out)[0], "mov_imm dst=x10 type=Integer value=0");
    assert_eq!(spelled(&out)[1], "cbnz register=x9 target=l");
}

#[test]
fn a_zero_on_the_left_needs_an_equality_branch() {
    let mut stream = vec![mov_imm("x10", "0"), cmp("x10", "x9"), branch("b.lt", "l")];
    stream.extend(tail(&["x10"]));
    let (out, counts) = run(
        stream.clone(),
        CompareBranchForms::Aarch64,
        &Aarch64RegisterModel,
    );
    assert_eq!(counts, Formed::default(), "0 < r is not r < 0");
    assert_eq!(out.len(), stream.len());
    stream[2] = branch("b.eq", "l");
    let (out, _) = run(stream, CompareBranchForms::Aarch64, &Aarch64RegisterModel);
    assert_eq!(spelled(&out)[0], "cbz register=x9 target=l");
}

#[test]
fn a_single_bit_test_becomes_tbz_on_every_target() {
    let shape = |m: &str, t: &str, r: &str, zero: &str| {
        vec![
            mov_imm(m, "1024"),
            and(t, r, m),
            mov_imm(zero, "0"),
            cmp(t, zero),
        ]
    };
    let mut stream = shape("x10", "x11", "x9", "x12");
    stream.push(branch("b.ne", "l"));
    stream.extend(tail(&["x10", "x11", "x12"]));
    let (out, counts) = run(stream, CompareBranchForms::Aarch64, &Aarch64RegisterModel);
    assert_eq!(spelled(&out)[0], "tbnz register=x9 bit=10 target=l");
    assert_eq!(counts.bit_tests, 1);
    let mut stream = vec![
        mov_imm("r12", "2147483648"),
        and("r14", "r12", "r11"),
        cmp0("r14"),
    ];
    stream.push(branch("b.eq", "l"));
    stream.extend(tail(&["r12", "r14"]));
    let (out, _) = run(stream, CompareBranchForms::X86, &X86_64RegisterModel);
    assert_eq!(spelled(&out)[0], "tbz register=r11 bit=31 target=l");
    let mut stream = vec![
        mov_imm("t5", "2048"),
        and("t6", "t4", "t5"),
        mov_imm("t3", "0"),
        rv_br("t6", "t3", "ne", "l"),
    ];
    stream.extend(tail(&["t3", "t5", "t6"]));
    let (out, counts) = run(stream, CompareBranchForms::Riscv64, &Riscv64RegisterModel);
    assert_eq!(
        spelled(&out)[0],
        "tbnz register=t4 bit=11 scratch=t6 target=l"
    );
    assert_eq!((counts.bit_tests, counts.zero_compares), (1, 1));
}

#[test]
fn a_live_masked_value_keeps_the_and() {
    let stream = vec![
        mov_imm("x10", "8"),
        and("x11", "x9", "x10"),
        cmp0("x11"),
        branch("b.eq", "l"),
        mov("x0", "x11"),
        ret(),
        label("l"),
        ret(),
    ];
    let (out, counts) = run(stream, CompareBranchForms::Aarch64, &Aarch64RegisterModel);
    assert_eq!(counts.bit_tests, 0);
    assert_eq!(out[1].op, CodeOp::And);
    assert_eq!(spelled(&out)[2], "cbz register=x11 target=l");
}

#[test]
fn a_mask_that_stays_live_keeps_its_move() {
    let stream = vec![
        mov_imm("x10", "8"),
        and("x11", "x9", "x10"),
        cmp0("x11"),
        branch("b.eq", "l"),
        mov_imm("x11", "0"),
        mov("x0", "x10"),
        ret(),
        label("l"),
        mov_imm("x11", "0"),
        ret(),
    ];
    let (out, counts) = run(stream, CompareBranchForms::Aarch64, &Aarch64RegisterModel);
    assert_eq!(counts.bit_tests, 1);
    assert_eq!(spelled(&out)[0], "mov_imm dst=x10 type=Integer value=8");
    assert_eq!(spelled(&out)[1], "tbz register=x9 bit=3 target=l");
}

#[test]
fn a_call_reading_the_scratch_keeps_it() {
    // A runtime helper may read any register: `x11` stays live across `bl`.
    let stream = vec![
        mov_imm("x10", "8"),
        and("x11", "x9", "x10"),
        cmp0("x11"),
        branch("b.eq", "l"),
        CodeInstruction::new("bl").field("target", "_mfb_rt_helper"),
        ret(),
        label("l"),
        ret(),
    ];
    let (_, counts) = run(stream, CompareBranchForms::Aarch64, &Aarch64RegisterModel);
    assert_eq!(counts.bit_tests, 0);
}

#[test]
fn a_constant_across_a_label_or_a_redefinition_is_unknown() {
    let mut across = vec![
        mov_imm("x10", "0"),
        label("m"),
        cmp("x9", "x10"),
        branch("b.ne", "l"),
    ];
    across.extend(tail(&["x10"]));
    let (out, _) = run(across, CompareBranchForms::Aarch64, &Aarch64RegisterModel);
    assert_eq!(out[2].op, CodeOp::Cmp);
    let mut redefined = vec![
        mov_imm("x10", "0"),
        mov("x10", "x8"),
        cmp("x9", "x10"),
        branch("b.ne", "l"),
    ];
    redefined.extend(tail(&["x10"]));
    let (out, _) = run(
        redefined,
        CompareBranchForms::Aarch64,
        &Aarch64RegisterModel,
    );
    assert_eq!(out[2].op, CodeOp::Cmp);
}

#[test]
fn x86_forms_no_cbz_and_riscv_zeroes_a_register_operand() {
    let mut stream = vec![cmp0("r11"), branch("b.eq", "l")];
    stream.extend(tail(&[]));
    let (out, counts) = run(stream, CompareBranchForms::X86, &X86_64RegisterModel);
    assert_eq!(counts, Formed::default());
    assert_eq!(out[0].op, CodeOp::CmpImm);
    let mut stream = vec![mov_imm("t5", "0"), rv_br("t4", "t5", "lt", "l")];
    stream.extend(tail(&["t5"]));
    let (out, _) = run(stream, CompareBranchForms::Riscv64, &Riscv64RegisterModel);
    assert_eq!(spelled(&out)[0], "rv.br lhs=t4 rhs=zero cond=lt target=l");
}

/// A Boolean `false` is a zero (`IF flag THEN` compares with it).
#[test]
fn a_false_boolean_is_a_known_zero() {
    let mut stream = vec![
        CodeInstruction::new("mov_imm")
            .physical_field("dst", "x10")
            .field("type", "Boolean")
            .field("value", "false"),
        cmp("x9", "x10"),
        branch("b.ne", "l"),
    ];
    stream.extend(tail(&["x10"]));
    let (out, _) = run(stream, CompareBranchForms::Aarch64, &Aarch64RegisterModel);
    assert_eq!(spelled(&out)[0], "cbnz register=x9 target=l");
}
