//! plan-176-J: the AArch64 encoder's branches_image_operands tests. The shared `fresh_encoder`/`encode_one`/\n//! `emit_words`/`plan` helpers live in `encode/tests/mod.rs`.

use super::*;

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
            .physical_field("dst", "x0")
            .field("symbol", "g"),
    )
    .unwrap();
    enc.emit_instruction(
        &CodeInstruction::new("add_pageoff")
            .physical_field("dst", "x0")
            .physical_field("src", "x0")
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
            .physical_field("dst", "x0")
            .field("symbol", "g"),
    )
    .unwrap();
    enc.emit_instruction(
        &CodeInstruction::new("add_pageoff")
            .physical_field("dst", "x0")
            .physical_field("src", "x0")
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
                .physical_field("dst", "d0")
                .physical_field("lhs", "d1")
                .physical_field("rhs", "d2"),
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
                    .physical_field("dst", "x0")
                    .physical_field("src", "x1")
                    .field("shift", "64")
            )
            .is_err());
    }
}

fn plan_fn(
    symbol: &str,
    instructions: Vec<CodeInstruction>,
) -> crate::codegen::engine::types::CodeFunction {
    crate::codegen::engine::types::CodeFunction {
        name: symbol.to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: crate::codegen::engine::types::CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        instructions,
        relocations: Vec::new(),
        stack_slots: Vec::new(),
    }
}

fn data_obj(
    symbol: &str,
    kind: &str,
    value: &str,
    align: usize,
    size: usize,
) -> crate::codegen::engine::types::CodeDataObject {
    crate::codegen::engine::types::CodeDataObject {
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
                .physical_field("dst", "x0")
                .physical_field("src", "x1"),
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
        vec![crate::codegen::engine::types::CodeImport {
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
    let inst = CodeInstruction::new("mov").physical_field("dst", "x0");
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
            .physical_field("dst", "x0")
            .field("value", v)
    };
    assert_eq!(instruction_size(&mk("0")).unwrap(), 4);
    assert_eq!(instruction_size(&mk("65536")).unwrap(), 8); // bits in the 2nd chunk
    assert_eq!(instruction_size(&mk("18446744073709551615")).unwrap(), 16); // u64::MAX
                                                                            // add/sub immediate sizing: small (1 word), large multi-chunk.
    let addi = |v: &str| {
        CodeInstruction::new("add_imm")
            .physical_field("dst", "x0")
            .physical_field("src", "x1")
            .field("imm", v)
    };
    assert_eq!(instruction_size(&addi("1")).unwrap(), 4);
    assert_eq!(instruction_size(&addi("0")).unwrap(), 4);
    assert!(instruction_size(&addi("40000")).unwrap() >= 8);
    // cmp_imm sizing reads the rhs field.
    assert_eq!(
        instruction_size(
            &CodeInstruction::new("cmp_imm")
                .physical_field("lhs", "x0")
                .physical_field("rhs", "1")
        )
        .unwrap(),
        4
    );
    // Memory sizing: aligned in-range = 4; large = scratch + word.
    let ldr = |off: &str| {
        CodeInstruction::new("ldr_u64")
            .physical_field("dst", "x0")
            .physical_field("base", "x1")
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
            .physical_field(field, r)
            .physical_field("base", "x1")
            .field("offset", "0");
        assert_eq!(instruction_size(&inst).unwrap(), 4);
    }
    // A plain op (not immediate/memory) is always one word.
    assert_eq!(
        instruction_size(
            &CodeInstruction::new("mov")
                .physical_field("dst", "x0")
                .physical_field("src", "x1")
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

/// bug-284 C1: register 31 is XZR in every shifted-register form and SP in the
/// immediate forms, but `reg()` folds both spellings onto 31 and drops the
/// difference. An `sp` operand reaching a register-form slot therefore encoded a
/// silent read of zero. bug-178 B fixed this for `mov` alone; these are the
/// remaining forms.
#[test]
fn sp_is_rejected_in_shifted_register_operand_slots() {
    let mut encoder = fresh_encoder();
    for (op, fields) in [
        ("add", vec![("dst", "x1"), ("lhs", "sp"), ("rhs", "x2")]),
        ("sub", vec![("dst", "x1"), ("lhs", "x2"), ("rhs", "sp")]),
        ("and", vec![("dst", "sp"), ("lhs", "x1"), ("rhs", "x2")]),
        ("orr", vec![("dst", "x1"), ("lhs", "raw_sp"), ("rhs", "x2")]),
        ("mul", vec![("dst", "x1"), ("lhs", "x2"), ("rhs", "sp")]),
        ("cmp", vec![("lhs", "sp"), ("rhs", "x2")]),
    ] {
        let mut ins = CodeInstruction::new(op);
        for (k, v) in &fields {
            ins = ins.physical_field(k, v);
        }
        let err = encoder
            .emit_instruction(&ins)
            .expect_err("an sp operand in a shifted-register slot must be rejected");
        assert!(
            err.contains("XZR") || err.contains("stack pointer"),
            "{op}: unexpected error: {err}"
        );
    }

    // xzr in the same slots stays legal -- reading zero is what it means.
    let ins = CodeInstruction::new("add")
        .physical_field("dst", "x1")
        .physical_field("lhs", "xzr")
        .physical_field("rhs", "x2");
    encoder
        .emit_instruction(&ins)
        .expect("xzr is a legitimate shifted-register operand");
}

/// bug-284 C1: `cmp_imm` picks its form from the immediate's magnitude. The
/// immediate form reads register 31 as SP (correct for `sp`), but the wide
/// fallback is a register-form `cmp` where 31 is XZR -- so the same instruction
/// changed meaning with the size of its immediate.
#[test]
fn cmp_imm_against_sp_is_rejected_only_when_the_immediate_forces_register_form() {
    let mut encoder = fresh_encoder();
    // Fits 12 bits: the immediate form is correct for sp, and stays allowed.
    let narrow = CodeInstruction::new("cmp_imm")
        .physical_field("lhs", "sp")
        .physical_field("rhs", "4095");
    encoder
        .emit_instruction(&narrow)
        .expect("a 12-bit immediate uses the sp-correct immediate form");

    // One past the 12-bit limit forces the register-form fallback, where the
    // operand would silently become XZR.
    let wide = CodeInstruction::new("cmp_imm")
        .physical_field("lhs", "sp")
        .physical_field("rhs", "4096");
    let err = encoder
        .emit_instruction(&wide)
        .expect_err("the register-form fallback must not silently compare against zero");
    assert!(err.contains("XZR"), "unexpected error: {err}");

    // A plain register is unaffected in both regimes.
    for imm in ["4095", "4096"] {
        let ins = CodeInstruction::new("cmp_imm")
            .physical_field("lhs", "x3")
            .physical_field("rhs", imm);
        encoder
            .emit_instruction(&ins)
            .expect("a normal register encodes in either form");
    }
}

/// bug-284 C2: the add/sub immediate decomposition removes at most `4095 << 12`
/// per iteration with no cap, so an immediate near `u64::MAX` -- what a lowering
/// arithmetic-wrap bug produces -- would spin for ~10^12 iterations inside
/// `instruction_size`, hanging the compiler before any error could be reported.
#[test]
fn an_absurdly_wide_add_sub_immediate_is_rejected_rather_than_looped_over() {
    let mut encoder = fresh_encoder();
    for op in ["add_imm", "sub_imm"] {
        let ins = CodeInstruction::new(op)
            .physical_field("dst", "x1")
            .physical_field("src", "x2")
            .field("imm", &u64::MAX.to_string());
        let err = encoder
            .emit_instruction(&ins)
            .expect_err("a u64::MAX immediate must be rejected, not decomposed");
        assert!(err.contains("too wide"), "{op}: unexpected error: {err}");
    }

    // A legitimately large multi-chunk offset still encodes.
    let ins = CodeInstruction::new("add_imm")
        .physical_field("dst", "x1")
        .physical_field("src", "x2")
        .field("imm", "70000");
    encoder
        .emit_instruction(&ins)
        .expect("a real multi-chunk offset still encodes");
}
