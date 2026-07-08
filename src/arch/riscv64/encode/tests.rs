use super::*;
use crate::target::shared::code::{CodeFrame, CodeFunction, NativeCodePlan};
use crate::target::NativeBuildMode;

/// Encode a single function's instructions and return its text bytes.
fn encode_text(instructions: Vec<CodeInstruction>) -> Vec<u8> {
    let plan = NativeCodePlan {
        target: "linux-riscv64".to_string(),
        build_mode: NativeBuildMode::Console,
        arch: "riscv64".to_string(),
        project: "t".to_string(),
        entry_symbol: Some("f".to_string()),
        imports: Vec::new(),
        data_objects: Vec::new(),
        functions: vec![CodeFunction {
            name: "f".to_string(),
            symbol: "f".to_string(),
            params: Vec::new(),
            returns: "Integer".to_string(),
            frame: CodeFrame {
                stack_size: 0,
                callee_saved: Vec::new(),
            },
            instructions,
            relocations: Vec::new(),
            stack_slots: Vec::new(),
        }],
    };
    encode(&plan).expect("encode").text
}

fn words(bytes: &[u8]) -> Vec<u32> {
    bytes
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
        .collect()
}

fn ci(op: &str, fields: &[(&'static str, &str)]) -> CodeInstruction {
    let mut inst = CodeInstruction::new(op);
    for (k, v) in fields {
        inst = inst.field(k, v);
    }
    inst
}

#[test]
fn ret_and_ecall() {
    let w = words(&encode_text(vec![ci("svc", &[]), ci("ret", &[])]));
    assert_eq!(w[0], 0x0000_0073); // ecall
    assert_eq!(w[1], 0x0000_8067); // jalr x0, 0(ra)
}

#[test]
fn add_and_sub_r_type() {
    let w = words(&encode_text(vec![
        ci("add", &[("dst", "a0"), ("lhs", "a1"), ("rhs", "a2")]),
        ci("sub", &[("dst", "a0"), ("lhs", "a1"), ("rhs", "a2")]),
        ci("ret", &[]),
    ]));
    assert_eq!(w[0], 0x00c5_8533); // add a0, a1, a2
    assert_eq!(w[1], 0x40c5_8533); // sub a0, a1, a2
}

#[test]
fn li_small_and_move() {
    let w = words(&encode_text(vec![
        ci("mov_imm", &[("dst", "a0"), ("value", "5")]),
        ci("mov", &[("dst", "a1"), ("src", "a0")]),
        ci("ret", &[]),
    ]));
    assert_eq!(w[0], 0x0050_0513); // addi a0, zero, 5
    assert_eq!(w[1], 0x0005_0593); // addi a1, a0, 0  (mv a1, a0)
}

#[test]
fn li_thirty_two_bit_uses_lui_addi() {
    // 0x12345 = 74565 needs lui + addi.
    let w = words(&encode_text(vec![
        ci("mov_imm", &[("dst", "a0"), ("value", "74565")]),
        ci("ret", &[]),
    ]));
    // lui a0, 0x12 ; addi a0, a0, 0x345 (with the +0x800 rounding the low part
    // is 0x345 and the high 0x12).
    assert_eq!(w.len(), 3); // lui, addi, ret
}

#[test]
fn load_store_word_offsets() {
    let w = words(&encode_text(vec![
        ci("str_u64", &[("src", "a0"), ("base", "sp"), ("offset", "8")]),
        ci("ldr_u64", &[("dst", "a1"), ("base", "sp"), ("offset", "16")]),
        ci("ret", &[]),
    ]));
    // sd a0, 8(sp): S-type, funct3=011, opcode 0x23.
    assert_eq!(w[0], 0x00a1_3423);
    // ld a1, 16(sp): I-type, funct3=011, opcode 0x03.
    assert_eq!(w[1], 0x0101_3583);
}

#[test]
fn conditional_branch_is_long_form() {
    let w = words(&encode_text(vec![
        ci("label", &[("name", "top")]),
        ci(
            "rv.br",
            &[("lhs", "a0"), ("rhs", "a1"), ("cond", "lt"), ("target", "top")],
        ),
        ci("ret", &[]),
    ]));
    // Long form: bge a0,a1,+8 (inverted) then jal zero, top(-4).
    // bge: funct3=101, so word = imm(+8) | a1<<20 | a0<<15 | 101<<12 | 0x63.
    // b_type(8, 11, 10, 0b101, 0x63):
    let expected_bge = {
        let imm: u32 = 8;
        let b11 = (imm >> 11) & 1;
        let b4_1 = (imm >> 1) & 0xf;
        (0 << 31) | (0 << 25) | (11 << 20) | (10 << 15) | (0b101 << 12) | (b4_1 << 8) | (b11 << 7) | 0x63
    };
    assert_eq!(w[0], expected_bge);
    // jal zero, -4 (back to top): opcode 0x6f, rd=0.
    assert_eq!(w[1] & 0x7f, 0x6f);
    assert_eq!((w[1] >> 7) & 0x1f, 0); // rd = zero
}
