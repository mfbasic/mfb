use super::*;
use crate::arch::aarch64::ops::CodeOp;
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
fn encodes_umulh_adc_and_rorv() {
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
    encoder
        .emit_instruction(
            &CodeInstruction::new("adc")
                .field("dst", "x10")
                .field("lhs", "x14")
                .field("rhs", "x12"),
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
    expected.extend_from_slice(&0x9bc9_7d6e_u32.to_le_bytes());
    expected.extend_from_slice(&0x9a0c_01ca_u32.to_le_bytes());
    expected.extend_from_slice(&0x9acb_2d80_u32.to_le_bytes());
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
    let two = |op: &str| CodeInstruction::new(op).field("dst", "v5").field("src", "v17");

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

    // Scalar fused multiply-add (dst=d5, addend=d3, lhs=d9, rhs=d17).
    assert_eq!(
        encode_one(
            &CodeInstruction::new("fmadd_d")
                .field("dst", "d5")
                .field("addend", "d3")
                .field("lhs", "d9")
                .field("rhs", "d17")
        ),
        0x1f51_0d25
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
