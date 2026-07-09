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
fn fp_min_max_d() {
    // fmin.d / fmax.d fa0, fs1, fa7 — funct7=0010101, funct3 000(min)/001(max),
    // OP-FP opcode 0x53. rd=fa0(10), rs1=fs1(9), rs2=fa7(17). These implement the
    // IEEE number semantics, matching AArch64 fminnm/fmaxnm (plan-02 §4).
    let w = words(&encode_text(vec![
        ci("fminnm_d", &[("dst", "fa0"), ("lhs", "fs1"), ("rhs", "fa7")]),
        ci("fmaxnm_d", &[("dst", "fa0"), ("lhs", "fs1"), ("rhs", "fa7")]),
        ci("ret", &[]),
    ]));
    assert_eq!(w[0], 0x2b14_8553); // fmin.d fa0, fs1, fa7
    assert_eq!(w[1], 0x2b14_9553); // fmax.d fa0, fs1, fa7
}

#[test]
fn fma_family_d() {
    // Scalar FMA family, R4-type. rd=fa0(10), rs3/addend=fa1(11), rs1/lhs=fa2(12),
    // rs2/rhs=fa3(13). RISC-V's native names match our neutral MIR result naming
    // (plan-02 §5), so fmadd_d→MADD, fmsub_d→MSUB, fnmsub_d→NMSUB, fnmadd_d→NMADD.
    let w = words(&encode_text(vec![
        ci(
            "fmadd_d",
            &[("dst", "fa0"), ("addend", "fa1"), ("lhs", "fa2"), ("rhs", "fa3")],
        ),
        ci(
            "fmsub_d",
            &[("dst", "fa0"), ("addend", "fa1"), ("lhs", "fa2"), ("rhs", "fa3")],
        ),
        ci(
            "fnmsub_d",
            &[("dst", "fa0"), ("addend", "fa1"), ("lhs", "fa2"), ("rhs", "fa3")],
        ),
        ci(
            "fnmadd_d",
            &[("dst", "fa0"), ("addend", "fa1"), ("lhs", "fa2"), ("rhs", "fa3")],
        ),
        ci("ret", &[]),
    ]));
    assert_eq!(w[0], 0x5ad6_0543); // fmadd.d  fa0, fa2, fa3, fa1
    assert_eq!(w[1], 0x5ad6_0547); // fmsub.d  fa0, fa2, fa3, fa1
    assert_eq!(w[2], 0x5ad6_054b); // fnmsub.d fa0, fa2, fa3, fa1
    assert_eq!(w[3], 0x5ad6_054f); // fnmadd.d fa0, fa2, fa3, fa1
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
fn large_offset_load_uses_rd_not_t0_as_address_scratch() {
    // A big-frame reload must not stage its address in `t0`: a scalarized `v128`
    // sequence keeps live lanes in `t0`/`t1`, so the fallback materializes the
    // address in the destination register (overwritten by the load) instead.
    let w = words(&encode_text(vec![
        ci("ldr_u64", &[("dst", "a1"), ("base", "sp"), ("offset", "7472")]),
        ci("ret", &[]),
    ]));
    // li a1, 7472 ; add a1, sp, a1 ; ld a1, 0(a1) — no `t0` (x5) anywhere.
    let a1 = 11u32;
    // The `add` is R-type add a1, sp(x2), a1: funct7=0, rs2=a1, rs1=sp, f3=0.
    let add = (0 << 25) | (a1 << 20) | (2 << 15) | (0 << 12) | (a1 << 7) | 0x33;
    assert_eq!(w[w.len() - 3], add, "address add targets rd, sourced from sp+rd");
    // ld a1, 0(a1): I-type, imm 0, rs1=a1, funct3=011, rd=a1.
    assert_eq!(w[w.len() - 2], (a1 << 15) | (0b011 << 12) | (a1 << 7) | 0x03);
    // No instruction reads or writes t0 (x5) as rd/rs1/rs2.
    for &word in &w[..w.len() - 1] {
        let (rd, rs1, rs2) = ((word >> 7) & 0x1f, (word >> 15) & 0x1f, (word >> 20) & 0x1f);
        assert!(rd != 5 && rs1 != 5 && rs2 != 5, "t0 (x5) must not appear: {word:#010x}");
    }
}

#[test]
fn large_offset_load_with_rd_aliasing_base_stages_through_t0() {
    // bug-14: when the allocator coalesces `dst == base`, staging the address in
    // `rd` would `li` over `base` first and load from `2 * offset`. Stage through
    // `t0` in that one case. (`rd == base` never occurs inside a v128 lane
    // sequence — those load from the `t2` slot base — so `t0` is dead here.)
    let w = words(&encode_text(vec![
        ci("ldr_u64", &[("dst", "a1"), ("base", "a1"), ("offset", "7472")]),
        ci("ret", &[]),
    ]));
    let (a1, t0) = (11u32, 5u32);
    // li t0, 7472 (lui+addi or addi) ; add t0, a1, t0 ; ld a1, 0(t0)
    let add = (t0 << 20) | (a1 << 15) | (t0 << 7) | 0x33;
    assert_eq!(w[w.len() - 3], add, "address add reads base before writing rd");
    assert_eq!(
        w[w.len() - 2],
        (t0 << 15) | (0b011 << 12) | (a1 << 7) | 0x03,
        "ld a1, 0(t0)"
    );
    // `base` (a1) is never written before the final load.
    for &word in &w[..w.len() - 2] {
        assert_ne!((word >> 7) & 0x1f, a1, "base clobbered: {word:#010x}");
    }
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

/// The base-ISA bit-manipulation expansions (`clz`/`rbit`/`rev_x`/`rev_w`, no
/// Zbb) emit multi-word sequences whose length varies with the `li` mask
/// materializations. The two-pass encoder relies on `instruction_size` predicting
/// that length exactly — a mismatch silently misplaces every later label — so
/// assert the prediction equals the bytes actually emitted for each.
#[test]
fn bitmanip_expansions_size_matches_emitted_bytes() {
    for op in ["clz", "rbit", "rev_x", "rev_w"] {
        let inst = ci(op, &[("dst", "a0"), ("src", "a1")]);
        let predicted = sizing::instruction_size(&inst).expect("size");
        let emitted = encode_text(vec![inst]).len();
        assert_eq!(
            predicted, emitted,
            "{op}: sizing predicted {predicted} bytes but emitted {emitted}"
        );
        assert_eq!(emitted % 4, 0, "{op}: emitted a non-word-aligned length");
    }
}

/// Simulate a `li` sequence and check it reconstructs the exact 64-bit value —
/// for small values, powers of ten (the float formatter's divisors), negatives,
/// and float bit patterns near the extremes.
#[test]
fn li_reconstructs_all_values() {
    fn simulate(value: i64) -> i64 {
        let mut rd: i64 = 0;
        for step in super::sizing::li_steps(value) {
            rd = match step {
                super::sizing::LiStep::Lui(hi20) => {
                    // sign-extend the 20-bit field, then <<12.
                    ((hi20 << 12) as i32) as i64
                }
                super::sizing::LiStep::Addi(imm) => imm as i64,
                super::sizing::LiStep::Slli(sh) => rd.wrapping_shl(sh),
                super::sizing::LiStep::AddiFrom(imm) => rd.wrapping_add(imm as i64),
            };
        }
        rd
    }
    let mut cases: Vec<i64> = vec![0, 1, -1, 2047, 2048, -2048, -2049, 4095, i32::MAX as i64,
        i32::MIN as i64, i64::MAX, i64::MIN, 0x400C_0000_0000_0000u64 as i64,
        0x3FF8_0000_0000_0000, 0x4004_0000_0000_0000];
    let mut p: i64 = 1;
    for _ in 0..19 { cases.push(p); p = p.wrapping_mul(10); }
    for v in cases {
        assert_eq!(simulate(v), v, "li mismatch for {v} ({v:#018x})");
    }
}

#[test]
fn align_zero_does_not_divide_by_zero() {
    // bug-18: a malformed plan (decoded `.mfp` IR is not re-validated before
    // codegen) could carry a data object with align 0, panicking `div_ceil(0)`.
    // Treat 0 (and 1) as "no alignment".
    assert_eq!(super::data::align(1, 0), 1);
    assert_eq!(super::data::align(0, 0), 0);
    assert_eq!(super::data::align(7, 1), 7);
    // Real alignments are unchanged.
    assert_eq!(super::data::align(1, 8), 8);
    assert_eq!(super::data::align(16, 16), 16);
    assert_eq!(super::data::align(17, 16), 32);
}
