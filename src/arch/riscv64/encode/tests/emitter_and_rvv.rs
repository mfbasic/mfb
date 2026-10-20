//! plan-176-J: the riscv64 encoder's emitter_and_rvv tests. The shared `encode_text`/`words`/`ci`/\n//! `plan_of`/`func`/`try_encode` helpers live in `encode/tests/mod.rs`.

use super::*;

// --- emitter.rs: the scalar op vocabulary --------------------------------------

#[test]
fn integer_alu_and_logic_ops_encode_as_op() {
    for op in [
        "and", "orr", "eor", "mul", "smulh", "umulh", "sdiv", "udiv", "rv.slt", "rv.sltu", "lslv",
        "lsrv", "asrv",
    ] {
        let w = words(&encode_text(vec![
            ci(op, &[("dst", "a0"), ("lhs", "a1"), ("rhs", "a2")]),
            ci("ret", &[]),
        ]));
        assert_eq!(w[0] & 0x7f, 0x33, "{op} must be an OP-format word");
    }
}

#[test]
fn rotate_ops_expand_to_fixed_sequences() {
    let w = words(&encode_text(vec![
        ci("rorv", &[("dst", "a0"), ("lhs", "a1"), ("rhs", "a2")]),
        ci("ret", &[]),
    ]));
    assert_eq!(w.len(), 5, "rorv is 4 words + ret");
    let w = words(&encode_text(vec![
        ci("rorv_w", &[("dst", "a0"), ("lhs", "a1"), ("rhs", "a2")]),
        ci("ret", &[]),
    ]));
    assert_eq!(w.len(), 7, "rorv_w is 6 words + ret");
}

#[test]
fn unary_and_muladd_ops_encode() {
    let w = words(&encode_text(vec![
        ci("mvn", &[("dst", "a0"), ("src", "a1")]),
        ci("sxtw", &[("dst", "a0"), ("src", "a1")]),
        ci("ret", &[]),
    ]));
    assert_eq!(w[0] & 0x7f, 0x13, "mvn -> xori (OP-IMM)");
    assert_eq!(w[1] & 0x7f, 0x1b, "sxtw -> addiw (OP-IMM-32)");

    let w = words(&encode_text(vec![
        ci(
            "msub",
            &[
                ("dst", "a0"),
                ("lhs", "a1"),
                ("rhs", "a2"),
                ("minuend", "a3"),
            ],
        ),
        ci("ret", &[]),
    ]));
    assert_eq!(w.len(), 3, "msub is mul;sub + ret");
}

#[test]
fn shift_immediate_ops_encode() {
    for op in ["lsl_imm", "lsr_imm", "asr_imm"] {
        let w = words(&encode_text(vec![
            ci(op, &[("dst", "a0"), ("src", "a1"), ("shift", "3")]),
            ci("ret", &[]),
        ]));
        assert_eq!(w[0] & 0x7f, 0x13, "{op} is an OP-IMM shift");
    }
}

#[test]
fn add_sub_immediate_small_and_large_paths() {
    // Small immediates fit the 12-bit field: a single word.
    let w = words(&encode_text(vec![
        ci("add_imm", &[("dst", "a0"), ("src", "a1"), ("imm", "5")]),
        ci("ret", &[]),
    ]));
    assert_eq!(w.len(), 2);
    // >2047 forces the `li t0, imm; add` fallback.
    let w = words(&encode_text(vec![
        ci("add_imm", &[("dst", "a0"), ("src", "a1"), ("imm", "5000")]),
        ci("ret", &[]),
    ]));
    assert!(w.len() > 2, "large add_imm must materialize the immediate");
    // sub_imm: -imm fits at <=2048.
    let w = words(&encode_text(vec![
        ci("sub_imm", &[("dst", "a0"), ("src", "a1"), ("imm", "2048")]),
        ci("ret", &[]),
    ]));
    assert_eq!(w.len(), 2);
    let w = words(&encode_text(vec![
        ci("sub_imm", &[("dst", "a0"), ("src", "a1"), ("imm", "5000")]),
        ci("ret", &[]),
    ]));
    assert!(w.len() > 2);
    // sp adjustments reuse the add/sub-imm helpers with x2.
    let w = words(&encode_text(vec![
        ci("add_sp", &[("imm", "16")]),
        ci("sub_sp", &[("imm", "16")]),
        ci("ret", &[]),
    ]));
    assert_eq!(w.len(), 3);
}

#[test]
fn load_store_variants_and_large_offsets() {
    for op in ["ldr_u32", "ldr_u16", "ldr_u8"] {
        let w = words(&encode_text(vec![
            ci(op, &[("dst", "a0"), ("base", "sp"), ("offset", "8")]),
            ci("ret", &[]),
        ]));
        assert_eq!(w[0] & 0x7f, 0x03, "{op} is a LOAD");
    }
    for op in ["str_u32", "str_u16", "str_u8"] {
        let w = words(&encode_text(vec![
            ci(op, &[("src", "a0"), ("base", "sp"), ("offset", "8")]),
            ci("ret", &[]),
        ]));
        assert_eq!(w[0] & 0x7f, 0x23, "{op} is a STORE");
    }
    // Large store offset -> li + add + store.
    let w = words(&encode_text(vec![
        ci(
            "str_u64",
            &[("src", "a0"), ("base", "sp"), ("offset", "5000")],
        ),
        ci("ret", &[]),
    ]));
    assert!(w.len() >= 4);
    // FP load/store, small and large.
    let w = words(&encode_text(vec![
        ci("ldr_d", &[("dst", "fa0"), ("base", "sp"), ("offset", "8")]),
        ci("str_d", &[("src", "fa0"), ("base", "sp"), ("offset", "8")]),
        ci("ret", &[]),
    ]));
    assert_eq!(w[0] & 0x7f, 0x07, "ldr_d is LOAD-FP");
    assert_eq!(w[1] & 0x7f, 0x27, "str_d is STORE-FP");
    let w = words(&encode_text(vec![
        ci(
            "ldr_d",
            &[("dst", "fa0"), ("base", "sp"), ("offset", "5000")],
        ),
        ci(
            "str_d",
            &[("src", "fa0"), ("base", "sp"), ("offset", "5000")],
        ),
        ci("ret", &[]),
    ]));
    assert!(w.len() > 3, "large fp offsets materialize the address");
}

#[test]
fn control_flow_and_system_ops_encode() {
    let w = words(&encode_text(vec![
        ci("blr", &[("register", "a0")]),
        ci("branch_self", &[]),
        ci("svc", &[]),
        ci("ret", &[]),
    ]));
    assert_eq!(w[0] & 0x7f, 0x67, "blr -> jalr");
    assert_eq!(w[1] & 0x7f, 0x6f, "branch_self -> jal");
    assert_eq!(w[2], 0x73, "svc -> ecall");

    let w = words(&encode_text(vec![
        ci("label", &[("name", "here")]),
        ci("b", &[("target", "here")]),
        ci("ret", &[]),
    ]));
    assert_eq!(w[0] & 0x7f, 0x6f, "b -> jal");
}

#[test]
fn explicit_carry_ops_are_fixed_length() {
    let w = words(&encode_text(vec![
        ci(
            "add_carry",
            &[
                ("dst", "a0"),
                ("carry_out", "a1"),
                ("lhs", "a2"),
                ("rhs", "a3"),
                ("carry_in", "a4"),
            ],
        ),
        ci("ret", &[]),
    ]));
    assert_eq!(w.len(), 8, "add_carry is a 7-word sltu sequence + ret");
    let w = words(&encode_text(vec![
        ci(
            "sub_borrow",
            &[
                ("dst", "a0"),
                ("borrow_out", "a1"),
                ("lhs", "a2"),
                ("rhs", "a3"),
                ("borrow_in", "a4"),
            ],
        ),
        ci("ret", &[]),
    ]));
    assert_eq!(w.len(), 8, "sub_borrow is a 7-word sequence + ret");
}

#[test]
fn scalar_fp_ops_encode_as_op_fp() {
    for op in ["fadd_d", "fsub_d", "fmul_d", "fdiv_d"] {
        let w = words(&encode_text(vec![
            ci(op, &[("dst", "fa0"), ("lhs", "fa1"), ("rhs", "fa2")]),
            ci("ret", &[]),
        ]));
        assert_eq!(w[0] & 0x7f, 0x53, "{op} is OP-FP");
    }
    for op in ["fmov_d_from_d", "fneg_d", "fabs_d", "fsqrt_d"] {
        let w = words(&encode_text(vec![
            ci(op, &[("dst", "fa0"), ("src", "fa1")]),
            ci("ret", &[]),
        ]));
        assert_eq!(w[0] & 0x7f, 0x53, "{op} is OP-FP");
    }
    // FP<->GPR moves and conversions.
    let w = words(&encode_text(vec![
        ci("fmov_x_from_d", &[("dst", "a0"), ("src", "fa1")]),
        ci("fmov_d_from_x", &[("dst", "fa0"), ("src", "a1")]),
        ci("scvtf_d_from_x", &[("dst", "fa0"), ("src", "a1")]),
        ci("ret", &[]),
    ]));
    for (i, word) in w.iter().take(3).enumerate() {
        assert_eq!(word & 0x7f, 0x53, "fp move/convert {i} is OP-FP");
    }
    for op in [
        "fcvtzs_x_from_d",
        "fcvtms_x_from_d",
        "fcvtps_x_from_d",
        "fcvtas_x_from_d",
    ] {
        let w = words(&encode_text(vec![
            ci(op, &[("dst", "a0"), ("src", "fa1")]),
            ci("ret", &[]),
        ]));
        assert_eq!(w[0] & 0x7f, 0x53, "{op} is OP-FP");
    }
    for cmp in ["eq", "lt", "le"] {
        let w = words(&encode_text(vec![
            ci(
                "rv.fcmp",
                &[("dst", "a0"), ("lhs", "fa1"), ("rhs", "fa2"), ("cmp", cmp)],
            ),
            ci("ret", &[]),
        ]));
        assert_eq!(w[0] & 0x7f, 0x53, "fcmp {cmp} is OP-FP");
    }
}

#[test]
fn immediate_materialization_true_and_wide_values() {
    // `true`/`false` immediates decode to 1/0 through `mov_imm`.
    let w = words(&encode_text(vec![
        ci("mov_imm", &[("dst", "a0"), ("value", "true")]),
        ci("ret", &[]),
    ]));
    assert_eq!(w[0], 0x0010_0513, "addi a0, zero, 1");
    // A > 32-bit value with non-zero low bits forces the recursive li,
    // exercising both the `Slli` and `AddiFrom` expansion steps.
    let w = words(&encode_text(vec![
        ci("mov_imm", &[("dst", "a0"), ("value", "4294967297")]), // 2^32 + 1
        ci("ret", &[]),
    ]));
    assert!(
        w.len() >= 4,
        "wide immediate needs a multi-step li: {}",
        w.len()
    );
}

#[test]
fn emitter_rejects_bad_operands() {
    // Unknown fcmp kind.
    assert!(try_encode(vec![ci(
        "rv.fcmp",
        &[("dst", "a0"), ("lhs", "fa1"), ("rhs", "fa2"), ("cmp", "gt")],
    )])
    .unwrap_err()
    .contains("fcmp"));
    // Unknown branch condition.
    assert!(try_encode(vec![ci(
        "rv.br",
        &[
            ("lhs", "a0"),
            ("rhs", "a1"),
            ("cond", "zz"),
            ("target", "x")
        ],
    )])
    .unwrap_err()
    .contains("branch condition"));
    // Unknown register.
    assert!(try_encode(vec![ci(
        "add",
        &[("dst", "x99"), ("lhs", "a1"), ("rhs", "a2")],
    )])
    .unwrap_err()
    .contains("register"));
    // Missing field.
    assert!(try_encode(vec![ci("add", &[("dst", "a0"), ("lhs", "a1")])])
        .unwrap_err()
        .contains("missing field"));
    // Shift out of range.
    assert!(try_encode(vec![ci(
        "lsl_imm",
        &[("dst", "a0"), ("src", "a1"), ("shift", "64")],
    )])
    .unwrap_err()
    .contains("out of range"));
    // An op the rv64 scalar encoder does not yet handle (flag-setting `adds`).
    assert!(try_encode(vec![ci(
        "adds",
        &[("dst", "a0"), ("lhs", "a1"), ("rhs", "a2")],
    )])
    .unwrap_err()
    .contains("does not yet support"));
}

// --- operand.rs / emitter.rs: the RVV vector encoder (plan-32-B) ---------------

/// Encode a single `rv.vop` (with the `vop` field first) and return its word.
fn vword(vop: &str, fields: &[(&'static str, &str)]) -> u32 {
    let mut all: Vec<(&'static str, &str)> = vec![("vop", vop)];
    all.extend_from_slice(fields);
    words(&encode_text(vec![ci("rv.vop", &all), ci("ret", &[])]))[0]
}

#[test]
fn vector_register_names_decode_to_numbers() {
    assert_eq!(super::operand::vreg("v0".to_string()).unwrap(), 0);
    assert_eq!(super::operand::vreg("v1".to_string()).unwrap(), 1);
    assert_eq!(super::operand::vreg("v31".to_string()).unwrap(), 31);
    // Out of range and non-vector names are rejected.
    for bad in ["v32", "v99", "a0", "ft0", "vx", "x1"] {
        assert!(
            super::operand::vreg(bad.to_string())
                .unwrap_err()
                .contains("unknown rv64 vector register"),
            "vreg({bad}) should error"
        );
    }
}

/// Every vector mnemonic encodes to the exact 32-bit word a reference RISC-V
/// assembler emits for the same operands. The golden words were produced by
///   `riscv64-unknown-elf-as -march=rv64gcv` (cross-checked with
///   `llvm-mc -triple=riscv64 -mattr=+v --show-encoding`)
/// on the operand set `vd=v1, vs2=v2, vs1=v3, rs1=a0` (plan-32-B); regenerate by
/// assembling the same mnemonics and disassembling.
#[test]
fn rvv_ops_encode_to_reference_words() {
    // (vop, fields, expected little-endian word).
    let cases: &[(&str, &[(&'static str, &str)], u32)] = &[
        // configuration: SEW=64, LMUL=1, ta, ma.
        ("vsetvli", &[("dst", "a0"), ("avl", "a1")], 0x0d85_f557),
        ("vsetivli", &[("dst", "zero"), ("avl", "2")], 0xcd81_7057),
        // float vector-vector arithmetic (vd=v1, lhs=v2, rhs=v3).
        (
            "vfadd.vv",
            &[("dst", "v1"), ("lhs", "v2"), ("rhs", "v3")],
            0x0221_90d7,
        ),
        (
            "vfsub.vv",
            &[("dst", "v1"), ("lhs", "v2"), ("rhs", "v3")],
            0x0a21_90d7,
        ),
        (
            "vfmul.vv",
            &[("dst", "v1"), ("lhs", "v2"), ("rhs", "v3")],
            0x9221_90d7,
        ),
        (
            "vfdiv.vv",
            &[("dst", "v1"), ("lhs", "v2"), ("rhs", "v3")],
            0x8221_90d7,
        ),
        (
            "vfmin.vv",
            &[("dst", "v1"), ("lhs", "v2"), ("rhs", "v3")],
            0x1221_90d7,
        ),
        (
            "vfmax.vv",
            &[("dst", "v1"), ("lhs", "v2"), ("rhs", "v3")],
            0x1a21_90d7,
        ),
        (
            "vfsgnjn.vv",
            &[("dst", "v1"), ("lhs", "v2"), ("rhs", "v3")],
            0x2621_90d7,
        ),
        (
            "vfsgnjx.vv",
            &[("dst", "v1"), ("lhs", "v2"), ("rhs", "v3")],
            0x2a21_90d7,
        ),
        (
            "vmfeq.vv",
            &[("dst", "v1"), ("lhs", "v2"), ("rhs", "v3")],
            0x6221_90d7,
        ),
        (
            "vmfle.vv",
            &[("dst", "v1"), ("lhs", "v2"), ("rhs", "v3")],
            0x6621_90d7,
        ),
        (
            "vmflt.vv",
            &[("dst", "v1"), ("lhs", "v2"), ("rhs", "v3")],
            0x6e21_90d7,
        ),
        // fused multiply-accumulate (vd += lhs*rhs): lhs=v2, rhs=v3.
        (
            "vfmacc.vv",
            &[("dst", "v1"), ("lhs", "v2"), ("rhs", "v3")],
            0xb231_10d7,
        ),
        (
            "vfnmsac.vv",
            &[("dst", "v1"), ("lhs", "v2"), ("rhs", "v3")],
            0xbe31_10d7,
        ),
        // unary float (vd=v1, src=v2).
        ("vfsqrt.v", &[("dst", "v1"), ("src", "v2")], 0x4e20_10d7),
        ("vfcvt.x.f.v", &[("dst", "v1"), ("src", "v2")], 0x4a20_90d7),
        (
            "vfcvt.rtz.x.f.v",
            &[("dst", "v1"), ("src", "v2")],
            0x4a23_90d7,
        ),
        ("vfcvt.f.x.v", &[("dst", "v1"), ("src", "v2")], 0x4a21_90d7),
        // integer vector-vector.
        (
            "vadd.vv",
            &[("dst", "v1"), ("lhs", "v2"), ("rhs", "v3")],
            0x0221_80d7,
        ),
        (
            "vsub.vv",
            &[("dst", "v1"), ("lhs", "v2"), ("rhs", "v3")],
            0x0a21_80d7,
        ),
        (
            "vand.vv",
            &[("dst", "v1"), ("lhs", "v2"), ("rhs", "v3")],
            0x2621_80d7,
        ),
        (
            "vor.vv",
            &[("dst", "v1"), ("lhs", "v2"), ("rhs", "v3")],
            0x2a21_80d7,
        ),
        (
            "vxor.vv",
            &[("dst", "v1"), ("lhs", "v2"), ("rhs", "v3")],
            0x2e21_80d7,
        ),
        (
            "vmseq.vv",
            &[("dst", "v1"), ("lhs", "v2"), ("rhs", "v3")],
            0x6221_80d7,
        ),
        (
            "vmsle.vv",
            &[("dst", "v1"), ("lhs", "v2"), ("rhs", "v3")],
            0x7621_80d7,
        ),
        (
            "vmslt.vv",
            &[("dst", "v1"), ("lhs", "v2"), ("rhs", "v3")],
            0x6e21_80d7,
        ),
        (
            "vsll.vv",
            &[("dst", "v1"), ("lhs", "v2"), ("rhs", "v3")],
            0x9621_80d7,
        ),
        (
            "vsra.vv",
            &[("dst", "v1"), ("lhs", "v2"), ("rhs", "v3")],
            0xa621_80d7,
        ),
        (
            "vsrl.vv",
            &[("dst", "v1"), ("lhs", "v2"), ("rhs", "v3")],
            0xa221_80d7,
        ),
        // vector-immediate shifts / slide (imm=3, slide index=1).
        (
            "vsll.vi",
            &[("dst", "v1"), ("lhs", "v2"), ("imm", "3")],
            0x9621_b0d7,
        ),
        (
            "vsra.vi",
            &[("dst", "v1"), ("lhs", "v2"), ("imm", "3")],
            0xa621_b0d7,
        ),
        (
            "vsrl.vi",
            &[("dst", "v1"), ("lhs", "v2"), ("imm", "3")],
            0xa221_b0d7,
        ),
        (
            "vslidedown.vi",
            &[("dst", "v1"), ("lhs", "v2"), ("imm", "1")],
            0x3e20_b0d7,
        ),
        // vector-scalar shifts / reverse-subtract (rs1=a0; neg uses rs1=zero).
        (
            "vsll.vx",
            &[("dst", "v1"), ("lhs", "v2"), ("gpr", "a0")],
            0x9625_40d7,
        ),
        (
            "vsra.vx",
            &[("dst", "v1"), ("lhs", "v2"), ("gpr", "a0")],
            0xa625_40d7,
        ),
        (
            "vsrl.vx",
            &[("dst", "v1"), ("lhs", "v2"), ("gpr", "a0")],
            0xa225_40d7,
        ),
        (
            "vrsub.vx",
            &[("dst", "v1"), ("lhs", "v2"), ("gpr", "zero")],
            0x0e20_40d7,
        ),
        // splat / lane-mask materialization (imm=31 encodes the all-ones -1).
        ("vmv.v.i", &[("dst", "v1"), ("imm", "0")], 0x5e00_30d7),
        ("vmv.v.x", &[("dst", "v1"), ("gpr", "a0")], 0x5e05_40d7),
        (
            "vmerge.vim",
            &[("dst", "v1"), ("src", "v2"), ("imm", "31")],
            0x5c2f_b0d7,
        ),
        // element extract / insert (lane 0 ↔ GPR).
        ("vmv.x.s", &[("dst", "a0"), ("src", "v2")], 0x4220_2557),
        ("vmv.s.x", &[("dst", "v1"), ("gpr", "a0")], 0x4205_60d7),
        // unit-stride 128-bit (2×e64) load / store.
        ("vle64.v", &[("dst", "v1"), ("base", "a0")], 0x0205_7087),
        ("vse64.v", &[("src", "v1"), ("base", "a0")], 0x0205_70a7),
    ];
    for (vop, fields, expected) in cases {
        assert_eq!(
            vword(vop, fields),
            *expected,
            "{vop} encoded to {:#010x}, expected {expected:#010x}",
            vword(vop, fields)
        );
    }
}

/// Every RVV mnemonic is a single 4-byte word, and the emitter-derived
/// `instruction_size` predicts exactly that (the two-pass encoder relies on it).
#[test]
fn rvv_ops_are_single_words_and_size_matches() {
    let insts = [
        ci(
            "rv.vop",
            &[("vop", "vsetivli"), ("dst", "zero"), ("avl", "2")],
        ),
        ci(
            "rv.vop",
            &[
                ("vop", "vfadd.vv"),
                ("dst", "v1"),
                ("lhs", "v2"),
                ("rhs", "v3"),
            ],
        ),
        ci(
            "rv.vop",
            &[
                ("vop", "vmerge.vim"),
                ("dst", "v1"),
                ("src", "v2"),
                ("imm", "31"),
            ],
        ),
        ci(
            "rv.vop",
            &[("vop", "vmv.x.s"), ("dst", "a0"), ("src", "v2")],
        ),
        ci(
            "rv.vop",
            &[("vop", "vle64.v"), ("dst", "v1"), ("base", "a0")],
        ),
        ci(
            "rv.vop",
            &[("vop", "vse64.v"), ("src", "v1"), ("base", "a0")],
        ),
    ];
    for inst in insts {
        let predicted = sizing::instruction_size(&inst).expect("size");
        assert_eq!(
            predicted,
            4,
            "{:?} should size to one word",
            inst.get("vop")
        );
        let emitted = encode_text(vec![inst]).len();
        assert_eq!(emitted, 4, "expected a single 4-byte word");
    }
}

#[test]
fn rvv_encoder_rejects_unknown_vop_and_bad_registers() {
    // Unknown vector op mnemonic.
    assert!(try_encode(vec![ci("rv.vop", &[("vop", "vzzz.vv")])])
        .unwrap_err()
        .contains("does not support vector op"));
    // Out-of-range vector register.
    assert!(try_encode(vec![ci(
        "rv.vop",
        &[
            ("vop", "vfadd.vv"),
            ("dst", "v32"),
            ("lhs", "v2"),
            ("rhs", "v3")
        ],
    )])
    .unwrap_err()
    .contains("unknown rv64 vector register"));
    // Missing operand field for the selected op.
    assert!(
        try_encode(vec![ci("rv.vop", &[("vop", "vfadd.vv"), ("dst", "v1")],)])
            .unwrap_err()
            .contains("missing field")
    );
}

/// bug-788: the binary32 conversions a `LINK` `CFloat` crosses.
#[test]
fn float_precision_conversions_encode() {
    let w = words(&encode_text(vec![
        ci("fcvt_s_from_d", &[("dst", "fa0"), ("src", "fa1")]),
        ci("fcvt_d_from_s", &[("dst", "fa0"), ("src", "fa1")]),
        ci("ret", &[]),
    ]));
    // fcvt.s.d fa0, fa1, rne
    assert_eq!(w[0], 0x4015_8553);
    // fcvt.d.s fa0, fa1
    assert_eq!(w[1], 0x4205_8553);
}
