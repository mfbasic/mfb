//! plan-176-J: the AArch64 encoder's forms tests. The shared `fresh_encoder`/`encode_one`/\n//! `emit_words`/`plan` helpers live in `encode/tests/mod.rs`.

use super::*;

#[test]
fn instruction_size_expands_large_stack_and_memory_immediates() {
    let sub_sp = CodeInstruction::new("sub_sp").field("imm", "6400");
    let ldr = CodeInstruction::new("ldr_u64")
        .physical_field("dst", "x0")
        .physical_field("base", "sp")
        .field("offset", "32800");

    assert_eq!(instruction_size(&sub_sp).unwrap(), 8);
    assert_eq!(instruction_size(&ldr).unwrap(), 12);
}

#[test]
fn encoder_accepts_large_immediates_with_fallback_sequences() {
    let mut encoder = Encoder {
        text: Vec::new(),
        data: Vec::new(),
        symbols: Vec::new(),
        relocations: Vec::new(),
        imports: HashMap::new(),
        labels: HashMap::new(),
        patches: Vec::new(),
    };

    encoder
        .emit_instruction(&CodeInstruction::new("sub_sp").field("imm", "6400"))
        .unwrap();
    encoder
        .emit_instruction(
            &CodeInstruction::new("str_u64")
                .physical_field("src", "x0")
                .physical_field("base", "sp")
                .field("offset", "32800"),
        )
        .unwrap();
    encoder
        .emit_instruction(
            &CodeInstruction::new("cmp_imm")
                .physical_field("lhs", "x1")
                .physical_field("rhs", "6400"),
        )
        .unwrap();

    assert_eq!(encoder.text.len(), 28);
}

#[test]
fn encodes_umulh_add_carry_and_rorv() {
    let mut encoder = Encoder {
        text: Vec::new(),
        data: Vec::new(),
        symbols: Vec::new(),
        relocations: Vec::new(),
        imports: HashMap::new(),
        labels: HashMap::new(),
        patches: Vec::new(),
    };

    encoder
        .emit_instruction(
            &CodeInstruction::new("umulh")
                .physical_field("dst", "x14")
                .physical_field("lhs", "x11")
                .physical_field("rhs", "x9"),
        )
        .unwrap();
    // Explicit-carry add, no carry-in (plan-00-G §4) → `adds; cset` (2 words).
    // It must NOT use `cmp xzr,#1` (x31 = SP in the immediate form, a real bug
    // this guards: it would compute SP-1 and wrongly add 1).
    encoder
        .emit_instruction(
            &CodeInstruction::new("add_carry")
                .physical_field("dst", "x10")
                .physical_field("carry_out", "x11")
                .physical_field("lhs", "x14")
                .physical_field("rhs", "x12")
                .physical_field("carry_in", "xzr"),
        )
        .unwrap();
    // Explicit-carry add with a carry-in register → `cmp; adcs; cset` (3 words).
    encoder
        .emit_instruction(
            &CodeInstruction::new("add_carry")
                .physical_field("dst", "x13")
                .physical_field("carry_out", "xzr")
                .physical_field("lhs", "x15")
                .physical_field("rhs", "x16")
                .physical_field("carry_in", "x11"),
        )
        .unwrap();
    encoder
        .emit_instruction(
            &CodeInstruction::new("rorv")
                .physical_field("dst", "x0")
                .physical_field("lhs", "x12")
                .physical_field("rhs", "x11"),
        )
        .unwrap();

    let mut expected = Vec::new();
    expected.extend_from_slice(&0x9bc9_7d6e_u32.to_le_bytes()); // umulh x14, x11, x9
    expected.extend_from_slice(&0xab0c_01ca_u32.to_le_bytes()); // adds  x10, x14, x12
    expected.extend_from_slice(&0x9a9f_37eb_u32.to_le_bytes()); // cset  x11, cs
    expected.extend_from_slice(&0xf100_057f_u32.to_le_bytes()); // cmp   x11, #1
    expected.extend_from_slice(&0xba10_01ed_u32.to_le_bytes()); // adcs  x13, x15, x16
    expected.extend_from_slice(&0x9a9f_37ff_u32.to_le_bytes()); // cset  xzr, cs
    expected.extend_from_slice(&0x9acb_2d80_u32.to_le_bytes()); // rorv  x0, x12, x11
    assert_eq!(encoder.text, expected);
}

#[test]
fn encodes_fmov_d_from_d() {
    let mut encoder = fresh_encoder();
    for inst in [
        CodeInstruction::new("fmov_d_from_d")
            .physical_field("dst", "d5")
            .physical_field("src", "d3"),
        CodeInstruction::new("fmov_d_from_d")
            .physical_field("dst", "d8")
            .physical_field("src", "d0"),
    ] {
        encoder.emit_instruction(&inst).unwrap();
    }
    let mut expected = Vec::new();
    for word in [0x1e60_4065_u32, 0x1e60_4008] {
        expected.extend_from_slice(&word.to_le_bytes());
    }
    assert_eq!(encoder.text, expected);
}

#[test]
fn encodes_sxtw() {
    // `sxtw Xd, Wn` = `SBFM Xd, Xn, #0, #31` (sf=1, N=1, immr=0, imms=31).
    // Guards bug-04: narrows a C `int` return before the 64-bit flush sign-check.
    let mut encoder = fresh_encoder();
    for inst in [
        CodeInstruction::new("sxtw")
            .physical_field("dst", "x0")
            .physical_field("src", "x0"),
        CodeInstruction::new("sxtw")
            .physical_field("dst", "x3")
            .physical_field("src", "x1"),
    ] {
        encoder.emit_instruction(&inst).unwrap();
    }
    let mut expected = Vec::new();
    for word in [0x9340_7c00_u32, 0x9340_7c23] {
        expected.extend_from_slice(&word.to_le_bytes());
    }
    assert_eq!(encoder.text, expected);
}

#[test]
fn encodes_fabs_d() {
    // FABS (scalar, double) d5, d3 — checked against `as -arch arm64`.
    assert_eq!(
        encode_one(
            &CodeInstruction::new("fabs_d")
                .physical_field("dst", "d5")
                .physical_field("src", "d3")
        ),
        0x1e60_c065
    );
}

#[test]
fn encodes_fminnm_fmaxnm_d() {
    // FMINNM/FMAXNM (scalar, double) d5, d9, d17 — checked against
    // `as -arch arm64`. Opcode 0111 (minnm) / 0110 (maxnm) in the FP
    // data-processing 2-source `.d` group.
    assert_eq!(
        encode_one(
            &CodeInstruction::new("fminnm_d")
                .physical_field("dst", "d5")
                .physical_field("lhs", "d9")
                .physical_field("rhs", "d17")
        ),
        0x1e71_7925
    );
    assert_eq!(
        encode_one(
            &CodeInstruction::new("fmaxnm_d")
                .physical_field("dst", "d5")
                .physical_field("lhs", "d9")
                .physical_field("rhs", "d17")
        ),
        0x1e71_6925
    );
}

#[test]
fn encodes_b_vs_branch() {
    // `b.vs` (overflow set, the FP-domain NaN branch) must encode condition 0b0110,
    // distinct from `b.vc` (0b0111) — swapping them would trap every finite float.
    let mut encoder = fresh_encoder();
    // Reserve the label position one word ahead, then emit and patch.
    encoder.labels.insert("target".to_string(), 4);
    encoder
        .emit_instruction(&CodeInstruction::new("b.vs").field("target", "target"))
        .unwrap();
    encoder.patch_labels().unwrap();
    let word = u32::from_le_bytes(encoder.text[..4].try_into().unwrap());
    // 0x54000006 (b.vs) | imm19(=1) << 5.
    assert_eq!(word, 0x5400_0026);
}

#[test]
fn encodes_b_mi_and_b_ls_branches() {
    // `b.mi` (N set) is the IEEE float `<` and `b.ls` (C clear or Z set) the
    // IEEE float `<=` (plan-17): an unordered NaN must fall to the false side,
    // so the conditions are 0b0100 and 0b1001 respectively — distinct from the
    // signed-integer `b.lt` (0b1011) / `b.le` (0b1101) used for `Integer`.
    let mut encoder = fresh_encoder();
    encoder.labels.insert("target".to_string(), 4);
    encoder
        .emit_instruction(&CodeInstruction::new("b.mi").field("target", "target"))
        .unwrap();
    encoder.patch_labels().unwrap();
    let word = u32::from_le_bytes(encoder.text[..4].try_into().unwrap());
    // 0x54000004 (b.mi) | imm19(=1) << 5.
    assert_eq!(word, 0x5400_0024);

    let mut encoder = fresh_encoder();
    encoder.labels.insert("target".to_string(), 4);
    encoder
        .emit_instruction(&CodeInstruction::new("b.ls").field("target", "target"))
        .unwrap();
    encoder.patch_labels().unwrap();
    let word = u32::from_le_bytes(encoder.text[..4].try_into().unwrap());
    // 0x54000009 (b.ls) | imm19(=1) << 5.
    assert_eq!(word, 0x5400_0029);
}

#[test]
fn encodes_fp_scalar_spill_load_store() {
    let mut encoder = fresh_encoder();
    for inst in [
        CodeInstruction::new("str_d")
            .physical_field("src", "d0")
            .physical_field("base", "sp")
            .field("offset", "0"),
        CodeInstruction::new("ldr_d")
            .physical_field("dst", "d0")
            .physical_field("base", "sp")
            .field("offset", "8"),
        CodeInstruction::new("str_d")
            .physical_field("src", "d8")
            .physical_field("base", "sp")
            .field("offset", "16"),
        CodeInstruction::new("ldr_d")
            .physical_field("dst", "d15")
            .physical_field("base", "x9")
            .field("offset", "4088"),
    ] {
        encoder.emit_instruction(&inst).unwrap();
    }
    let mut expected = Vec::new();
    for word in [0xfd00_03e0_u32, 0xfd40_07e0, 0xfd00_0be8, 0xfd47_fd2f] {
        expected.extend_from_slice(&word.to_le_bytes());
    }
    assert_eq!(encoder.text, expected);
}

#[test]
fn encodes_load_acquire_store_release() {
    // bug-564. The oracle words come from Apple clang 17 on the macOS host:
    // `clang -c -arch arm64` of these exact lines, then `otool -t`. A64 has no
    // offset form for either instruction, so the base is a register alone.
    let op = |mnemonic: &str, rt_field: &'static str, rt: &str, base: &str| {
        CodeInstruction::new(mnemonic)
            .physical_field(rt_field, rt)
            .physical_field("base", base)
    };
    for (instruction, word, spelling) in [
        (
            op("stlr_u64", "src", "x9", "x10"),
            0xc89f_fd49_u32,
            "stlr x9, [x10]",
        ),
        (
            op("stlr_u64", "src", "x1", "x19"),
            0xc89f_fe61,
            "stlr x1, [x19]",
        ),
        (
            op("stlr_u64", "src", "x30", "x0"),
            0xc89f_fc1e,
            "stlr x30, [x0]",
        ),
        (
            op("stlr_u64", "src", "xzr", "sp"),
            0xc89f_ffff,
            "stlr xzr, [sp]",
        ),
        (
            op("ldar_u64", "dst", "x9", "x10"),
            0xc8df_fd49,
            "ldar x9, [x10]",
        ),
        (
            op("ldar_u64", "dst", "x0", "sp"),
            0xc8df_ffe0,
            "ldar x0, [sp]",
        ),
        (
            op("ldar_u32", "dst", "w9", "x10"),
            0x88df_fd49,
            "ldar w9, [x10]",
        ),
        (
            op("ldar_u32", "dst", "x10", "x9"),
            0x88df_fd2a,
            "ldar w10, [x9]",
        ),
    ] {
        assert_eq!(encode_one(&instruction), word, "{spelling}");
        assert_eq!(instruction_size(&instruction).unwrap(), 4, "{spelling}");
    }
    for (mnemonic, op) in [
        ("stlr_u64", CodeOp::StlrU64),
        ("ldar_u64", CodeOp::LdarU64),
        ("ldar_u32", CodeOp::LdarU32),
    ] {
        assert_eq!(CodeInstruction::new(mnemonic).op, op);
    }
}

/// Each new NEON op's encoding is checked against the exact little-endian word
/// produced by the system assembler (`as -arch arm64`) for the same mnemonic
/// and operands. Operands deliberately use distinct register numbers
/// (dst=v5/x5, lhs/src=v9/x9, rhs=v17/x17) so a misplaced Rd/Rn/Rm bit-field
/// is caught, not just a wrong base constant.
#[test]
fn encodes_neon_vector_ops() {
    let three = |op: &str| {
        CodeInstruction::new(op)
            .physical_field("dst", "v5")
            .physical_field("lhs", "v9")
            .physical_field("rhs", "v17")
    };
    let two = |op: &str| {
        CodeInstruction::new(op)
            .physical_field("dst", "v5")
            .physical_field("src", "v17")
    };

    // Three-same .2d / .16b.
    assert_eq!(encode_one(&three("fadd_v")), 0x4e71_d525);
    assert_eq!(encode_one(&three("fsub_v")), 0x4ef1_d525);
    assert_eq!(encode_one(&three("fmul_v")), 0x6e71_dd25);
    assert_eq!(encode_one(&three("fdiv_v")), 0x6e71_fd25);
    assert_eq!(encode_one(&three("fmla_v")), 0x4e71_cd25);
    assert_eq!(encode_one(&three("fmls_v")), 0x4ef1_cd25);
    assert_eq!(encode_one(&three("fmin_v")), 0x4ef1_f525);
    assert_eq!(encode_one(&three("fmax_v")), 0x4e71_f525);
    assert_eq!(encode_one(&three("fcmgt_v")), 0x6ef1_e525);
    assert_eq!(encode_one(&three("fcmge_v")), 0x6e71_e525);
    assert_eq!(encode_one(&three("fcmeq_v")), 0x4e71_e525);
    assert_eq!(encode_one(&three("add_v")), 0x4ef1_8525);
    assert_eq!(encode_one(&three("sub_v")), 0x6ef1_8525);
    assert_eq!(encode_one(&three("cmgt_v")), 0x4ef1_3525);
    assert_eq!(encode_one(&three("cmge_v")), 0x4ef1_3d25);
    assert_eq!(encode_one(&three("cmeq_v")), 0x6ef1_8d25);
    assert_eq!(encode_one(&three("sshl_v")), 0x4ef1_4525);
    assert_eq!(encode_one(&three("ushl_v")), 0x6ef1_4525);
    assert_eq!(encode_one(&three("and_v")), 0x4e31_1d25);
    assert_eq!(encode_one(&three("orr_v")), 0x4eb1_1d25);
    assert_eq!(encode_one(&three("eor_v")), 0x6e31_1d25);
    assert_eq!(encode_one(&three("bsl_v")), 0x6e71_1d25);
    assert_eq!(encode_one(&three("bit_v")), 0x6eb1_1d25);

    // Two-reg-misc .2d.
    assert_eq!(encode_one(&two("fabs_v")), 0x4ee0_fa25);
    assert_eq!(encode_one(&two("fneg_v")), 0x6ee0_fa25);
    assert_eq!(encode_one(&two("fsqrt_v")), 0x6ee1_fa25);
    assert_eq!(encode_one(&two("frintp_v")), 0x4ee1_8a25);
    assert_eq!(encode_one(&two("frintm_v")), 0x4e61_9a25);
    assert_eq!(encode_one(&two("frinta_v")), 0x6e61_8a25);
    assert_eq!(encode_one(&two("frintn_v")), 0x4e61_8a25);
    assert_eq!(encode_one(&two("frintz_v")), 0x4ee1_9a25);
    assert_eq!(encode_one(&two("fcvtzs_v")), 0x4ee1_ba25);
    assert_eq!(encode_one(&two("fcvtas_v")), 0x4e61_ca25);
    assert_eq!(encode_one(&two("scvtf_v")), 0x4e61_da25);
    assert_eq!(encode_one(&two("neg_v")), 0x6ee0_ba25);
    assert_eq!(encode_one(&two("abs_v")), 0x4ee0_ba25);
    assert_eq!(encode_one(&two("fcmgt_zero_v")), 0x4ee0_ca25);
    assert_eq!(encode_one(&two("fcmge_zero_v")), 0x6ee0_ca25);
    assert_eq!(encode_one(&two("fcmeq_zero_v")), 0x4ee0_da25);
    assert_eq!(encode_one(&two("fcmlt_zero_v")), 0x4ee0_ea25);
    assert_eq!(encode_one(&two("fcmle_zero_v")), 0x6ee0_da25);

    // Shifted-immediate .2d.
    assert_eq!(
        encode_one(
            &CodeInstruction::new("shl_v")
                .physical_field("dst", "v5")
                .physical_field("src", "v17")
                .field("shift", "32")
        ),
        0x4f60_5625
    );
    assert_eq!(
        encode_one(
            &CodeInstruction::new("sshr_v")
                .physical_field("dst", "v5")
                .physical_field("src", "v17")
                .field("shift", "32")
        ),
        0x4f60_0625
    );
    assert_eq!(
        encode_one(
            &CodeInstruction::new("ushr_v")
                .physical_field("dst", "v5")
                .physical_field("src", "v17")
                .field("shift", "20")
        ),
        0x6f6c_0625
    );

    // Lane broadcast / extract.
    assert_eq!(
        encode_one(
            &CodeInstruction::new("dup_v_from_x")
                .physical_field("dst", "v5")
                .physical_field("src", "x17")
        ),
        0x4e08_0e25
    );
    assert_eq!(
        encode_one(
            &CodeInstruction::new("umov_x_from_v")
                .physical_field("dst", "x5")
                .physical_field("src", "v17")
                .field("index", "0")
        ),
        0x4e08_3e25
    );
    assert_eq!(
        encode_one(
            &CodeInstruction::new("umov_x_from_v")
                .physical_field("dst", "x5")
                .physical_field("src", "v17")
                .field("index", "1")
        ),
        0x4e18_3e25
    );

    // Scalar fused multiply-add family (dst=d5, addend=d3, lhs=d9, rhs=d17).
    // The neutral mnemonic names the result; the AArch64 instruction that computes
    // it can carry a different name (fmsub_d→FNMSUB, fnmsub_d→FMSUB). Checked
    // against `as -arch arm64`.
    assert_eq!(
        encode_one(
            &CodeInstruction::new("fmadd_d") // FMADD: d3 + d9*d17
                .physical_field("dst", "d5")
                .physical_field("addend", "d3")
                .physical_field("lhs", "d9")
                .physical_field("rhs", "d17")
        ),
        0x1f51_0d25
    );
    assert_eq!(
        encode_one(
            &CodeInstruction::new("fmsub_d") // FNMSUB: d9*d17 - d3
                .physical_field("dst", "d5")
                .physical_field("addend", "d3")
                .physical_field("lhs", "d9")
                .physical_field("rhs", "d17")
        ),
        0x1f71_8d25
    );
    assert_eq!(
        encode_one(
            &CodeInstruction::new("fnmsub_d") // FMSUB: d3 - d9*d17
                .physical_field("dst", "d5")
                .physical_field("addend", "d3")
                .physical_field("lhs", "d9")
                .physical_field("rhs", "d17")
        ),
        0x1f51_8d25
    );
    assert_eq!(
        encode_one(
            &CodeInstruction::new("fnmadd_d") // FNMADD: -(d9*d17) - d3
                .physical_field("dst", "d5")
                .physical_field("addend", "d3")
                .physical_field("lhs", "d9")
                .physical_field("rhs", "d17")
        ),
        0x1f71_0d25
    );

    // 128-bit load/store, with and without an offset.
    assert_eq!(
        encode_one(
            &CodeInstruction::new("ldr_q")
                .physical_field("dst", "v5")
                .physical_field("base", "x9")
                .field("offset", "0")
        ),
        0x3dc0_0125
    );
    assert_eq!(
        encode_one(
            &CodeInstruction::new("str_q")
                .physical_field("src", "v5")
                .physical_field("base", "x9")
                .field("offset", "16")
        ),
        0x3d80_0525
    );
}

#[test]
fn codeop_sizes_cover_large_add_imm() {
    let add = CodeInstruction {
        op: CodeOp::AddImm,
        fields: vec![
            ("dst", Operand::from("x0")),
            ("src", Operand::from("sp")),
            ("imm", Operand::from("6400")),
        ],
        source: None,
    };

    assert_eq!(instruction_size(&add).unwrap(), 8);
}

#[test]
fn every_scalar_op_encodes() {
    // One representative case per emitter arm — drives the dispatch and each
    // emit_* helper (byte-exactness for the arithmetic core is checked elsewhere).
    let three = |op: &str| emit_words(op, &[("dst", "x0"), ("lhs", "x1"), ("rhs", "x2")]);
    for op in [
        "add", "adds", "sub", "subs", "and", "orr", "eor", "mul", "smulh", "umulh", "rorv", "lslv",
        "lsrv", "asrv", "sdiv", "udiv",
    ] {
        assert_eq!(three(op).len(), 1);
    }
    // rorv_w (32-bit).
    assert_eq!(three("rorv_w").len(), 1);
    // Two-register ops.
    for op in ["mov", "mvn", "clz", "rbit", "rev_w", "rev_x"] {
        assert_eq!(emit_words(op, &[("dst", "x0"), ("src", "x1")]).len(), 1);
    }
    // msub has a minuend.
    assert_eq!(
        emit_words(
            "msub",
            &[
                ("dst", "x0"),
                ("lhs", "x1"),
                ("rhs", "x2"),
                ("minuend", "x3")
            ]
        )
        .len(),
        1
    );
    // Shift-immediate ops.
    for op in ["lsl_imm", "lsr_imm", "asr_imm"] {
        assert_eq!(
            emit_words(op, &[("dst", "x0"), ("src", "x1"), ("shift", "3")]).len(),
            1
        );
    }
    // Compare / immediate arithmetic.
    assert_eq!(emit_words("cmp", &[("lhs", "x0"), ("rhs", "x1")]).len(), 1);
    assert_eq!(
        emit_words("cmp_imm", &[("lhs", "x0"), ("rhs", "5")]).len(),
        1
    );
    assert_eq!(
        emit_words("mov_imm", &[("dst", "x0"), ("value", "5")]).len(),
        1
    );
    // sp adjustments.
    assert_eq!(emit_words("sub_sp", &[("imm", "16")]).len(), 1);
    assert_eq!(emit_words("add_sp", &[("imm", "16")]).len(), 1);
    // Fixed-word ops.
    assert_eq!(emit_words("svc", &[]), [0xd400_0001]);
    assert_eq!(emit_words("branch_self", &[]), [0x1400_0000]);
    assert_eq!(emit_words("ret", &[]), [0xd65f_03c0]);
    assert_eq!(emit_words("blr", &[("register", "x1")]).len(), 1);
}

#[test]
fn memory_ops_encode_all_widths() {
    for op in [
        "ldr_u64", "ldr_u32", "ldr_u16", "ldr_u8", "str_u64", "str_u32", "str_u16", "str_u8",
    ] {
        let field = if op.starts_with("ldr") { "dst" } else { "src" };
        assert_eq!(
            emit_words(op, &[(field, "x0"), ("base", "x1"), ("offset", "0")]).len(),
            1
        );
    }
    // FP scalar spill/reload.
    assert_eq!(
        emit_words("ldr_d", &[("dst", "d0"), ("base", "x1"), ("offset", "0")]).len(),
        1
    );
    assert_eq!(
        emit_words("str_d", &[("src", "d0"), ("base", "x1"), ("offset", "0")]).len(),
        1
    );
}

/// plan-50-D: `STRH Wt, [Xn, #imm12*2]`, the store counterpart of `LDRH`.
/// Asserted as exact words against the ARM ARM, not merely "encodes without
/// error" — a wrong width or opcode still produces a valid instruction.
#[test]
fn str_u16_encodes_strh() {
    // STRH w0, [x1, #0]  ->  0x79000020
    assert_eq!(
        emit_words("str_u16", &[("src", "x0"), ("base", "x1"), ("offset", "0")]),
        [0x7900_0020]
    );
    // STRH w2, [x3, #4]  -> imm12 = 4/2 = 2  ->  0x79000862
    assert_eq!(
        emit_words("str_u16", &[("src", "x2"), ("base", "x3"), ("offset", "4")]),
        [0x7900_0862]
    );
    // It must differ from LDRH by exactly the load bit (0x00400000).
    let strh = emit_words("str_u16", &[("src", "x0"), ("base", "x1"), ("offset", "2")])[0];
    let ldrh = emit_words("ldr_u16", &[("dst", "x0"), ("base", "x1"), ("offset", "2")])[0];
    assert_eq!(ldrh - strh, 0x0040_0000);
    // ...and from STR (32-bit) by exactly the size field (bits 31:30): STRH is
    // size=01, STR(32) is size=10.
    let str32 = emit_words("str_u32", &[("src", "x0"), ("base", "x1"), ("offset", "4")])[0];
    assert_eq!(str32 & 0xc000_0000, 0x8000_0000);
    assert_eq!(strh & 0xc000_0000, 0x4000_0000);
}

#[test]
fn memory_ops_use_scratch_for_large_offsets() {
    // An offset beyond the scaled imm12 ceiling materializes an address in a
    // scratch register first (2 words), for every width.
    let big = "40000";
    for op in [
        "ldr_u64", "str_u64", "ldr_u32", "str_u32", "ldr_u16", "str_u16", "ldr_u8", "str_u8",
        "ldr_d", "str_d",
    ] {
        let field = if op.starts_with("ldr") { "dst" } else { "src" };
        let regv = if op.ends_with('d') { "d0" } else { "x0" };
        let words = emit_words(op, &[(field, regv), ("base", "x1"), ("offset", big)]);
        assert!(
            words.len() >= 2,
            "{op} large offset should use a scratch address"
        );
    }
}

#[test]
fn unaligned_memory_offsets_error() {
    let mut enc = fresh_encoder();
    let bad = |op: &str, field: &'static str, r: &str| {
        CodeInstruction::new(op)
            .physical_field(field, r)
            .physical_field("base", "x1")
            .field("offset", "3")
    };
    assert!(enc.emit_instruction(&bad("ldr_u64", "dst", "x0")).is_err());
    assert!(enc.emit_instruction(&bad("str_u64", "src", "x0")).is_err());
    assert!(enc.emit_instruction(&bad("ldr_u32", "dst", "x0")).is_err());
    assert!(enc.emit_instruction(&bad("str_u32", "src", "x0")).is_err());
    assert!(enc.emit_instruction(&bad("ldr_u16", "dst", "x0")).is_err());
    assert!(enc.emit_instruction(&bad("str_u16", "src", "x0")).is_err());
    assert!(enc.emit_instruction(&bad("ldr_d", "dst", "d0")).is_err());
    assert!(enc.emit_instruction(&bad("str_d", "src", "d0")).is_err());
}

#[test]
fn float_scalar_ops_encode() {
    let three = |op: &str| emit_words(op, &[("dst", "d0"), ("lhs", "d1"), ("rhs", "d2")]);
    for op in ["fadd_d", "fsub_d", "fmul_d", "fdiv_d"] {
        assert_eq!(three(op).len(), 1);
    }
    for op in ["fmov_d_from_d", "fneg_d", "fabs_d", "fsqrt_d"] {
        assert_eq!(emit_words(op, &[("dst", "d0"), ("src", "d1")]).len(), 1);
    }
    assert_eq!(
        emit_words("fmov_x_from_d", &[("dst", "x0"), ("src", "d1")]).len(),
        1
    );
    assert_eq!(
        emit_words("fmov_d_from_x", &[("dst", "d0"), ("src", "x1")]).len(),
        1
    );
    assert_eq!(
        emit_words("fcmp_d", &[("lhs", "d0"), ("rhs", "d1")]).len(),
        1
    );
    assert_eq!(emit_words("fcmp_zero_d", &[("src", "d0")]).len(), 1);
    assert_eq!(
        emit_words("scvtf_d_from_x", &[("dst", "d0"), ("src", "x1")]).len(),
        1
    );
    for op in [
        "fcvtzs_x_from_d",
        "fcvtms_x_from_d",
        "fcvtps_x_from_d",
        "fcvtas_x_from_d",
    ] {
        assert_eq!(emit_words(op, &[("dst", "x0"), ("src", "d1")]).len(), 1);
    }
}

#[test]
fn neon_shift_and_umov_index() {
    // The `shift` operand parser caps values below 64, so drive the in-range
    // shifted-immediate NEON ops (shl 0..=63, sshr/ushr 1..=63).
    assert_eq!(
        emit_words("shl_v", &[("dst", "v0"), ("src", "v1"), ("shift", "63")]).len(),
        1
    );
    for op in ["sshr_v", "ushr_v"] {
        assert_eq!(
            emit_words(op, &[("dst", "v0"), ("src", "v1"), ("shift", "1")]).len(),
            1
        );
    }
    // umov lane index 0/1 valid; 2 out of range.
    assert!(fresh_encoder()
        .emit_instruction(
            &CodeInstruction::new("umov_x_from_v")
                .physical_field("dst", "x0")
                .physical_field("src", "v1")
                .field("index", "1")
        )
        .is_ok());
    let mut enc = fresh_encoder();
    let err = enc
        .emit_instruction(
            &CodeInstruction::new("umov_x_from_v")
                .physical_field("dst", "x0")
                .physical_field("src", "v1")
                .field("index", "2"),
        )
        .unwrap_err();
    assert!(err.contains("lane index"), "got: {err}");
}

#[test]
fn ldr_q_str_q_scratch_for_unaligned_or_large_offset() {
    // 16-aligned in range → one word; non-16-aligned/large → scratch address.
    assert_eq!(
        emit_words("ldr_q", &[("dst", "v0"), ("base", "x1"), ("offset", "16")]).len(),
        1
    );
    assert!(emit_words("ldr_q", &[("dst", "v0"), ("base", "x1"), ("offset", "8")]).len() >= 2);
    assert!(emit_words("str_q", &[("src", "v0"), ("base", "x1"), ("offset", "8")]).len() >= 2);
}

/// bug-788: the binary32 conversions a `LINK` `CFloat` crosses.
#[test]
fn float_precision_conversions_encode() {
    // fcvt s0, d1
    assert_eq!(
        emit_words("fcvt_s_from_d", &[("dst", "d0"), ("src", "d1")]),
        [0x1E62_4020]
    );
    // fcvt d0, s1
    assert_eq!(
        emit_words("fcvt_d_from_s", &[("dst", "d0"), ("src", "d1")]),
        [0x1E22_C020]
    );
}
