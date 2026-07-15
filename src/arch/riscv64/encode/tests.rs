use super::*;
use crate::target::shared::code::{
    CodeDataObject, CodeFrame, CodeFunction, CodeImport, NativeCodePlan,
};
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


// --- Shared plan builders for the broader op/error coverage below --------------

/// A minimal one-function plan whose body is `instructions`, entry `f`.
fn plan_of(instructions: Vec<CodeInstruction>) -> NativeCodePlan {
    NativeCodePlan {
        target: "linux-riscv64".to_string(),
        build_mode: NativeBuildMode::Console,
        arch: "riscv64".to_string(),
        project: "t".to_string(),
        entry_symbol: Some("f".to_string()),
        imports: Vec::new(),
        data_objects: Vec::new(),
        functions: vec![func("f", instructions)],
    }
}

fn func(name: &str, instructions: Vec<CodeInstruction>) -> CodeFunction {
    CodeFunction {
        name: name.to_string(),
        symbol: name.to_string(),
        params: Vec::new(),
        returns: "Integer".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        instructions,
        relocations: Vec::new(),
        stack_slots: Vec::new(),
    }
}

/// Encode a one-function body, returning the encoder's `Result` so error paths
/// can be asserted (mirrors `encode_text` but does not `.expect`).
fn try_encode(instructions: Vec<CodeInstruction>) -> Result<Vec<u8>, String> {
    encode(&plan_of(instructions)).map(|image| image.text)
}

// --- operand.rs: register / immediate / shift decoding -------------------------

#[test]
fn integer_register_names_decode_to_numbers() {
    let table: &[(&str, u8)] = &[
        ("zero", 0), ("ra", 1), ("sp", 2), ("gp", 3), ("tp", 4),
        ("t0", 5), ("t1", 6), ("t2", 7), ("s0", 8), ("fp", 8), ("s1", 9),
        ("a0", 10), ("a1", 11), ("a2", 12), ("a3", 13), ("a4", 14), ("a5", 15),
        ("a6", 16), ("a7", 17), ("s2", 18), ("s3", 19), ("s4", 20), ("s5", 21),
        ("s6", 22), ("s7", 23), ("s8", 24), ("s9", 25), ("s10", 26), ("s11", 27),
        ("t3", 28), ("t4", 29), ("t5", 30), ("t6", 31),
    ];
    for &(name, num) in table {
        assert_eq!(super::operand::reg(name.to_string()).unwrap(), num, "reg {name}");
    }
    assert!(super::operand::reg("x99".to_string())
        .unwrap_err()
        .contains("unknown rv64 integer register"));
}

#[test]
fn fp_register_names_decode_to_numbers() {
    let table: &[(&str, u8)] = &[
        ("ft0", 0), ("ft1", 1), ("ft2", 2), ("ft3", 3), ("ft4", 4), ("ft5", 5),
        ("ft6", 6), ("ft7", 7), ("fs0", 8), ("fs1", 9), ("fa0", 10), ("fa1", 11),
        ("fa2", 12), ("fa3", 13), ("fa4", 14), ("fa5", 15), ("fa6", 16), ("fa7", 17),
        ("fs2", 18), ("fs3", 19), ("fs4", 20), ("fs5", 21), ("fs6", 22), ("fs7", 23),
        ("fs8", 24), ("fs9", 25), ("fs10", 26), ("fs11", 27), ("ft8", 28), ("ft9", 29),
        ("ft10", 30), ("ft11", 31),
    ];
    for &(name, num) in table {
        assert_eq!(super::operand::freg(name.to_string()).unwrap(), num, "freg {name}");
    }
    assert!(super::operand::freg("fx".to_string())
        .unwrap_err()
        .contains("unknown rv64 FP register"));
}

#[test]
fn immediate_and_shift_decoding() {
    assert_eq!(super::operand::immediate("true".to_string()).unwrap(), 1);
    assert_eq!(super::operand::immediate("false".to_string()).unwrap(), 0);
    assert_eq!(super::operand::immediate("42".to_string()).unwrap(), 42);
    assert!(super::operand::immediate("nope".to_string())
        .unwrap_err()
        .contains("invalid rv64 immediate"));

    assert_eq!(super::operand::shift("0".to_string()).unwrap(), 0);
    assert_eq!(super::operand::shift("63".to_string()).unwrap(), 63);
    assert!(super::operand::shift("64".to_string())
        .unwrap_err()
        .contains("out of range"));
    assert!(super::operand::shift("x".to_string())
        .unwrap_err()
        .contains("invalid rv64 shift immediate"));
}

#[test]
fn field_lookup_reports_missing_field() {
    let inst = ci("add", &[("dst", "a0")]);
    assert_eq!(super::operand::field(&inst, "dst").unwrap(), "a0");
    assert!(super::operand::field(&inst, "lhs")
        .unwrap_err()
        .contains("missing field 'lhs'"));
}

// --- data.rs: data objects (string + raw) and hex validation -------------------

#[test]
fn data_objects_string_and_raw_encode_and_symbolize() {
    let mut plan = plan_of(vec![ci("ret", &[])]);
    plan.data_objects = vec![
        CodeDataObject {
            symbol: "greeting".to_string(),
            kind: "string".to_string(),
            layout: String::new(),
            align: 8,
            size: 16,
            value: "hi".to_string(),
        },
        CodeDataObject {
            symbol: "blob".to_string(),
            kind: "raw".to_string(),
            layout: String::new(),
            align: 4,
            size: 4,
            value: "de ad_be ef".to_string(),
        },
    ];
    let image = encode(&plan).unwrap();
    // The string object stores its length as a leading u64, then the bytes + NUL.
    assert_eq!(&image.data[0..8], &2u64.to_le_bytes());
    assert_eq!(&image.data[8..10], b"hi");
    // The raw object's hex (whitespace/underscore-stripped) decodes to bytes.
    // The string object padded to its own align (8) occupies 16 bytes, so the
    // raw object begins at offset 16.
    let raw_start = 16;
    assert_eq!(&image.data[raw_start..raw_start + 4], &[0xde, 0xad, 0xbe, 0xef]);
    // Both data symbols are recorded.
    assert!(image.symbols.iter().any(|s| s.name == "greeting"));
    assert!(image.symbols.iter().any(|s| s.name == "blob"));
}

#[test]
fn raw_data_object_rejects_malformed_hex() {
    let mut plan = plan_of(vec![ci("ret", &[])]);
    plan.data_objects = vec![CodeDataObject {
        symbol: "b".to_string(),
        kind: "raw".to_string(),
        layout: String::new(),
        align: 1,
        size: 1,
        value: "abc".to_string(), // odd digit count
    }];
    assert!(encode(&plan).map(|_| ()).unwrap_err().contains("even digit count"));

    plan.data_objects[0].value = "zz".to_string(); // even, but non-hex
    assert!(encode(&plan).map(|_| ()).unwrap_err().contains("non-hex digit"));
}

// --- mod.rs / emitter.rs: relocations, symbol resolution, labels ---------------

#[test]
fn call_and_data_reference_to_import_and_internal_data() {
    let mut plan = plan_of(vec![
        ci("adrp", &[("dst", "a0"), ("symbol", "msg")]),
        ci("add_pageoff", &[("dst", "a0"), ("symbol", "msg")]),
        ci("bl", &[("target", "puts")]),
        ci("ret", &[]),
    ]);
    plan.imports = vec![CodeImport {
        library: "libc".to_string(),
        symbol: "puts".to_string(),
    }];
    plan.data_objects = vec![CodeDataObject {
        symbol: "msg".to_string(),
        kind: "string".to_string(),
        layout: String::new(),
        align: 8,
        size: 16,
        value: "x".to_string(),
    }];
    let image = encode(&plan).unwrap();
    assert!(image.imports.iter().any(|i| i.symbol == "puts"));
    // The `bl puts` resolves through the import table (external binding).
    assert!(image
        .relocations
        .iter()
        .any(|r| r.target == "puts" && r.binding == "external"));
    // The `adrp/add_pageoff msg` reference an internal data symbol (data binding).
    assert!(image
        .relocations
        .iter()
        .any(|r| r.target == "msg" && r.binding == "data"));
}

#[test]
fn adrp_pageoff_to_import_uses_got_binding() {
    let mut plan = plan_of(vec![
        ci("adrp", &[("dst", "a0"), ("symbol", "errno_location")]),
        ci("add_pageoff", &[("dst", "a0"), ("symbol", "errno_location")]),
        ci("ret", &[]),
    ]);
    plan.imports = vec![CodeImport {
        library: "libc".to_string(),
        symbol: "errno_location".to_string(),
    }];
    let image = encode(&plan).unwrap();
    // Both halves of the GOT load are external relocations.
    assert!(
        image
            .relocations
            .iter()
            .filter(|r| r.target == "errno_location" && r.binding == "external")
            .count()
            >= 2
    );
}

#[test]
fn call_to_internal_function_is_internal_binding() {
    let mut plan = plan_of(vec![ci("bl", &[("target", "g")]), ci("ret", &[])]);
    plan.functions.push(func("g", vec![ci("ret", &[])]));
    let image = encode(&plan).unwrap();
    assert!(image
        .relocations
        .iter()
        .any(|r| r.target == "g" && r.binding == "internal"));
}

#[test]
fn call_to_unresolved_symbol_errors() {
    let err = try_encode(vec![ci("bl", &[("target", "nowhere")])]).unwrap_err();
    assert!(err.contains("does not resolve"), "{err}");
}

#[test]
fn duplicate_label_in_function_errors() {
    let err = try_encode(vec![
        ci("label", &[("name", "loop")]),
        ci("label", &[("name", "loop")]),
        ci("ret", &[]),
    ])
    .unwrap_err();
    assert!(err.contains("duplicate label"), "{err}");
}

#[test]
fn branch_to_unknown_label_errors() {
    let err = try_encode(vec![ci("b", &[("target", "missing")]), ci("ret", &[])]).unwrap_err();
    assert!(err.contains("does not resolve"), "{err}");
}

#[test]
fn missing_entry_symbol_errors() {
    let mut plan = plan_of(vec![ci("ret", &[])]);
    plan.entry_symbol = None;
    assert!(encode(&plan).map(|_| ()).unwrap_err().contains("entry symbol"));
}

// --- emitter.rs: the scalar op vocabulary --------------------------------------

#[test]
fn integer_alu_and_logic_ops_encode_as_op() {
    for op in [
        "and", "orr", "eor", "mul", "smulh", "umulh", "sdiv", "udiv", "rv.slt", "rv.sltu",
        "lslv", "lsrv", "asrv",
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
            &[("dst", "a0"), ("lhs", "a1"), ("rhs", "a2"), ("minuend", "a3")],
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
    for op in ["str_u32", "str_u8"] {
        let w = words(&encode_text(vec![
            ci(op, &[("src", "a0"), ("base", "sp"), ("offset", "8")]),
            ci("ret", &[]),
        ]));
        assert_eq!(w[0] & 0x7f, 0x23, "{op} is a STORE");
    }
    // Large store offset -> li + add + store.
    let w = words(&encode_text(vec![
        ci("str_u64", &[("src", "a0"), ("base", "sp"), ("offset", "5000")]),
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
        ci("ldr_d", &[("dst", "fa0"), ("base", "sp"), ("offset", "5000")]),
        ci("str_d", &[("src", "fa0"), ("base", "sp"), ("offset", "5000")]),
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
                ("dst", "a0"), ("carry_out", "a1"), ("lhs", "a2"), ("rhs", "a3"),
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
                ("dst", "a0"), ("borrow_out", "a1"), ("lhs", "a2"), ("rhs", "a3"),
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
    for i in 0..3 {
        assert_eq!(w[i] & 0x7f, 0x53, "fp move/convert {i} is OP-FP");
    }
    for op in [
        "fcvtzs_x_from_d", "fcvtms_x_from_d", "fcvtps_x_from_d", "fcvtas_x_from_d",
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
    assert!(w.len() >= 4, "wide immediate needs a multi-step li: {}", w.len());
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
        &[("lhs", "a0"), ("rhs", "a1"), ("cond", "zz"), ("target", "x")],
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
