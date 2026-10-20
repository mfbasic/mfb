//! plan-176-J: the x86-64 encoder's image_and_coverage tests. The shared `fresh_encoder`/`bytes`/\n//! `enc_err`/`enc` helpers live in `encode/tests/mod.rs`.

use super::*;

fn encode_err(plan: &crate::codegen::engine::types::NativeCodePlan) -> String {
    match super::encode(plan) {
        Ok(_) => panic!("expected encode to fail"),
        Err(err) => err,
    }
}

fn minimal_plan(
    functions: Vec<crate::codegen::engine::types::CodeFunction>,
    data_objects: Vec<crate::codegen::engine::types::CodeDataObject>,
    imports: Vec<crate::codegen::engine::types::CodeImport>,
    entry: Option<&str>,
) -> crate::codegen::engine::types::NativeCodePlan {
    crate::codegen::engine::types::NativeCodePlan {
        target: "linux-x86_64".to_string(),
        build_mode: crate::target::NativeBuildMode::Console,
        arch: "x86_64".to_string(),
        project: "t".to_string(),
        entry_symbol: entry.map(str::to_string),
        imports,
        data_objects,
        functions,
    }
}

fn simple_function(
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

#[test]
fn encode_produces_an_image_with_symbols_and_data() {
    let data = crate::codegen::engine::types::CodeDataObject {
        symbol: "g".to_string(),
        kind: "string".to_string(),
        layout: "bytes".to_string(),
        align: 8,
        size: 16,
        value: "hi".to_string(),
    };
    let raw = crate::codegen::engine::types::CodeDataObject {
        symbol: "r".to_string(),
        kind: "raw".to_string(),
        layout: "bytes".to_string(),
        align: 1,
        size: 2,
        value: "de ad".to_string(),
    };
    let func = simple_function(
        "main",
        vec![
            crate::arch::aarch64::abi::label("entry"),
            CodeInstruction::new("mov")
                .physical_field("dst", "rax")
                .physical_field("src", "rbx"),
            CodeInstruction::new("b").field("target", "entry"),
            CodeInstruction::new("ret"),
        ],
    );
    let plan = minimal_plan(vec![func], vec![data, raw], Vec::new(), Some("main"));
    let image = super::encode(&plan).expect("encode");
    assert_eq!(image.entry, "main");
    // The two data symbols and the one function symbol are present.
    assert!(image.symbols.iter().any(|s| s.name == "g"));
    assert!(image.symbols.iter().any(|s| s.name == "main"));
    // The raw object decoded to two bytes (0xDE, 0xAD) somewhere in data.
    assert!(image.data.windows(2).any(|w| w == [0xDE, 0xAD]));
    // The string object wrote its length prefix (2) and bytes.
    assert!(!image.data.is_empty());
}

#[test]
fn encode_requires_entry_symbol() {
    let plan = minimal_plan(
        vec![simple_function("f", vec![CodeInstruction::new("ret")])],
        Vec::new(),
        Vec::new(),
        None,
    );
    let err = encode_err(&plan);
    assert!(err.contains("entry symbol"), "got: {err}");
}

#[test]
fn encode_data_rejects_bad_hex() {
    // Odd digit count.
    let plan = minimal_plan(
        Vec::new(),
        vec![crate::codegen::engine::types::CodeDataObject {
            symbol: "r".to_string(),
            kind: "raw".to_string(),
            layout: "bytes".to_string(),
            align: 1,
            size: 1,
            value: "abc".to_string(),
        }],
        Vec::new(),
        Some("main"),
    );
    assert!(encode_err(&plan).contains("even digit"));
    // Non-hex digit.
    let plan = minimal_plan(
        Vec::new(),
        vec![crate::codegen::engine::types::CodeDataObject {
            symbol: "r".to_string(),
            kind: "raw".to_string(),
            layout: "bytes".to_string(),
            align: 1,
            size: 1,
            value: "zz".to_string(),
        }],
        Vec::new(),
        Some("main"),
    );
    assert!(encode_err(&plan).contains("non-hex"));
}

#[test]
fn encode_carries_imports() {
    let func = simple_function(
        "main",
        vec![
            CodeInstruction::new("bl").field("target", "puts"),
            CodeInstruction::new("ret"),
        ],
    );
    let plan = minimal_plan(
        vec![func],
        Vec::new(),
        vec![crate::codegen::engine::types::CodeImport {
            library: "libc".to_string(),
            symbol: "puts".to_string(),
        }],
        Some("main"),
    );
    let image = super::encode(&plan).expect("encode");
    assert!(image
        .imports
        .iter()
        .any(|i| i.symbol == "puts" && i.library == "libc"));
}

#[test]
fn encoder_emits_and_patches_a_local_branch() {
    // Drive emit_instruction + patch_labels: a forward `b` to a label two words on.
    let mut enc = fresh_encoder();
    enc.labels.insert("L".to_string(), 5); // 5 bytes ahead (the jmp itself)
    enc.emit_instruction(&CodeInstruction::new("b").field("target", "L"))
        .unwrap();
    enc.patch_labels().unwrap();
    // jmp rel32; the displacement resolves to 0 (target is the next instruction).
    assert_eq!(enc.text[0], 0xE9);
    assert_eq!(&enc.text[1..5], &[0, 0, 0, 0]);
}

#[test]
fn encoder_unresolved_label_errors() {
    let mut enc = fresh_encoder();
    enc.emit_instruction(&CodeInstruction::new("b.eq").field("target", "missing"))
        .unwrap();
    let err = enc.patch_labels().unwrap_err();
    assert!(err.contains("does not resolve"), "got: {err}");
}

#[test]
fn encoder_call_relocation_bindings() {
    use super::{EncodedSection, EncodedSymbol};
    // Internal call: the target is a known text symbol → an "internal" reloc.
    let mut enc = fresh_encoder();
    enc.symbols.push(EncodedSymbol {
        name: "_mfb_internal".to_string(),
        section: EncodedSection::Text,
        offset: 0,
    });
    enc.emit_instruction(&CodeInstruction::new("bl").field("target", "_mfb_internal"))
        .unwrap();
    assert_eq!(enc.relocations.len(), 1);
    assert_eq!(enc.relocations[0].binding, "internal");

    // External call: the target is an imported symbol → an "external" reloc.
    let mut enc = fresh_encoder();
    enc.imports
        .insert("snprintf".to_string(), "libc".to_string());
    enc.emit_instruction(&CodeInstruction::new("bl").field("target", "snprintf"))
        .unwrap();
    assert_eq!(enc.relocations[0].binding, "external");
    assert_eq!(enc.relocations[0].library.as_deref(), Some("libc"));

    // Unresolved call target errors.
    let mut enc = fresh_encoder();
    let err = enc
        .emit_instruction(&CodeInstruction::new("bl").field("target", "nowhere"))
        .unwrap_err();
    assert!(err.contains("does not resolve"), "got: {err}");
}

#[test]
fn encoder_data_and_got_relocations() {
    // adrp to an internal data symbol → a "data" binding relocation.
    let mut enc = fresh_encoder();
    enc.emit_instruction(
        &CodeInstruction::new("adrp")
            .physical_field("dst", "rax")
            .field("symbol", "g"),
    )
    .unwrap();
    assert_eq!(enc.relocations[0].binding, "data");

    // adrp to an imported symbol re-routes through the GOT (external binding).
    let mut enc = fresh_encoder();
    enc.imports.insert("g".to_string(), "libc".to_string());
    enc.emit_instruction(
        &CodeInstruction::new("adrp")
            .physical_field("dst", "rax")
            .field("symbol", "g"),
    )
    .unwrap();
    assert_eq!(enc.relocations[0].binding, "external");
    assert_eq!(enc.relocations[0].library.as_deref(), Some("libc"));
}

#[test]
fn operand_decoding_edge_cases() {
    use super::operand::{fp_reg, immediate, is_zero_token, reg, shift};
    // The neutral zero token maps to 16 and is recognized (plan-34-A retired the
    // dead `rzero`/`zero` aliases).
    assert!(is_zero_token(reg("xzr".to_string()).unwrap()));
    assert!(reg("rzero".to_string()).is_err());
    assert!(reg("zero".to_string()).is_err());
    // raw_sp / sp alias rsp (4).
    assert_eq!(reg("raw_sp".to_string()).unwrap(), 4);
    assert_eq!(reg("sp".to_string()).unwrap(), 4);
    // Extended registers.
    assert_eq!(reg("r8".to_string()).unwrap(), 8);
    assert_eq!(reg("r15".to_string()).unwrap(), 15);
    // Unknown register errors.
    assert!(reg("bogus".to_string()).is_err());
    // fp_reg parses xmmN and rejects out-of-range / non-xmm names.
    assert_eq!(fp_reg("xmm7".to_string()).unwrap(), 7);
    assert!(fp_reg("xmm16".to_string()).is_err());
    assert!(fp_reg("rax".to_string()).is_err());
    // immediate accepts booleans and rejects garbage.
    assert_eq!(immediate("true".to_string()).unwrap(), 1);
    assert_eq!(immediate("false".to_string()).unwrap(), 0);
    assert!(immediate("nope".to_string()).is_err());
    // shift range guard.
    assert_eq!(shift("5".to_string()).unwrap(), 5);
    assert!(shift("64".to_string()).is_err());
    assert!(shift("x".to_string()).is_err());
    // A missing field reports the op name.
    let ins = CodeInstruction::new("mov").physical_field("dst", "rax");
    assert!(enc_err(&ins).contains("missing field"));
}

#[test]
fn unsupported_op_errors() {
    // A valid CodeOp the x86 backend does not encode (an AArch64-only SIMD
    // lane shift); `CodeInstruction::new` only accepts real mnemonics, so this
    // exercises the emitter's unsupported-op fallthrough rather than an unknown
    // mnemonic.
    let ins = CodeInstruction::new("sshl_v")
        .physical_field("dst", "v0")
        .physical_field("lhs", "v1")
        .physical_field("rhs", "v2");
    let err = match encode_instruction(&ins) {
        Ok(_) => panic!("expected sshl_v to be unsupported"),
        Err(err) => err,
    };
    assert!(err.contains("unsupported op"), "got: {err}");
}

#[test]
fn label_and_add_pageoff_are_empty() {
    assert_eq!(enc("label", &[("name", "L")]).len(), 0);
    assert_eq!(
        enc(
            "add_pageoff",
            &[("dst", "rax"), ("src", "rax"), ("symbol", "g")]
        )
        .len(),
        0
    );
}

#[test]
fn rev_w_rev_x() {
    // rev_x rbx, rbx : dst==src, wide → bswap rbx = 48 0F CB
    assert_eq!(
        bytes("rev_x", &[("dst", "rbx"), ("src", "rbx")]),
        [0x48, 0x0F, 0xCB]
    );
    // rev_x rbx, rsi : dst!=src, wide → mov rbx,rsi ; bswap rbx
    assert_eq!(
        bytes("rev_x", &[("dst", "rbx"), ("src", "rsi")]),
        [0x48, 0x89, 0xF3, 0x48, 0x0F, 0xCB]
    );
    // rev_w rbx, rbx : dst==src, 32-bit bswap = 0F CB (no REX for low regs)
    assert_eq!(
        bytes("rev_w", &[("dst", "rbx"), ("src", "rbx")]),
        [0x0F, 0xCB]
    );
    // rev_w rbx, rsi : 32-bit mov (89 /r) ; bswap
    assert_eq!(
        bytes("rev_w", &[("dst", "rbx"), ("src", "rsi")]),
        [0x89, 0xF3, 0x0F, 0xCB]
    );
    // rev_w with extended reg exercises the 32-bit REX + high bswap arms:
    // mov r8d,r9d ; bswap r8d.
    assert_eq!(
        enc("rev_w", &[("dst", "r8"), ("src", "r9")]),
        [0x45, 0x89, 0xC8, 0x41, 0x0F, 0xC8]
    );
}

#[test]
fn rbit_reverse_bits() {
    // Long expansion; assert it encodes and ends in a 64-bit bswap of the dst.
    let b = enc("rbit", &[("dst", "rbx"), ("src", "rsi")]);
    assert!(b.len() > 20);
    // dst==src variant skips the initial mov.
    let same = enc("rbit", &[("dst", "rbx"), ("src", "rbx")]);
    assert!(same.len() < b.len());
    // extended-register form exercises the REX.B paths inside the closures.
    // push rax/rdx ; three (mask & shift) rounds over 0x55.../0x33.../0x0f...
    // constants ; bswap r8 ; pop rdx/rax.
    assert_eq!(
        enc("rbit", &[("dst", "r8"), ("src", "r8")]),
        [
            0x50, 0x52, 0x4C, 0x89, 0xC2, 0x48, 0xB8, 0x55, 0x55, 0x55, 0x55, 0x55, 0x55, 0x55,
            0x55, 0x48, 0x21, 0xC2, 0x48, 0xC1, 0xE2, 0x01, 0x49, 0xC1, 0xE8, 0x01, 0x49, 0x21,
            0xC0, 0x49, 0x09, 0xD0, 0x4C, 0x89, 0xC2, 0x48, 0xB8, 0x33, 0x33, 0x33, 0x33, 0x33,
            0x33, 0x33, 0x33, 0x48, 0x21, 0xC2, 0x48, 0xC1, 0xE2, 0x02, 0x49, 0xC1, 0xE8, 0x02,
            0x49, 0x21, 0xC0, 0x49, 0x09, 0xD0, 0x4C, 0x89, 0xC2, 0x48, 0xB8, 0x0F, 0x0F, 0x0F,
            0x0F, 0x0F, 0x0F, 0x0F, 0x0F, 0x48, 0x21, 0xC2, 0x48, 0xC1, 0xE2, 0x04, 0x49, 0xC1,
            0xE8, 0x04, 0x49, 0x21, 0xC0, 0x49, 0x09, 0xD0, 0x49, 0x0F, 0xC8, 0x5A, 0x58
        ]
    );
}

#[test]
fn msub_disjoint_and_dst_aliases_lhs() {
    // dst aliases lhs (not rax minuend) keeps product-first order:
    // mov rax,rbx ; imul rax,rdi ; mov rbx,rcx ; sub rbx,rax.
    assert_eq!(
        enc(
            "msub",
            &[
                ("dst", "rbx"),
                ("lhs", "rbx"),
                ("rhs", "rdi"),
                ("minuend", "rcx")
            ]
        ),
        [0x48, 0x89, 0xD8, 0x48, 0x0F, 0xAF, 0xC7, 0x48, 0x89, 0xCB, 0x48, 0x29, 0xC3]
    );
}

#[test]
fn div_aliasing_and_dividend_preservation() {
    // Divisor mapped onto rax → stage in a stack slot (memory divide):
    // sub rsp,8 ; mov [rsp],rax ; mov rax,rsi ; xor rdx,rdx ; div [rsp] ;
    // mov rbx,rax ; add rsp,8.
    assert_eq!(
        enc("udiv", &[("dst", "rbx"), ("lhs", "rsi"), ("rhs", "rax")]),
        [
            0x48, 0x81, 0xEC, 0x08, 0x00, 0x00, 0x00, 0x48, 0x89, 0x04, 0x24, 0x48, 0x89, 0xF0,
            0x48, 0x31, 0xD2, 0x48, 0xF7, 0x34, 0x24, 0x48, 0x89, 0xC3, 0x48, 0x81, 0xC4, 0x08,
            0x00, 0x00, 0x00
        ]
    );
    // Divisor mapped onto rdx → same memory path, signed (cqo ; idiv [rsp]).
    assert_eq!(
        enc("sdiv", &[("dst", "rbx"), ("lhs", "rsi"), ("rhs", "rdx")]),
        [
            0x48, 0x81, 0xEC, 0x08, 0x00, 0x00, 0x00, 0x48, 0x89, 0x14, 0x24, 0x48, 0x89, 0xF0,
            0x48, 0x99, 0x48, 0xF7, 0x3C, 0x24, 0x48, 0x89, 0xC3, 0x48, 0x81, 0xC4, 0x08, 0x00,
            0x00, 0x00
        ]
    );
    // Dividend IS rax and quotient wanted elsewhere → preserve rax across div
    // (single stack slot, direct `div rdi`).
    assert_eq!(
        enc("udiv", &[("dst", "rbx"), ("lhs", "rax"), ("rhs", "rdi")]),
        [
            0x48, 0x81, 0xEC, 0x08, 0x00, 0x00, 0x00, 0x48, 0x89, 0x04, 0x24, 0x48, 0x89, 0xC0,
            0x48, 0x31, 0xD2, 0x48, 0xF7, 0xF7, 0x48, 0x89, 0xC3, 0x48, 0x8B, 0x04, 0x24, 0x48,
            0x81, 0xC4, 0x08, 0x00, 0x00, 0x00
        ]
    );
    // Both preserve-dividend AND rhs-alias paths at once → two stack slots.
    assert_eq!(
        enc("udiv", &[("dst", "rbx"), ("lhs", "rax"), ("rhs", "rdx")]),
        [
            0x48, 0x81, 0xEC, 0x08, 0x00, 0x00, 0x00, 0x48, 0x89, 0x04, 0x24, 0x48, 0x81, 0xEC,
            0x08, 0x00, 0x00, 0x00, 0x48, 0x89, 0x14, 0x24, 0x48, 0x89, 0xC0, 0x48, 0x31, 0xD2,
            0x48, 0xF7, 0x34, 0x24, 0x48, 0x89, 0xC3, 0x48, 0x81, 0xC4, 0x08, 0x00, 0x00, 0x00,
            0x48, 0x8B, 0x04, 0x24, 0x48, 0x81, 0xC4, 0x08, 0x00, 0x00, 0x00
        ]
    );
}

#[test]
fn shifts_var_32bit() {
    // rorv_w rbx, rbx, rsi : push rcx ; mov ecx,esi ; ror ebx,cl ; pop rcx (bug-125).
    assert_eq!(
        bytes("rorv_w", &[("dst", "rbx"), ("lhs", "rbx"), ("rhs", "rsi")]),
        [0x51, 0x89, 0xF1, 0xD3, 0xCB, 0x59]
    );
    // dst != value copies the value too; extended reg sets REX:
    // push rcx ; mov ecx,esi ; mov r8d,r9d ; ror r8d,cl ; pop rcx.
    assert_eq!(
        enc("rorv_w", &[("dst", "r8"), ("lhs", "r9"), ("rhs", "rsi")]),
        [0x51, 0x89, 0xF1, 0x45, 0x89, 0xC8, 0x41, 0xD3, 0xC8, 0x59]
    );
    // lslv with dst != value (mov value in first):
    // push rcx ; mov rcx,rdi ; mov rbx,rsi ; shl rbx,cl ; pop rcx.
    assert_eq!(
        enc("lslv", &[("dst", "rbx"), ("lhs", "rsi"), ("rhs", "rdi")]),
        [0x51, 0x48, 0x89, 0xF9, 0x48, 0x89, 0xF3, 0x48, 0xD3, 0xE3, 0x59]
    );
    // lsrv arm: push rcx ; mov rcx,rsi ; shr rbx,cl ; pop rcx.
    assert_eq!(
        enc("lsrv", &[("dst", "rbx"), ("lhs", "rbx"), ("rhs", "rsi")]),
        [0x51, 0x48, 0x89, 0xF1, 0x48, 0xD3, 0xEB, 0x59]
    );
}

#[test]
fn shift_imm_move_first() {
    // lsl_imm rbx, rsi, 2 : dst != src → mov rbx,rsi ; shl rbx,2
    assert_eq!(
        enc("lsl_imm", &[("dst", "rbx"), ("src", "rsi"), ("shift", "2")]),
        [0x48, 0x89, 0xF3, 0x48, 0xC1, 0xE3, 0x02]
    );
}

#[test]
fn add_imm_move_first_and_str_u32_extended() {
    // add_imm rbx, rsi, 8 : dst != src → mov rbx,rsi ; add rbx,8
    assert_eq!(
        enc("add_imm", &[("dst", "rbx"), ("src", "rsi"), ("imm", "8")]),
        [0x48, 0x89, 0xF3, 0x48, 0x81, 0xC3, 0x08, 0x00, 0x00, 0x00]
    );
    // sub_imm dst != src : mov rbx,rsi ; sub rbx,8.
    assert_eq!(
        enc("sub_imm", &[("dst", "rbx"), ("src", "rsi"), ("imm", "8")]),
        [0x48, 0x89, 0xF3, 0x48, 0x81, 0xEB, 0x08, 0x00, 0x00, 0x00]
    );
    // str_u32 with extended base/src forces REX.R.B : mov [r9], r8d.
    assert_eq!(
        enc("str_u32", &[("src", "r8"), ("base", "r9"), ("offset", "0")]),
        [0x45, 0x89, 0x81, 0x00, 0x00, 0x00, 0x00]
    );
    // ldr_u32 extended too : mov r8d, [r9].
    assert_eq!(
        enc("ldr_u32", &[("dst", "r8"), ("base", "r9"), ("offset", "0")]),
        [0x45, 0x8B, 0x81, 0x00, 0x00, 0x00, 0x00]
    );
}

#[test]
fn str_u8_extended_and_u16_encode() {
    // str_u8 with an r8b destination sets REX.B : mov [rbx], r8b.
    assert_eq!(
        enc("str_u8", &[("src", "r8"), ("base", "rbx"), ("offset", "0")]),
        [0x44, 0x88, 0x83, 0x00, 0x00, 0x00, 0x00]
    );
    // bug-294: `str_u16` reaches the MemWidth::U16 store arm through ordinary
    // dispatch and now encodes rather than erroring; see
    // `str_u16_encodes_the_operand_size_prefixed_store` for the byte-exact forms.
    // mov [r9+2], r8w (66 operand-size + REX.R.B).
    assert_eq!(
        enc("str_u16", &[("src", "r8"), ("base", "r9"), ("offset", "2")]),
        [0x66, 0x45, 0x89, 0x81, 0x02, 0x00, 0x00, 0x00]
    );
    // movzx r8, word [r9+2].
    assert_eq!(
        enc("ldr_u16", &[("dst", "r8"), ("base", "r9"), ("offset", "2")]),
        [0x4D, 0x0F, 0xB7, 0x81, 0x02, 0x00, 0x00, 0x00]
    );
}

#[test]
fn extra_branch_conditions() {
    // Overflow / sign / unsigned-LE and float-only jcc mnemonics.
    for (op, cc) in [
        ("b.vs", 0x80u8),
        ("b.vc", 0x81),
        ("b.mi", 0x88),
        ("b.ls", 0x86),
    ] {
        let b = bytes(op, &[("target", "L")]);
        assert_eq!(b[0], 0x0F);
        assert_eq!(b[1], cc);
    }
    for (op, cc) in [
        ("x86.jae", 0x83u8),
        ("x86.jp", 0x8A),
        ("x86.jnp", 0x8B),
        ("x86.ja", 0x87),
        ("x86.jb", 0x82),
        ("x86.jbe", 0x86),
        ("x86.je", 0x84),
        ("x86.jne", 0x85),
    ] {
        let b = bytes(op, &[("target", "L")]);
        assert_eq!([b[0], b[1]], [0x0F, cc]);
    }
}

#[test]
fn scalar_double_moves_and_arith() {
    // fmov_d_from_x xmm0, rbx : movq xmm0, rbx = 66 48 0F 6E C3 (neutral: fmov_i2f)
    assert_eq!(
        bytes("fmov_d_from_x", &[("dst", "xmm0"), ("src", "rbx")]),
        [0x66, 0x48, 0x0F, 0x6E, 0xC3]
    );
    // fmov_x_from_d rbx, xmm0 : movq rbx, xmm0 = 66 48 0F 7E C3 (neutral: fmov_f2i)
    assert_eq!(
        bytes("fmov_x_from_d", &[("dst", "rbx"), ("src", "xmm0")]),
        [0x66, 0x48, 0x0F, 0x7E, 0xC3]
    );
    // fmov_d_from_d xmm1, xmm0 : movaps = 0F 28 C8
    assert_eq!(
        bytes("fmov_d_from_d", &[("dst", "xmm1"), ("src", "xmm0")]),
        [0x0F, 0x28, 0xC8]
    );
    // addsd dst==lhs in place: fadd_d xmm0, xmm0, xmm1 = F2 0F 58 C1
    assert_eq!(
        bytes(
            "fadd_d",
            &[("dst", "xmm0"), ("lhs", "xmm0"), ("rhs", "xmm1")]
        ),
        [0xF2, 0x0F, 0x58, 0xC1]
    );
    // fmul_d commutative dst==rhs → swap operands: mulsd xmm0,xmm1.
    assert_eq!(
        enc(
            "fmul_d",
            &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm0")]
        ),
        [0xF2, 0x0F, 0x59, 0xC1]
    );
    // fsub_d dst==rhs non-commutative → staged through xmm15:
    // movsd xmm15,xmm1 ; subsd xmm15,xmm0 ; movsd xmm0,xmm15.
    assert_eq!(
        enc(
            "fsub_d",
            &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm0")]
        ),
        [
            0xF2, 0x44, 0x0F, 0x10, 0xF9, 0xF2, 0x44, 0x0F, 0x5C, 0xF8, 0xF2, 0x41, 0x0F, 0x10,
            0xC7
        ]
    );
    // fdiv_d disjoint → copy lhs then op: movsd xmm2,xmm1 ; divsd xmm2,xmm0.
    assert_eq!(
        enc(
            "fdiv_d",
            &[("dst", "xmm2"), ("lhs", "xmm1"), ("rhs", "xmm0")]
        ),
        [0xF2, 0x0F, 0x10, 0xD1, 0xF2, 0x0F, 0x5E, 0xD0]
    );
    // fsqrt_d xmm1, xmm0 : F2 0F 51 C8
    assert_eq!(
        bytes("fsqrt_d", &[("dst", "xmm1"), ("src", "xmm0")]),
        [0xF2, 0x0F, 0x51, 0xC8]
    );
}

#[test]
fn scalar_double_min_max() {
    // minsd/maxsd via the non-commutative sse_arith path (plan-02 §4).
    // fminnm_d xmm0, xmm0, xmm1 (dst==lhs in place) : minsd = F2 0F 5D C1.
    assert_eq!(
        bytes(
            "fminnm_d",
            &[("dst", "xmm0"), ("lhs", "xmm0"), ("rhs", "xmm1")]
        ),
        [0xF2, 0x0F, 0x5D, 0xC1]
    );
    // fmaxnm_d xmm0, xmm0, xmm1 : maxsd = F2 0F 5F C1.
    assert_eq!(
        bytes(
            "fmaxnm_d",
            &[("dst", "xmm0"), ("lhs", "xmm0"), ("rhs", "xmm1")]
        ),
        [0xF2, 0x0F, 0x5F, 0xC1]
    );
    // Disjoint destination copies lhs first, then the op:
    // movsd xmm2,xmm1 ; minsd xmm2,xmm0.
    assert_eq!(
        enc(
            "fminnm_d",
            &[("dst", "xmm2"), ("lhs", "xmm1"), ("rhs", "xmm0")]
        ),
        [0xF2, 0x0F, 0x10, 0xD1, 0xF2, 0x0F, 0x5D, 0xD0]
    );
}

#[test]
fn scalar_double_fma_family() {
    // Scalar FMA family (FMA3), staged in xmm15 (plan-02 §5). For
    // dst=xmm0, addend=xmm1, lhs=xmm2, rhs=xmm3 each expands to:
    //   movsd  xmm15, xmm1        F2 44 0F 10 F9
    //   v...231sd xmm15, xmm2, xmm3   C4 62 E9 <op> FB   (op = B9/BB/BD/BF)
    //   movsd  xmm0, xmm15        F2 41 0F 10 C7
    let fields = &[
        ("dst", "xmm0"),
        ("addend", "xmm1"),
        ("lhs", "xmm2"),
        ("rhs", "xmm3"),
    ];
    let head = [0xF2u8, 0x44, 0x0F, 0x10, 0xF9];
    let tail = [0xF2u8, 0x41, 0x0F, 0x10, 0xC7];
    let mid = |op: u8| [0xC4u8, 0x62, 0xE9, op, 0xFB];
    let expect = |op: u8| -> Vec<u8> {
        head.iter()
            .chain(mid(op).iter())
            .chain(tail.iter())
            .copied()
            .collect()
    };
    assert_eq!(bytes("fmadd_d", fields), expect(0xB9)); // vfmadd231sd
    assert_eq!(bytes("fmsub_d", fields), expect(0xBB)); // vfmsub231sd
    assert_eq!(bytes("fnmsub_d", fields), expect(0xBD)); // vfnmadd231sd
    assert_eq!(bytes("fnmadd_d", fields), expect(0xBF)); // vfnmsub231sd
}

#[test]
fn scalar_double_compares_and_signops() {
    // fcmp_d xmm0, xmm1 : ucomisd = 66 0F 2E C1
    assert_eq!(
        bytes("fcmp_d", &[("lhs", "xmm0"), ("rhs", "xmm1")]),
        [0x66, 0x0F, 0x2E, 0xC1]
    );
    // fcmp_zero_d src : xorps xmm15,xmm15 ; ucomisd xmm0,xmm15.
    assert_eq!(
        enc("fcmp_zero_d", &[("src", "xmm0")]),
        [0x45, 0x0F, 0x57, 0xFF, 0x66, 0x41, 0x0F, 0x2E, 0xC7]
    );
    // fneg_d dst==src (no move): pcmpeqd xmm15 ; psllq xmm15,63 ; xorpd xmm0,xmm15.
    assert_eq!(
        enc("fneg_d", &[("dst", "xmm0"), ("src", "xmm0")]),
        [
            0x66, 0x45, 0x0F, 0x76, 0xFF, 0x66, 0x41, 0x0F, 0x73, 0xF7, 0x3F, 0x66, 0x41, 0x0F,
            0x57, 0xC7
        ]
    );
    // fneg_d dst!=src (movsd xmm1,xmm0 first, then the sign flip on xmm1).
    assert_eq!(
        enc("fneg_d", &[("dst", "xmm1"), ("src", "xmm0")]),
        [
            0xF2, 0x0F, 0x10, 0xC8, 0x66, 0x45, 0x0F, 0x76, 0xFF, 0x66, 0x41, 0x0F, 0x73, 0xF7,
            0x3F, 0x66, 0x41, 0x0F, 0x57, 0xCF
        ]
    );
    // fabs_d dst==src: psllq xmm0,1 ; psrlq xmm0,1.
    assert_eq!(
        enc("fabs_d", &[("dst", "xmm0"), ("src", "xmm0")]),
        [0x66, 0x0F, 0x73, 0xF0, 0x01, 0x66, 0x0F, 0x73, 0xD0, 0x01]
    );
    // fabs_d dst!=src: movsd xmm1,xmm0 ; psllq xmm1,1 ; psrlq xmm1,1.
    assert_eq!(
        enc("fabs_d", &[("dst", "xmm1"), ("src", "xmm0")]),
        [0xF2, 0x0F, 0x10, 0xC8, 0x66, 0x0F, 0x73, 0xF1, 0x01, 0x66, 0x0F, 0x73, 0xD1, 0x01]
    );
}

#[test]
fn int_float_conversions() {
    // scvtf_d_from_x xmm0, rbx : cvtsi2sd = F2 48 0F 2A C3 (neutral: i2f)
    assert_eq!(
        bytes("scvtf_d_from_x", &[("dst", "xmm0"), ("src", "rbx")]),
        [0xF2, 0x48, 0x0F, 0x2A, 0xC3]
    );
    // fcvtzs_x_from_d rbx, xmm0 : cvttsd2si = F2 48 0F 2C D8 (neutral: f2i_trunc)
    assert_eq!(
        bytes("fcvtzs_x_from_d", &[("dst", "rbx"), ("src", "xmm0")]),
        [0xF2, 0x48, 0x0F, 0x2C, 0xD8]
    );
    // floor : roundsd xmm15,xmm0,1 ; cvttsd2si rbx,xmm15.
    assert_eq!(
        enc("fcvtms_x_from_d", &[("dst", "rbx"), ("src", "xmm0")]),
        [0x66, 0x44, 0x0F, 0x3A, 0x0B, 0xF8, 0x01, 0xF2, 0x49, 0x0F, 0x2C, 0xDF]
    );
    // ceil : roundsd xmm15,xmm0,2 ; cvttsd2si rbx,xmm15.
    assert_eq!(
        enc("fcvtps_x_from_d", &[("dst", "rbx"), ("src", "xmm0")]),
        [0x66, 0x44, 0x0F, 0x3A, 0x0B, 0xF8, 0x02, 0xF2, 0x49, 0x0F, 0x2C, 0xDF]
    );
    // nearest ties-away (commandeers rax since dst=rbx): push rax ; ... ; pop rax.
    assert_eq!(
        enc("fcvtas_x_from_d", &[("dst", "rbx"), ("src", "xmm0")]),
        [
            0x50, 0xF2, 0x48, 0x0F, 0x2C, 0xD8, 0xF2, 0x4C, 0x0F, 0x2A, 0xFB, 0xF2, 0x44, 0x0F,
            0x5C, 0xF8, 0xF2, 0x45, 0x0F, 0x58, 0xFF, 0x66, 0x45, 0x0F, 0x3A, 0x0B, 0xFF, 0x03,
            0xF2, 0x49, 0x0F, 0x2C, 0xC7, 0x48, 0x29, 0xC3, 0x58
        ]
    );
}

#[test]
fn scalar_double_mem() {
    // ldr_d xmm0, [rbx+8] : F2 0F 10 43 08 (mod=10 base=rbx no SIB, disp32)
    assert_eq!(
        bytes(
            "ldr_d",
            &[("dst", "xmm0"), ("base", "rbx"), ("offset", "8")]
        ),
        [0xF2, 0x0F, 0x10, 0x83, 0x08, 0, 0, 0]
    );
    // str_d xmm0, [rsp+16] : F2 0F 11 with SIB for rsp base.
    assert_eq!(
        bytes(
            "str_d",
            &[("src", "xmm0"), ("base", "rsp"), ("offset", "16")]
        ),
        [0xF2, 0x0F, 0x11, 0x84, 0x24, 0x10, 0, 0, 0]
    );
    // negative offset exercises the i32 parse branch:
    // movsd xmm8, [r8-8] (REX.R.B, disp32=-8).
    assert_eq!(
        enc(
            "ldr_d",
            &[("dst", "xmm8"), ("base", "r8"), ("offset", "-8")]
        ),
        [0xF2, 0x45, 0x0F, 0x10, 0x80, 0xF8, 0xFF, 0xFF, 0xFF]
    );
}

#[test]
fn v128_load_store_and_arith() {
    // ldr_q / str_q movups.
    assert_eq!(
        bytes(
            "ldr_q",
            &[("dst", "xmm0"), ("base", "rbx"), ("offset", "0")]
        ),
        [0x0F, 0x10, 0x83, 0, 0, 0, 0]
    );
    // str_q xmm8, [r8-16] : movups [r8-16], xmm8 (REX.R.B, disp32=-16).
    assert_eq!(
        enc(
            "str_q",
            &[("src", "xmm8"), ("base", "r8"), ("offset", "-16")]
        ),
        [0x45, 0x0F, 0x11, 0x80, 0xF0, 0xFF, 0xFF, 0xFF]
    );
    // Packed arithmetic: each vec3_op arm — disjoint (movaps xmm2,xmm0 then the
    // op), dst==lhs (op in place), dst==rhs (commutative swap, or staged via
    // xmm15 for the non-commutative sub/div/min/max).
    for (op, dis, lhs, rhs) in [
        (
            "fadd_v",
            &[0x0F, 0x28, 0xD0, 0x66, 0x0F, 0x58, 0xD1][..],
            &[0x66, 0x0F, 0x58, 0xC1][..],
            &[0x66, 0x0F, 0x58, 0xC8][..],
        ),
        (
            "fmul_v",
            &[0x0F, 0x28, 0xD0, 0x66, 0x0F, 0x59, 0xD1][..],
            &[0x66, 0x0F, 0x59, 0xC1][..],
            &[0x66, 0x0F, 0x59, 0xC8][..],
        ),
        (
            "fsub_v",
            &[0x0F, 0x28, 0xD0, 0x66, 0x0F, 0x5C, 0xD1][..],
            &[0x66, 0x0F, 0x5C, 0xC1][..],
            &[
                0x44, 0x0F, 0x28, 0xF9, 0x0F, 0x28, 0xC8, 0x66, 0x41, 0x0F, 0x5C, 0xCF,
            ][..],
        ),
        (
            "fdiv_v",
            &[0x0F, 0x28, 0xD0, 0x66, 0x0F, 0x5E, 0xD1][..],
            &[0x66, 0x0F, 0x5E, 0xC1][..],
            &[
                0x44, 0x0F, 0x28, 0xF9, 0x0F, 0x28, 0xC8, 0x66, 0x41, 0x0F, 0x5E, 0xCF,
            ][..],
        ),
        (
            "fmin_v",
            &[0x0F, 0x28, 0xD0, 0x66, 0x0F, 0x5D, 0xD1][..],
            &[0x66, 0x0F, 0x5D, 0xC1][..],
            &[
                0x44, 0x0F, 0x28, 0xF9, 0x0F, 0x28, 0xC8, 0x66, 0x41, 0x0F, 0x5D, 0xCF,
            ][..],
        ),
        (
            "fmax_v",
            &[0x0F, 0x28, 0xD0, 0x66, 0x0F, 0x5F, 0xD1][..],
            &[0x66, 0x0F, 0x5F, 0xC1][..],
            &[
                0x44, 0x0F, 0x28, 0xF9, 0x0F, 0x28, 0xC8, 0x66, 0x41, 0x0F, 0x5F, 0xCF,
            ][..],
        ),
        (
            "add_v",
            &[0x0F, 0x28, 0xD0, 0x66, 0x0F, 0xD4, 0xD1][..],
            &[0x66, 0x0F, 0xD4, 0xC1][..],
            &[0x66, 0x0F, 0xD4, 0xC8][..],
        ),
        (
            "sub_v",
            &[0x0F, 0x28, 0xD0, 0x66, 0x0F, 0xFB, 0xD1][..],
            &[0x66, 0x0F, 0xFB, 0xC1][..],
            &[
                0x44, 0x0F, 0x28, 0xF9, 0x0F, 0x28, 0xC8, 0x66, 0x41, 0x0F, 0xFB, 0xCF,
            ][..],
        ),
        (
            "and_v",
            &[0x0F, 0x28, 0xD0, 0x66, 0x0F, 0xDB, 0xD1][..],
            &[0x66, 0x0F, 0xDB, 0xC1][..],
            &[0x66, 0x0F, 0xDB, 0xC8][..],
        ),
        (
            "orr_v",
            &[0x0F, 0x28, 0xD0, 0x66, 0x0F, 0xEB, 0xD1][..],
            &[0x66, 0x0F, 0xEB, 0xC1][..],
            &[0x66, 0x0F, 0xEB, 0xC8][..],
        ),
        (
            "eor_v",
            &[0x0F, 0x28, 0xD0, 0x66, 0x0F, 0xEF, 0xD1][..],
            &[0x66, 0x0F, 0xEF, 0xC1][..],
            &[0x66, 0x0F, 0xEF, 0xC8][..],
        ),
    ] {
        assert_eq!(
            enc(op, &[("dst", "xmm2"), ("lhs", "xmm0"), ("rhs", "xmm1")]),
            dis,
            "{op} disjoint"
        );
        assert_eq!(
            enc(op, &[("dst", "xmm0"), ("lhs", "xmm0"), ("rhs", "xmm1")]),
            lhs,
            "{op} dst==lhs"
        );
        assert_eq!(
            enc(op, &[("dst", "xmm1"), ("lhs", "xmm0"), ("rhs", "xmm1")]),
            rhs,
            "{op} dst==rhs"
        );
    }
}

#[test]
fn v128_unary_and_negabs() {
    // fsqrt_v xmm1, xmm0 : sqrtpd xmm1,xmm0.
    assert_eq!(
        enc("fsqrt_v", &[("dst", "xmm1"), ("src", "xmm0")]),
        [0x66, 0x0F, 0x51, 0xC8]
    );
    // fneg_v / fabs_v: build the mask (pcmpeqd + psllq/psrlq) then xorpd/andpd;
    // the dst!=src arm inserts a movaps xmm1,xmm0 copy before the mask op.
    for (op, same, diff) in [
        (
            "fneg_v",
            &[
                0x66, 0x45, 0x0F, 0x76, 0xFF, 0x66, 0x41, 0x0F, 0x73, 0xF7, 0x3F, 0x66, 0x41, 0x0F,
                0x57, 0xC7,
            ][..],
            &[
                0x66, 0x45, 0x0F, 0x76, 0xFF, 0x66, 0x41, 0x0F, 0x73, 0xF7, 0x3F, 0x0F, 0x28, 0xC8,
                0x66, 0x41, 0x0F, 0x57, 0xCF,
            ][..],
        ),
        (
            "fabs_v",
            &[
                0x66, 0x45, 0x0F, 0x76, 0xFF, 0x66, 0x41, 0x0F, 0x73, 0xD7, 0x01, 0x66, 0x41, 0x0F,
                0x54, 0xC7,
            ][..],
            &[
                0x66, 0x45, 0x0F, 0x76, 0xFF, 0x66, 0x41, 0x0F, 0x73, 0xD7, 0x01, 0x0F, 0x28, 0xC8,
                0x66, 0x41, 0x0F, 0x54, 0xCF,
            ][..],
        ),
    ] {
        assert_eq!(
            enc(op, &[("dst", "xmm0"), ("src", "xmm0")]),
            same,
            "{op} dst==src"
        );
        assert_eq!(
            enc(op, &[("dst", "xmm1"), ("src", "xmm0")]),
            diff,
            "{op} dst!=src"
        );
    }
    // neg_v integer negate: pxor xmm15 ; psubq xmm15,xmm0 ; movaps xmm1,xmm15.
    assert_eq!(
        enc("neg_v", &[("dst", "xmm1"), ("src", "xmm0")]),
        [0x66, 0x45, 0x0F, 0xEF, 0xFF, 0x66, 0x44, 0x0F, 0xFB, 0xF8, 0x41, 0x0F, 0x28, 0xCF]
    );
    // abs_v dst==src (in place).
    assert_eq!(
        enc("abs_v", &[("dst", "xmm0"), ("src", "xmm0")]),
        [
            0x66, 0x45, 0x0F, 0xEF, 0xFF, 0x66, 0x44, 0x0F, 0x38, 0x37, 0xF8, 0x66, 0x41, 0x0F,
            0xEF, 0xC7, 0x66, 0x41, 0x0F, 0xFB, 0xC7
        ]
    );
    // abs_v dst!=src inserts movaps xmm1,xmm0 after the mask.
    assert_eq!(
        enc("abs_v", &[("dst", "xmm1"), ("src", "xmm0")]),
        [
            0x66, 0x45, 0x0F, 0xEF, 0xFF, 0x66, 0x44, 0x0F, 0x38, 0x37, 0xF8, 0x0F, 0x28, 0xC8,
            0x66, 0x41, 0x0F, 0xEF, 0xCF, 0x66, 0x41, 0x0F, 0xFB, 0xCF
        ]
    );
}
