use super::*;
use crate::arch::ops::CodeOp;
use crate::target::shared::code::CodeInstruction;

#[test]
fn instruction_size_expands_large_stack_and_memory_immediates() {
    let sub_sp = CodeInstruction::new("sub_sp").field("imm", "6400");
    let ldr = CodeInstruction::new("ldr_u64")
        .field("dst", "x0")
        .field("base", "sp")
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
                .field("src", "x0")
                .field("base", "sp")
                .field("offset", "32800"),
        )
        .unwrap();
    encoder
        .emit_instruction(
            &CodeInstruction::new("cmp_imm")
                .field("lhs", "x1")
                .field("rhs", "6400"),
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
                .field("dst", "x14")
                .field("lhs", "x11")
                .field("rhs", "x9"),
        )
        .unwrap();
    // Explicit-carry add, no carry-in (plan-00-G §4) → `adds; cset` (2 words).
    // It must NOT use `cmp xzr,#1` (x31 = SP in the immediate form, a real bug
    // this guards: it would compute SP-1 and wrongly add 1).
    encoder
        .emit_instruction(
            &CodeInstruction::new("add_carry")
                .field("dst", "x10")
                .field("carry_out", "x11")
                .field("lhs", "x14")
                .field("rhs", "x12")
                .field("carry_in", "xzr"),
        )
        .unwrap();
    // Explicit-carry add with a carry-in register → `cmp; adcs; cset` (3 words).
    encoder
        .emit_instruction(
            &CodeInstruction::new("add_carry")
                .field("dst", "x13")
                .field("carry_out", "xzr")
                .field("lhs", "x15")
                .field("rhs", "x16")
                .field("carry_in", "x11"),
        )
        .unwrap();
    encoder
        .emit_instruction(
            &CodeInstruction::new("rorv")
                .field("dst", "x0")
                .field("lhs", "x12")
                .field("rhs", "x11"),
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
            .field("dst", "d5")
            .field("src", "d3"),
        CodeInstruction::new("fmov_d_from_d")
            .field("dst", "d8")
            .field("src", "d0"),
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
            .field("dst", "x0")
            .field("src", "x0"),
        CodeInstruction::new("sxtw")
            .field("dst", "x3")
            .field("src", "x1"),
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
                .field("dst", "d5")
                .field("src", "d3")
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
                .field("dst", "d5")
                .field("lhs", "d9")
                .field("rhs", "d17")
        ),
        0x1e71_7925
    );
    assert_eq!(
        encode_one(
            &CodeInstruction::new("fmaxnm_d")
                .field("dst", "d5")
                .field("lhs", "d9")
                .field("rhs", "d17")
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
            .field("src", "d0")
            .field("base", "sp")
            .field("offset", "0"),
        CodeInstruction::new("ldr_d")
            .field("dst", "d0")
            .field("base", "sp")
            .field("offset", "8"),
        CodeInstruction::new("str_d")
            .field("src", "d8")
            .field("base", "sp")
            .field("offset", "16"),
        CodeInstruction::new("ldr_d")
            .field("dst", "d15")
            .field("base", "x9")
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

fn fresh_encoder() -> Encoder {
    Encoder {
        text: Vec::new(),
        data: Vec::new(),
        symbols: Vec::new(),
        relocations: Vec::new(),
        imports: HashMap::new(),
        labels: HashMap::new(),
        patches: Vec::new(),
    }
}

fn encode_one(instruction: &CodeInstruction) -> u32 {
    let mut encoder = fresh_encoder();
    encoder.emit_instruction(instruction).unwrap();
    assert_eq!(encoder.text.len(), 4, "expected a single 4-byte word");
    u32::from_le_bytes(encoder.text[..4].try_into().unwrap())
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
            .field("dst", "v5")
            .field("lhs", "v9")
            .field("rhs", "v17")
    };
    let two = |op: &str| {
        CodeInstruction::new(op)
            .field("dst", "v5")
            .field("src", "v17")
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
                .field("dst", "v5")
                .field("src", "v17")
                .field("shift", "32")
        ),
        0x4f60_5625
    );
    assert_eq!(
        encode_one(
            &CodeInstruction::new("sshr_v")
                .field("dst", "v5")
                .field("src", "v17")
                .field("shift", "32")
        ),
        0x4f60_0625
    );
    assert_eq!(
        encode_one(
            &CodeInstruction::new("ushr_v")
                .field("dst", "v5")
                .field("src", "v17")
                .field("shift", "20")
        ),
        0x6f6c_0625
    );

    // Lane broadcast / extract.
    assert_eq!(
        encode_one(
            &CodeInstruction::new("dup_v_from_x")
                .field("dst", "v5")
                .field("src", "x17")
        ),
        0x4e08_0e25
    );
    assert_eq!(
        encode_one(
            &CodeInstruction::new("umov_x_from_v")
                .field("dst", "x5")
                .field("src", "v17")
                .field("index", "0")
        ),
        0x4e08_3e25
    );
    assert_eq!(
        encode_one(
            &CodeInstruction::new("umov_x_from_v")
                .field("dst", "x5")
                .field("src", "v17")
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
                .field("dst", "d5")
                .field("addend", "d3")
                .field("lhs", "d9")
                .field("rhs", "d17")
        ),
        0x1f51_0d25
    );
    assert_eq!(
        encode_one(
            &CodeInstruction::new("fmsub_d") // FNMSUB: d9*d17 - d3
                .field("dst", "d5")
                .field("addend", "d3")
                .field("lhs", "d9")
                .field("rhs", "d17")
        ),
        0x1f71_8d25
    );
    assert_eq!(
        encode_one(
            &CodeInstruction::new("fnmsub_d") // FMSUB: d3 - d9*d17
                .field("dst", "d5")
                .field("addend", "d3")
                .field("lhs", "d9")
                .field("rhs", "d17")
        ),
        0x1f51_8d25
    );
    assert_eq!(
        encode_one(
            &CodeInstruction::new("fnmadd_d") // FNMADD: -(d9*d17) - d3
                .field("dst", "d5")
                .field("addend", "d3")
                .field("lhs", "d9")
                .field("rhs", "d17")
        ),
        0x1f71_0d25
    );

    // 128-bit load/store, with and without an offset.
    assert_eq!(
        encode_one(
            &CodeInstruction::new("ldr_q")
                .field("dst", "v5")
                .field("base", "x9")
                .field("offset", "0")
        ),
        0x3dc0_0125
    );
    assert_eq!(
        encode_one(
            &CodeInstruction::new("str_q")
                .field("src", "v5")
                .field("base", "x9")
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
            ("dst", "x0".to_string()),
            ("src", "sp".to_string()),
            ("imm", "6400".to_string()),
        ],
    };

    assert_eq!(instruction_size(&add).unwrap(), 8);
}

/// Emit one instruction and return its 4-byte words (asserting size==emit).
fn emit_words(op: &str, fields: &[(&'static str, &str)]) -> Vec<u32> {
    let mut inst = CodeInstruction::new(op);
    for (k, v) in fields {
        inst = inst.field(k, v);
    }
    let mut enc = fresh_encoder();
    enc.emit_instruction(&inst).unwrap();
    assert_eq!(
        enc.text.len(),
        instruction_size(&inst).unwrap(),
        "size/emit mismatch for '{op}'"
    );
    enc.text
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
        .collect()
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
            .field(field, r)
            .field("base", "x1")
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
                .field("dst", "x0")
                .field("src", "v1")
                .field("index", "1")
        )
        .is_ok());
    let mut enc = fresh_encoder();
    let err = enc
        .emit_instruction(
            &CodeInstruction::new("umov_x_from_v")
                .field("dst", "x0")
                .field("src", "v1")
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

#[test]
fn add_carry_and_sub_borrow_sizes() {
    // No carry-in → adds; cset (2 words).
    assert_eq!(
        emit_words(
            "add_carry",
            &[
                ("dst", "x0"),
                ("carry_out", "x1"),
                ("lhs", "x2"),
                ("rhs", "x3"),
                ("carry_in", "xzr")
            ]
        )
        .len(),
        2
    );
    // Carry-in register → cmp; adcs; cset (3 words).
    assert_eq!(
        emit_words(
            "add_carry",
            &[
                ("dst", "x0"),
                ("carry_out", "x1"),
                ("lhs", "x2"),
                ("rhs", "x3"),
                ("carry_in", "x4")
            ]
        )
        .len(),
        3
    );
    // sub_borrow is always subs; sbcs; cset (3 words).
    assert_eq!(
        emit_words(
            "sub_borrow",
            &[
                ("dst", "x0"),
                ("borrow_out", "x1"),
                ("lhs", "x2"),
                ("rhs", "x3"),
                ("borrow_in", "xzr")
            ]
        )
        .len(),
        3
    );
    // The no-carry-in predicate is the resolved register (31), not the spelling:
    // "sp"/"x31" must size the same as "xzr" (emit_words asserts size == emit).
    for spelling in ["xzr", "sp", "x31"] {
        assert_eq!(
            emit_words(
                "add_carry",
                &[
                    ("dst", "x0"),
                    ("carry_out", "x1"),
                    ("lhs", "x2"),
                    ("rhs", "x3"),
                    ("carry_in", spelling)
                ]
            )
            .len(),
            2
        );
    }
}

#[test]
fn cmp_imm_size_matches_emit_for_out_of_range_immediates() {
    // `emit_words` asserts instruction_size == emitted bytes. Out of imm12 range
    // `emit_cmp_imm` is `mov_imm` (1–4 words) + `cmp`, not the add/sub chunking.
    assert_eq!(
        emit_words("cmp_imm", &[("lhs", "x0"), ("rhs", "4095")]).len(),
        1
    );
    assert_eq!(
        emit_words("cmp_imm", &[("lhs", "x0"), ("rhs", "4096")]).len(),
        2
    );
    assert_eq!(
        emit_words("cmp_imm", &[("lhs", "x0"), ("rhs", "4294967296")]).len(),
        3
    );
    assert_eq!(
        emit_words("cmp_imm", &[("lhs", "x0"), ("rhs", "18446744073709551615")]).len(),
        5
    );
}

#[test]
fn all_conditional_branches_patch() {
    // Every conditional-branch condition patches its imm19 field distinctly.
    for op in [
        "b.eq", "b.ne", "b.ge", "b.lt", "b.gt", "b.le", "b.vc", "b.vs", "b.hi", "b.lo", "b.mi",
        "b.ls",
    ] {
        let mut enc = fresh_encoder();
        enc.labels.insert("L".to_string(), 4);
        enc.emit_instruction(&CodeInstruction::new(op).field("target", "L"))
            .unwrap();
        enc.patch_labels().unwrap();
        let word = u32::from_le_bytes(enc.text[..4].try_into().unwrap());
        assert_eq!(word >> 24, 0x54, "{op} should be a conditional branch");
    }
    // Unconditional `b` uses imm26.
    let mut enc = fresh_encoder();
    enc.labels.insert("L".to_string(), 8);
    enc.emit_instruction(&CodeInstruction::new("b").field("target", "L"))
        .unwrap();
    enc.patch_labels().unwrap();
    let word = u32::from_le_bytes(enc.text[..4].try_into().unwrap());
    assert_eq!(word & 0xfc00_0000, 0x1400_0000);
}

#[test]
fn unresolved_branch_label_errors() {
    let mut enc = fresh_encoder();
    enc.emit_instruction(&CodeInstruction::new("b.eq").field("target", "gone"))
        .unwrap();
    assert!(enc.patch_labels().unwrap_err().contains("does not resolve"));
}

#[test]
fn bl_relocation_bindings() {
    // Internal: a known text symbol.
    let mut enc = fresh_encoder();
    enc.symbols.push(EncodedSymbol {
        name: "_mfb_fn".to_string(),
        section: EncodedSection::Text,
        offset: 0,
    });
    enc.emit_instruction(&CodeInstruction::new("bl").field("target", "_mfb_fn"))
        .unwrap();
    assert_eq!(enc.relocations[0].binding, "internal");
    // External: an imported symbol.
    let mut enc = fresh_encoder();
    enc.imports.insert("puts".to_string(), "libc".to_string());
    enc.emit_instruction(&CodeInstruction::new("bl").field("target", "puts"))
        .unwrap();
    assert_eq!(enc.relocations[0].binding, "external");
    // Unresolved.
    let mut enc = fresh_encoder();
    assert!(enc
        .emit_instruction(&CodeInstruction::new("bl").field("target", "nope"))
        .unwrap_err()
        .contains("does not resolve"));
}

#[test]
fn symbol_ref_data_and_got_relocations() {
    // adrp / add_pageoff for an internal data symbol → "data" binding, hi/lo kinds.
    let mut enc = fresh_encoder();
    enc.emit_instruction(
        &CodeInstruction::new("adrp")
            .field("dst", "x0")
            .field("symbol", "g"),
    )
    .unwrap();
    enc.emit_instruction(
        &CodeInstruction::new("add_pageoff")
            .field("dst", "x0")
            .field("src", "x0")
            .field("symbol", "g"),
    )
    .unwrap();
    assert_eq!(enc.relocations.len(), 2);
    assert!(enc.relocations.iter().all(|r| r.binding == "data"));
    // Imported symbol → GOT load, "external" binding.
    let mut enc = fresh_encoder();
    enc.imports.insert("g".to_string(), "libc".to_string());
    enc.emit_instruction(
        &CodeInstruction::new("adrp")
            .field("dst", "x0")
            .field("symbol", "g"),
    )
    .unwrap();
    enc.emit_instruction(
        &CodeInstruction::new("add_pageoff")
            .field("dst", "x0")
            .field("src", "x0")
            .field("symbol", "g"),
    )
    .unwrap();
    assert!(enc.relocations.iter().all(|r| r.binding == "external"));
}

#[test]
fn unsupported_instruction_errors() {
    // A NEON op the AArch64 encoder does support drives the three-same path; an
    // unsupported one (fmadd is scalar, handled) — use a genuinely unknown path by
    // constructing an op the dispatch does not cover via the fallback: there is
    // none reachable through `CodeInstruction::new`, so assert the three/two/shift
    // classifier errors instead for a mismatched op.
    let mut enc = fresh_encoder();
    // fmadd_d needs an addend field; missing it errors.
    let err = enc
        .emit_instruction(
            &CodeInstruction::new("fmadd_d")
                .field("dst", "d0")
                .field("lhs", "d1")
                .field("rhs", "d2"),
        )
        .unwrap_err();
    assert!(err.contains("missing field"), "got: {err}");
}

#[test]
fn shift_immediate_out_of_range_errors() {
    // lsl/lsr/asr immediate shift ≥ 64 is rejected by the shift operand parser.
    for op in ["lsl_imm", "lsr_imm", "asr_imm"] {
        let mut enc = fresh_encoder();
        assert!(enc
            .emit_instruction(
                &CodeInstruction::new(op)
                    .field("dst", "x0")
                    .field("src", "x1")
                    .field("shift", "64")
            )
            .is_err());
    }
}

fn plan_fn(
    symbol: &str,
    instructions: Vec<CodeInstruction>,
) -> crate::target::shared::code::CodeFunction {
    crate::target::shared::code::CodeFunction {
        name: symbol.to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: crate::target::shared::code::CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        instructions,
        relocations: Vec::new(),
        stack_slots: Vec::new(),
    }
}

fn plan(
    functions: Vec<crate::target::shared::code::CodeFunction>,
    data_objects: Vec<crate::target::shared::code::CodeDataObject>,
    imports: Vec<crate::target::shared::code::CodeImport>,
    entry: Option<&str>,
) -> crate::target::shared::code::NativeCodePlan {
    crate::target::shared::code::NativeCodePlan {
        target: "linux-aarch64".to_string(),
        build_mode: crate::target::NativeBuildMode::Console,
        arch: "aarch64".to_string(),
        project: "t".to_string(),
        entry_symbol: entry.map(str::to_string),
        imports,
        data_objects,
        functions,
    }
}

fn data_obj(
    symbol: &str,
    kind: &str,
    value: &str,
    align: usize,
    size: usize,
) -> crate::target::shared::code::CodeDataObject {
    crate::target::shared::code::CodeDataObject {
        symbol: symbol.to_string(),
        kind: kind.to_string(),
        layout: "bytes".to_string(),
        align,
        size,
        value: value.to_string(),
    }
}

#[test]
fn encode_builds_image_with_symbols_data_and_labels() {
    let func = plan_fn(
        "main",
        vec![
            crate::arch::aarch64::abi::label("entry"),
            CodeInstruction::new("mov")
                .field("dst", "x0")
                .field("src", "x1"),
            CodeInstruction::new("b").field("target", "entry"),
            CodeInstruction::new("ret"),
        ],
    );
    let image = super::encode(&plan(
        vec![func],
        vec![
            data_obj("g", "string", "hi", 8, 16),
            data_obj("r", "raw", "de ad", 1, 2),
        ],
        Vec::new(),
        Some("main"),
    ))
    .expect("encode");
    assert_eq!(image.entry, "main");
    assert!(image.symbols.iter().any(|s| s.name == "main"));
    assert!(image.symbols.iter().any(|s| s.name == "g"));
    assert!(image.data.windows(2).any(|w| w == [0xDE, 0xAD]));
}

#[test]
fn encode_requires_entry_and_rejects_bad_hex() {
    // Missing entry symbol.
    let err = match super::encode(&plan(
        vec![plan_fn("f", vec![CodeInstruction::new("ret")])],
        Vec::new(),
        Vec::new(),
        None,
    )) {
        Ok(_) => panic!("expected error"),
        Err(e) => e,
    };
    assert!(err.contains("entry symbol"), "got: {err}");
    // Odd hex digit count.
    let err = match super::encode(&plan(
        Vec::new(),
        vec![data_obj("r", "raw", "abc", 1, 1)],
        Vec::new(),
        Some("main"),
    )) {
        Ok(_) => panic!("expected error"),
        Err(e) => e,
    };
    assert!(err.contains("even digit"), "got: {err}");
    // Non-hex digit.
    let err = match super::encode(&plan(
        Vec::new(),
        vec![data_obj("r", "raw", "zz", 1, 1)],
        Vec::new(),
        Some("main"),
    )) {
        Ok(_) => panic!("expected error"),
        Err(e) => e,
    };
    assert!(err.contains("non-hex"), "got: {err}");
}

#[test]
fn encode_rejects_a_duplicate_label_within_a_function() {
    // The full two-pass `encode` (symbol layout + per-function label pass) must
    // reject a repeated label rather than last-writer-wins silently resolving every
    // reference to the final definition (bug-127). This drives the size-accumulation
    // first pass, the label-insert branch, and the duplicate-label error return.
    let func = plan_fn(
        "main",
        vec![
            crate::arch::aarch64::abi::label("loop"),
            CodeInstruction::new("ret"),
            crate::arch::aarch64::abi::label("loop"),
            CodeInstruction::new("ret"),
        ],
    );
    let err = match super::encode(&plan(vec![func], Vec::new(), Vec::new(), Some("main"))) {
        Ok(_) => panic!("expected a duplicate-label error"),
        Err(e) => e,
    };
    assert!(err.contains("duplicate label"), "got: {err}");
    assert!(err.contains("'loop'"), "got: {err}");
}

#[test]
fn encode_carries_imports_as_functions() {
    let func = plan_fn(
        "main",
        vec![
            CodeInstruction::new("bl").field("target", "puts"),
            CodeInstruction::new("ret"),
        ],
    );
    let image = super::encode(&plan(
        vec![func],
        Vec::new(),
        vec![crate::target::shared::code::CodeImport {
            library: "libc".to_string(),
            symbol: "puts".to_string(),
        }],
        Some("main"),
    ))
    .expect("encode");
    let import = image
        .imports
        .iter()
        .find(|i| i.symbol == "puts")
        .expect("import present");
    assert!(matches!(import.kind, ImportKind::Function));
    assert_eq!(import.version, None);
}

#[test]
fn operand_reg_decoding() {
    use super::operand::{immediate, reg, scratch_excluding, shift, vreg};
    // sp/xzr/x31 all map to 31; w-aliases decode like their x-registers.
    assert_eq!(reg("sp".to_string()).unwrap(), 31);
    assert_eq!(reg("xzr".to_string()).unwrap(), 31);
    assert_eq!(reg("raw_sp".to_string()).unwrap(), 31);
    assert_eq!(reg("w0".to_string()).unwrap(), 0);
    assert_eq!(reg("x19".to_string()).unwrap(), 19);
    assert_eq!(reg("x30".to_string()).unwrap(), 30);
    assert_eq!(reg("lr".to_string()).unwrap(), 30);
    // Scalar FP `dN` decodes to its number.
    assert_eq!(reg("d5".to_string()).unwrap(), 5);
    assert!(reg("d99".to_string()).is_err());
    assert!(reg("bogus".to_string()).is_err());
    // vreg accepts v/q/d spellings and rejects out-of-range / non-vector names.
    assert_eq!(vreg("v3".to_string()).unwrap(), 3);
    assert_eq!(vreg("q7".to_string()).unwrap(), 7);
    assert_eq!(vreg("d31".to_string()).unwrap(), 31);
    assert!(vreg("v32".to_string()).is_err());
    assert!(vreg("v9x".to_string()).is_err());
    assert!(vreg("x0".to_string()).is_err());
    // immediate booleans + parse error.
    assert_eq!(immediate("true".to_string()).unwrap(), 1);
    assert_eq!(immediate("false".to_string()).unwrap(), 0);
    assert!(immediate("bad".to_string()).is_err());
    // shift bounds.
    assert_eq!(shift("0".to_string()).unwrap(), 0);
    assert!(shift("64".to_string()).is_err());
    assert!(shift("q".to_string()).is_err());
    // scratch_excluding avoids the two given registers.
    let s = scratch_excluding(17, 16);
    assert_eq!(s, 15);
    assert_ne!(scratch_excluding(15, 16), 15);
}

#[test]
fn operand_field_missing_reports_op() {
    use super::operand::field;
    let inst = CodeInstruction::new("mov").field("dst", "x0");
    assert!(field(&inst, "src").is_err());
    assert_eq!(field(&inst, "dst").unwrap(), "x0");
}

#[test]
fn sizing_covers_immediate_and_memory_paths() {
    use super::sizing::instruction_size;
    // Label is zero-sized.
    assert_eq!(
        instruction_size(&crate::arch::aarch64::abi::label("L")).unwrap(),
        0
    );
    // mov_imm word count grows with non-zero 16-bit chunks.
    let mk = |v: &str| {
        CodeInstruction::new("mov_imm")
            .field("dst", "x0")
            .field("value", v)
    };
    assert_eq!(instruction_size(&mk("0")).unwrap(), 4);
    assert_eq!(instruction_size(&mk("65536")).unwrap(), 8); // bits in the 2nd chunk
    assert_eq!(instruction_size(&mk("18446744073709551615")).unwrap(), 16); // u64::MAX
                                                                            // add/sub immediate sizing: small (1 word), large multi-chunk.
    let addi = |v: &str| {
        CodeInstruction::new("add_imm")
            .field("dst", "x0")
            .field("src", "x1")
            .field("imm", v)
    };
    assert_eq!(instruction_size(&addi("1")).unwrap(), 4);
    assert_eq!(instruction_size(&addi("0")).unwrap(), 4);
    assert!(instruction_size(&addi("40000")).unwrap() >= 8);
    // cmp_imm sizing reads the rhs field.
    assert_eq!(
        instruction_size(
            &CodeInstruction::new("cmp_imm")
                .field("lhs", "x0")
                .field("rhs", "1")
        )
        .unwrap(),
        4
    );
    // Memory sizing: aligned in-range = 4; large = scratch + word.
    let ldr = |off: &str| {
        CodeInstruction::new("ldr_u64")
            .field("dst", "x0")
            .field("base", "x1")
            .field("offset", off)
    };
    assert_eq!(instruction_size(&ldr("8")).unwrap(), 4);
    assert!(instruction_size(&ldr("40000")).unwrap() >= 8);
    // u32/u16/u8/q memory widths.
    for (op, field, r) in [
        ("ldr_u32", "dst", "x0"),
        ("ldr_u16", "dst", "x0"),
        ("ldr_u8", "dst", "x0"),
        ("str_u8", "src", "x0"),
        ("ldr_q", "dst", "v0"),
        ("str_q", "src", "v0"),
    ] {
        let inst = CodeInstruction::new(op)
            .field(field, r)
            .field("base", "x1")
            .field("offset", "0");
        assert_eq!(instruction_size(&inst).unwrap(), 4);
    }
    // A plain op (not immediate/memory) is always one word.
    assert_eq!(
        instruction_size(
            &CodeInstruction::new("mov")
                .field("dst", "x0")
                .field("src", "x1")
        )
        .unwrap(),
        4
    );
}

#[test]
fn sizing_helpers_directly() {
    use super::sizing::{
        branch_imm19, branch_imm26, checked_imm12, encode_add_sub_imm, next_add_sub_chunk,
    };
    assert_eq!(checked_imm12(4095).unwrap(), 4095);
    assert!(checked_imm12(4096).is_err());
    // Small immediate (no shift); a 4096-multiple large immediate (shifted).
    assert_eq!(encode_add_sub_imm(100), Some((100, false)));
    assert_eq!(encode_add_sub_imm(4096), Some((1, true)));
    assert_eq!(encode_add_sub_imm(5000), None);
    // Chunking a large remainder.
    let (chunk, shifted) = next_add_sub_chunk(40000);
    assert!(shifted && chunk > 0);
    let (chunk, shifted) = next_add_sub_chunk(100);
    assert!(!shifted && chunk == 100);
    // Branch displacements are word-scaled and masked.
    assert_eq!(branch_imm26(0, 8).unwrap(), 2);
    assert_eq!(branch_imm19(0, 8).unwrap(), 2);
    // A negative delta wraps into the masked field.
    assert_ne!(branch_imm26(8, 0).unwrap(), 0);
    // bug-267: an out-of-reach or misaligned target is an encoder error, not a
    // silently truncated encoding. imm26 reaches ±128 MiB, imm19 only ±1 MiB.
    assert!(branch_imm26(0, 1 << 28).is_err()); // > +128 MiB
    assert!(branch_imm19(0, 1 << 21).is_err()); // > +1 MiB
    assert!(branch_imm19(0, 1 << 19).unwrap() != 0); // in range for imm19
    assert!(branch_imm26(0, 2).is_err()); // unaligned (not a multiple of 4)
}
