//! plan-176-J: the x86-64 encoder's lanes_plans_and_aliasing tests. The shared `fresh_encoder`/`bytes`/\n//! `enc_err`/`enc` helpers live in `encode/tests/mod.rs`.

use super::*;

#[test]
fn v128_compares_against_zero() {
    // gt/ge move zero (xmm15) into dst then cmplt/cmple dst,src; lt/le/eq
    // compare dst against xmm15. `same` is dst==src==xmm0; `diff` targets xmm1.
    for (op, same, diff) in [
        (
            "fcmgt_zero_v",
            &[
                0x66, 0x45, 0x0F, 0xEF, 0xFF, 0x41, 0x0F, 0x28, 0xC7, 0x66, 0x0F, 0xC2, 0xC0, 0x01,
            ][..],
            &[
                0x66, 0x45, 0x0F, 0xEF, 0xFF, 0x41, 0x0F, 0x28, 0xCF, 0x66, 0x0F, 0xC2, 0xC8, 0x01,
            ][..],
        ),
        (
            "fcmge_zero_v",
            &[
                0x66, 0x45, 0x0F, 0xEF, 0xFF, 0x41, 0x0F, 0x28, 0xC7, 0x66, 0x0F, 0xC2, 0xC0, 0x02,
            ][..],
            &[
                0x66, 0x45, 0x0F, 0xEF, 0xFF, 0x41, 0x0F, 0x28, 0xCF, 0x66, 0x0F, 0xC2, 0xC8, 0x02,
            ][..],
        ),
        (
            "fcmlt_zero_v",
            &[
                0x66, 0x45, 0x0F, 0xEF, 0xFF, 0x66, 0x41, 0x0F, 0xC2, 0xC7, 0x01,
            ][..],
            &[
                0x66, 0x45, 0x0F, 0xEF, 0xFF, 0x0F, 0x28, 0xC8, 0x66, 0x41, 0x0F, 0xC2, 0xCF, 0x01,
            ][..],
        ),
        (
            "fcmle_zero_v",
            &[
                0x66, 0x45, 0x0F, 0xEF, 0xFF, 0x66, 0x41, 0x0F, 0xC2, 0xC7, 0x02,
            ][..],
            &[
                0x66, 0x45, 0x0F, 0xEF, 0xFF, 0x0F, 0x28, 0xC8, 0x66, 0x41, 0x0F, 0xC2, 0xCF, 0x02,
            ][..],
        ),
        (
            "fcmeq_zero_v",
            &[
                0x66, 0x45, 0x0F, 0xEF, 0xFF, 0x66, 0x41, 0x0F, 0xC2, 0xC7, 0x00,
            ][..],
            &[
                0x66, 0x45, 0x0F, 0xEF, 0xFF, 0x0F, 0x28, 0xC8, 0x66, 0x41, 0x0F, 0xC2, 0xCF, 0x00,
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
}

#[test]
fn v128_lane_shifts_and_moves() {
    // shl_v (psllq) / ushr_v (psrlq): in place, or copy-first (movaps xmm1,xmm0).
    for (op, same, diff) in [
        (
            "shl_v",
            &[0x66, 0x0F, 0x73, 0xF0, 0x03][..],
            &[0x0F, 0x28, 0xC8, 0x66, 0x0F, 0x73, 0xF1, 0x03][..],
        ),
        (
            "ushr_v",
            &[0x66, 0x0F, 0x73, 0xD0, 0x03][..],
            &[0x0F, 0x28, 0xC8, 0x66, 0x0F, 0x73, 0xD1, 0x03][..],
        ),
    ] {
        assert_eq!(
            enc(op, &[("dst", "xmm0"), ("src", "xmm0"), ("shift", "3")]),
            same,
            "{op} in place"
        );
        assert_eq!(
            enc(op, &[("dst", "xmm1"), ("src", "xmm0"), ("shift", "3")]),
            diff,
            "{op} copy-first"
        );
    }
    // dup_v_from_x xmm0, rbx : movq xmm0,rbx ; punpcklqdq xmm0,xmm0.
    assert_eq!(
        enc("dup_v_from_x", &[("dst", "xmm0"), ("src", "rbx")]),
        [0x66, 0x48, 0x0F, 0x6E, 0xC3, 0x66, 0x0F, 0x6C, 0xC0]
    );
    // umov_x_from_v lane 0 (movq rbx,xmm0).
    assert_eq!(
        enc(
            "umov_x_from_v",
            &[("dst", "rbx"), ("src", "xmm0"), ("index", "0")]
        ),
        [0x66, 0x48, 0x0F, 0x7E, 0xC3]
    );
    // umov_x_from_v lane 1 (pextrq rbx,xmm0,1).
    assert_eq!(
        enc(
            "umov_x_from_v",
            &[("dst", "rbx"), ("src", "xmm0"), ("index", "1")]
        ),
        [0x66, 0x48, 0x0F, 0x3A, 0x16, 0xC3, 0x01]
    );
    // sshr_v k=5 copy-first: sign mask (pxor/pcmpgtq/psllq 59) ; movaps xmm1,xmm0 ;
    // psrlq xmm1,5 ; por xmm1,mask.
    assert_eq!(
        enc(
            "sshr_v",
            &[("dst", "xmm1"), ("src", "xmm0"), ("shift", "5")]
        ),
        [
            0x66, 0x45, 0x0F, 0xEF, 0xFF, 0x66, 0x44, 0x0F, 0x38, 0x37, 0xF8, 0x66, 0x41, 0x0F,
            0x73, 0xF7, 0x3B, 0x0F, 0x28, 0xC8, 0x66, 0x0F, 0x73, 0xD1, 0x05, 0x66, 0x41, 0x0F,
            0xEB, 0xCF
        ]
    );
    // sshr_v k=0 in place: mask re-zeroed (pxor twice) so por is a no-op.
    assert_eq!(
        enc(
            "sshr_v",
            &[("dst", "xmm0"), ("src", "xmm0"), ("shift", "0")]
        ),
        [
            0x66, 0x45, 0x0F, 0xEF, 0xFF, 0x66, 0x44, 0x0F, 0x38, 0x37, 0xF8, 0x66, 0x45, 0x0F,
            0xEF, 0xFF, 0x66, 0x0F, 0x73, 0xD0, 0x00, 0x66, 0x41, 0x0F, 0xEB, 0xC7
        ]
    );
}

#[test]
fn v128_bit_selects_fma_and_serial_conversions() {
    // bsl_v: movaps xmm15,xmm0 ; pand xmm0,xmm1 ; pandn xmm15,xmm2 ; por xmm0,xmm15.
    assert_eq!(
        enc(
            "bsl_v",
            &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm2")]
        ),
        [
            0x44, 0x0F, 0x28, 0xF8, 0x66, 0x0F, 0xDB, 0xC1, 0x66, 0x44, 0x0F, 0xDF, 0xFA, 0x66,
            0x41, 0x0F, 0xEB, 0xC7
        ]
    );
    // bit_v: movaps xmm15,xmm0 ; pxor xmm15,xmm1 ; pand xmm15,xmm2 ; pxor xmm0,xmm15.
    assert_eq!(
        enc(
            "bit_v",
            &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm2")]
        ),
        [
            0x44, 0x0F, 0x28, 0xF8, 0x66, 0x44, 0x0F, 0xEF, 0xF9, 0x66, 0x44, 0x0F, 0xDB, 0xFA,
            0x66, 0x41, 0x0F, 0xEF, 0xC7
        ]
    );
    // fmla_v : vfmadd231pd xmm0,xmm1,xmm2.
    assert_eq!(
        enc(
            "fmla_v",
            &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm2")]
        ),
        [0xC4, 0xE2, 0xF1, 0xB8, 0xC2]
    );
    // fmls_v : vfnmadd231pd xmm0,xmm1,xmm2.
    assert_eq!(
        enc(
            "fmls_v",
            &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm2")]
        ),
        [0xC4, 0xE2, 0xF1, 0xBC, 0xC2]
    );
    // Extended reg drives the VEX P0/P1 R/B-bar bits: vfmadd231pd xmm8,xmm9,xmm10.
    assert_eq!(
        enc(
            "fmla_v",
            &[("dst", "xmm8"), ("lhs", "xmm9"), ("rhs", "xmm10")]
        ),
        [0xC4, 0x42, 0xB1, 0xB8, 0xC2]
    );
    // Lane-serial fcvtzs_v (i64<-f64), both lanes via pshufd, reassembled.
    assert_eq!(
        enc("fcvtzs_v", &[("dst", "xmm1"), ("src", "xmm0")]),
        [
            0x50, 0x52, 0xF2, 0x48, 0x0F, 0x2C, 0xC0, 0x66, 0x44, 0x0F, 0x70, 0xF8, 0xEE, 0xF2,
            0x49, 0x0F, 0x2C, 0xD7, 0x66, 0x48, 0x0F, 0x6E, 0xC8, 0x66, 0x4C, 0x0F, 0x6E, 0xFA,
            0x66, 0x41, 0x0F, 0x6C, 0xCF, 0x5A, 0x58
        ]
    );
    // Lane-serial scvtf_v (f64<-i64).
    assert_eq!(
        enc("scvtf_v", &[("dst", "xmm1"), ("src", "xmm0")]),
        [
            0x50, 0x52, 0x66, 0x48, 0x0F, 0x7E, 0xC0, 0x66, 0x44, 0x0F, 0x70, 0xF8, 0xEE, 0x66,
            0x4C, 0x0F, 0x7E, 0xFA, 0xF2, 0x48, 0x0F, 0x2A, 0xC8, 0xF2, 0x4C, 0x0F, 0x2A, 0xFA,
            0x66, 0x41, 0x0F, 0x14, 0xCF, 0x5A, 0x58
        ]
    );
}

#[test]
fn alu3_and_zero_and_error_arms() {
    // and rax, xzr, rbx : and with zero lhs → dst = 0 (xor dst,dst).
    assert_eq!(
        bytes("and", &[("dst", "rax"), ("lhs", "xzr"), ("rhs", "rbx")]),
        [0x48, 0x31, 0xC0]
    );
    // eor rax, xzr, rbx : xor with zero → dst = rhs, dst!=rhs → mov rax,rbx.
    assert_eq!(
        enc("eor", &[("dst", "rax"), ("lhs", "xzr"), ("rhs", "rbx")]),
        [0x48, 0x89, 0xD8]
    );
    // add with zero lhs, dst==rhs → nothing (empty bytes are valid).
    let _ = enc("add", &[("dst", "rax"), ("lhs", "xzr"), ("rhs", "rax")]);
    // zero-token rhs is an explicit error.
    let ins = CodeInstruction::new("add")
        .physical_field("dst", "rax")
        .physical_field("lhs", "rbx")
        .physical_field("rhs", "xzr");
    assert!(encode_instruction(&ins).is_err());
}

#[test]
fn add_carry_dst_not_lhs_and_sub_borrow_with_borrow_in() {
    // add_carry no carry-in, dst != lhs → mov rbx,rdi first:
    // mov rbx,rdi ; add rbx,r10 ; setb sil ; movzx rsi,sil.
    assert_eq!(
        enc(
            "add_carry",
            &[
                ("dst", "rbx"),
                ("carry_out", "rsi"),
                ("lhs", "rdi"),
                ("rhs", "r10"),
                ("carry_in", "xzr")
            ]
        ),
        [0x48, 0x89, 0xFB, 0x4C, 0x01, 0xD3, 0x40, 0x0F, 0x92, 0xC6, 0x48, 0x0F, 0xB6, 0xF6]
    );
    // sub_borrow with a borrow-in register, dst != lhs:
    // bt r11,0 ; mov rbx,rdi ; sbb rbx,r10 ; setb sil ; movzx rsi,sil.
    assert_eq!(
        enc(
            "sub_borrow",
            &[
                ("dst", "rbx"),
                ("borrow_out", "rsi"),
                ("lhs", "rdi"),
                ("rhs", "r10"),
                ("borrow_in", "r11")
            ]
        ),
        [
            0x49, 0x0F, 0xBA, 0xE3, 0x00, 0x48, 0x89, 0xFB, 0x4C, 0x19, 0xD3, 0x40, 0x0F, 0x92,
            0xC6, 0x48, 0x0F, 0xB6, 0xF6
        ]
    );
}

#[test]
fn immediate_and_disp_overflow_errors() {
    // A disp beyond i32 range is rejected.
    let big = (i32::MAX as u64) + 1;
    let ins = CodeInstruction::new("ldr_u64")
        .physical_field("dst", "rax")
        .physical_field("base", "rbx")
        .field("offset", &big.to_string());
    assert!(encode_instruction(&ins).is_err());
    // An imm beyond imm32 (and beyond sign-extension) is rejected.
    let huge = 0x1_0000_0000u64; // fits neither i32 nor i32-of-i64 sign form
    let ins = CodeInstruction::new("add_imm")
        .physical_field("dst", "rax")
        .physical_field("src", "rax")
        .field("imm", &huge.to_string());
    assert!(encode_instruction(&ins).is_err());
    // A -1-style mask (u64::MAX) is accepted via the sign-extended path:
    // add rax, -1 (48 81 C0 FF FF FF FF).
    assert_eq!(
        enc(
            "add_imm",
            &[
                ("dst", "rax"),
                ("src", "rax"),
                ("imm", &u64::MAX.to_string())
            ]
        ),
        [0x48, 0x81, 0xC0, 0xFF, 0xFF, 0xFF, 0xFF]
    );
}

/// Build a minimal single-function plan and run the whole two-pass `encode`,
/// exercising `emit_instruction`, `record_reloc` (internal/external/data/GOT),
/// and `patch_labels` — the `Encoder` methods the arm tests bypass.
fn ci(op: &str, fields: &[(&'static str, &str)]) -> CodeInstruction {
    let mut ins = CodeInstruction::new(op);
    for (k, v) in fields {
        ins = ins.physical_field(k, v);
    }
    ins
}

fn plan_with(
    instructions: Vec<CodeInstruction>,
    imports: Vec<CodeImport>,
    data_objects: Vec<CodeDataObject>,
) -> NativeCodePlan {
    NativeCodePlan {
        target: "linux-x86_64".to_string(),
        build_mode: crate::target::NativeBuildMode::Console,
        arch: "x86_64".to_string(),
        project: "t".to_string(),
        entry_symbol: Some("_mfb_main".to_string()),
        imports,
        data_objects,
        functions: vec![CodeFunction {
            name: "main".to_string(),
            symbol: "_mfb_main".to_string(),
            params: Vec::new(),
            returns: "Void".to_string(),
            frame: CodeFrame {
                stack_size: 0,
                callee_saved: Vec::new(),
            },
            instructions,
            relocations: Vec::new(),
            stack_slots: Vec::new(),
        }],
    }
}

#[test]
fn encode_full_plan_labels_calls_and_data() {
    // An internal call (`main` resolves as an internal symbol), a forward branch
    // patched by `patch_labels`, a data-address `adrp` (internal → data reloc),
    // and a ret.
    let plan = plan_with(
        vec![
            ci("b.eq", &[("target", "done")]),
            ci("bl", &[("target", "_mfb_main")]), // internal (self) call, 5 bytes
            ci("adrp", &[("dst", "rsi"), ("symbol", "msg")]),
            ci("label", &[("name", "done")]),
            ci("ret", &[]),
        ],
        vec![],
        vec![CodeDataObject {
            symbol: "msg".to_string(),
            kind: "string".to_string(),
            layout: "utf8".to_string(),
            align: 8,
            size: 16,
            value: "hi".to_string(),
        }],
    );
    let image = super::encode(&plan).expect("encode");
    assert!(!image.text.is_empty());
    // The forward `b.eq done` rel32 was patched: its 4-byte disp is the distance
    // from the end of the jcc to the `done` label. The jcc is 6 bytes; after it
    // come bl (5) + adrp (7) = 12 bytes to reach `done`.
    let disp = i32::from_le_bytes([image.text[2], image.text[3], image.text[4], image.text[5]]);
    assert_eq!(disp, 12);
    // The internal call and the data address both produced relocations.
    assert!(image
        .relocations
        .iter()
        .any(|r| r.binding == "internal" && r.target == "_mfb_main"));
    assert!(image
        .relocations
        .iter()
        .any(|r| r.binding == "data" && r.target == "msg"));
}

#[test]
fn duplicate_label_in_a_function_is_rejected() {
    // bug-15: `labels.insert` is last-writer-wins, so a duplicate name silently
    // resolved every reference to the final definition. Reject it instead.
    let plan = plan_with(
        vec![
            ci("x86.jp", &[("target", "dup")]),
            ci("label", &[("name", "dup")]),
            ci("label", &[("name", "dup")]),
            ci("ret", &[]),
        ],
        vec![],
        vec![],
    );
    let error = match super::encode(&plan) {
        Ok(_) => panic!("duplicate label must not encode"),
        Err(error) => error,
    };
    assert!(error.contains("duplicate label 'dup'"), "{error}");
}

#[test]
fn ordered_only_float_branch_jp_skips_only_its_own_jcc() {
    // bug-15: two ordered-only float branches to the same target. Each `jp` must
    // resolve to its OWN skip label — i.e. jump exactly over the 6-byte `jcc`
    // that follows it — not to the second branch's label.
    let plan = plan_with(
        vec![
            ci("fcmp_d", &[("lhs", "xmm0"), ("rhs", "xmm1")]),
            ci("x86.jp", &[("target", "L__x86ford0")]),
            ci("x86.jb", &[("target", "L")]),
            ci("label", &[("name", "L__x86ford0")]),
            ci("fcmp_d", &[("lhs", "xmm2"), ("rhs", "xmm3")]),
            ci("x86.jp", &[("target", "L__x86ford1")]),
            ci("x86.jb", &[("target", "L")]),
            ci("label", &[("name", "L__x86ford1")]),
            ci("label", &[("name", "L")]),
            ci("ret", &[]),
        ],
        vec![],
        vec![],
    );
    let image = super::encode(&plan).expect("encode");
    // Both `jp`s and `jb`s are 6-byte near jumps (0x0f 0x8a/0x82 + rel32). Find
    // each `jp` (0x0f 0x8a) and assert its displacement is 6 — over its own jcc.
    let jps: Vec<usize> = (0..image.text.len().saturating_sub(6))
        .filter(|&i| image.text[i] == 0x0f && image.text[i + 1] == 0x8a)
        .collect();
    assert_eq!(jps.len(), 2, "two jp instructions");
    for at in jps {
        let disp = i32::from_le_bytes([
            image.text[at + 2],
            image.text[at + 3],
            image.text[at + 4],
            image.text[at + 5],
        ]);
        assert_eq!(disp, 6, "jp at {at} must skip only its own 6-byte jcc");
    }
}

#[test]
fn encode_external_call_and_got_load() {
    // An imported symbol: `bl` routes to an external reloc, and an `adrp` against
    // the same import re-routes through the GOT (`got_pc32`).
    let plan = plan_with(
        vec![
            ci("bl", &[("target", "snprintf")]),
            ci("adrp", &[("dst", "rsi"), ("symbol", "snprintf")]),
            ci("ret", &[]),
        ],
        vec![CodeImport {
            library: "libc".to_string(),
            symbol: "snprintf".to_string(),
        }],
        vec![],
    );
    let image = super::encode(&plan).expect("encode");
    assert!(image
        .relocations
        .iter()
        .any(|r| r.binding == "external" && r.library.as_deref() == Some("libc")));
    // The GOT-routed data load carries the GotLoadLo kind.
    let got_kind = crate::arch::x86_64::reloc::reloc_kind(
        crate::codegen::engine::types::RelocIntent::GotLoadLo,
    );
    assert!(image.relocations.iter().any(|r| r.kind == got_kind));
}

#[test]
fn encode_unresolved_call_and_label_error() {
    // A `bl` to a symbol that is neither internal nor imported is an error.
    let plan = plan_with(
        vec![ci("bl", &[("target", "nope")]), ci("ret", &[])],
        vec![],
        vec![],
    );
    assert!(super::encode(&plan).is_err());
    // A branch to a label that never appears is a `patch_labels` error.
    let plan = plan_with(
        vec![ci("b", &[("target", "missing")]), ci("ret", &[])],
        vec![],
        vec![],
    );
    assert!(super::encode(&plan).is_err());
}

/// bug-284 C6: several x86-64 expansions use fixed registers beyond their named
/// operands, and an operand coloured onto one of those produces wrong bytes
/// silently. Each combination below has no correct encoding in its expansion, so
/// the encoder rejects it rather than emitting something plausible.
#[test]
fn fixed_register_aliasing_is_rejected_rather_than_miscompiled() {
    let err = |op: &str, fields: &[(&'static str, &str)]| -> String {
        let mut ins = CodeInstruction::new(op);
        for (k, v) in fields {
            ins = ins.physical_field(k, v);
        }
        match encode_instruction(&ins) {
            Ok(_) => panic!("{op} must reject this fixed-register aliasing"),
            Err(err) => err,
        }
    };

    // (plan-85: a variable shift with dst==rcx is NO LONGER rejected — the encoder
    // shifts in a scratch and moves the result into rcx; see
    // `variable_shift_targeting_rcx_shifts_in_a_scratch`.)

    // msub stages the product in rax: a dst there is subtracted from itself (0),
    // and an rhs there is destroyed before the multiply (yielding lhs*lhs).
    let message = err(
        "msub",
        &[
            ("dst", "rax"),
            ("lhs", "r10"),
            ("rhs", "r11"),
            ("minuend", "r12"),
        ],
    );
    assert!(message.contains("rax"), "unexpected error: {message}");
    let message = err(
        "msub",
        &[
            ("dst", "r10"),
            ("lhs", "r11"),
            ("rhs", "rax"),
            ("minuend", "r12"),
        ],
    );
    assert!(message.contains("rax"), "unexpected error: {message}");

    // rbit uses rax as its mask register and rdx as its accumulator, both saved
    // and restored around the sequence -- so a dst on either is discarded.
    for reg in ["rax", "rdx"] {
        let message = err("rbit", &[("dst", reg), ("src", "r10")]);
        assert!(message.contains(reg), "unexpected error: {message}");
    }

    // The same ops on allocatable registers are unaffected.
    // lslv r10, r11, r12 : push rcx ; mov rcx,r12 ; mov r10,r11 ; shl r10,cl ; pop rcx.
    assert_eq!(
        enc("lslv", &[("dst", "r10"), ("lhs", "r11"), ("rhs", "r12")]),
        [0x51, 0x4C, 0x89, 0xE1, 0x4D, 0x89, 0xDA, 0x49, 0xD3, 0xE2, 0x59]
    );
    // rbit r10, r11 : push rax/rdx ; three mask/shift rounds ; bswap r10 ; pop rdx/rax.
    assert_eq!(
        enc("rbit", &[("dst", "r10"), ("src", "r11")]),
        [
            0x50, 0x52, 0x4D, 0x89, 0xDA, 0x4C, 0x89, 0xD2, 0x48, 0xB8, 0x55, 0x55, 0x55, 0x55,
            0x55, 0x55, 0x55, 0x55, 0x48, 0x21, 0xC2, 0x48, 0xC1, 0xE2, 0x01, 0x49, 0xC1, 0xEA,
            0x01, 0x49, 0x21, 0xC2, 0x49, 0x09, 0xD2, 0x4C, 0x89, 0xD2, 0x48, 0xB8, 0x33, 0x33,
            0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x48, 0x21, 0xC2, 0x48, 0xC1, 0xE2, 0x02, 0x49,
            0xC1, 0xEA, 0x02, 0x49, 0x21, 0xC2, 0x49, 0x09, 0xD2, 0x4C, 0x89, 0xD2, 0x48, 0xB8,
            0x0F, 0x0F, 0x0F, 0x0F, 0x0F, 0x0F, 0x0F, 0x0F, 0x48, 0x21, 0xC2, 0x48, 0xC1, 0xE2,
            0x04, 0x49, 0xC1, 0xEA, 0x04, 0x49, 0x21, 0xC2, 0x49, 0x09, 0xD2, 0x49, 0x0F, 0xCA,
            0x5A, 0x58
        ]
    );
    // msub r10, r11, r12, r14 : mov rax,r11 ; imul rax,r12 ; mov r10,r14 ; sub r10,rax.
    assert_eq!(
        enc(
            "msub",
            &[
                ("dst", "r10"),
                ("lhs", "r11"),
                ("rhs", "r12"),
                ("minuend", "r14"),
            ]
        ),
        [0x4C, 0x89, 0xD8, 0x49, 0x0F, 0xAF, 0xC4, 0x4D, 0x89, 0xF2, 0x49, 0x29, 0xC2]
    );
}

/// bug-295: the ties-away emulation's *arithmetic*, checked independently of the
/// encoding. This models exactly what the emitted sequence computes —
/// `t = trunc(x); f = t − x; d = trunc(2f); result = t − d` — and pins it against
/// the old `trunc(x + copysign(0.5, x))` formula, which double-rounds.
///
/// The x86 backend cannot be executed from this host, so this is what proves the
/// replacement is right rather than merely different: every step below is exact in
/// IEEE-754 double, so evaluating the model in Rust computes the same values the
/// SSE sequence does.
#[test]
fn ties_away_model_matches_aarch64_semantics() {
    // What the emitted sequence computes.
    let fixed = |x: f64| -> f64 {
        let t = x.trunc();
        let f = t - x;
        t - (2.0 * f).trunc()
    };
    // What it computed before: the addition itself rounds.
    let broken = |x: f64| -> f64 { (x + 0.5f64.copysign(x)).trunc() };

    // The reported input: 0.5 − 2⁻⁵⁴, strictly below one half, so ties-away is 0.
    let below_half = 0.499_999_999_999_999_94_f64;
    assert!(below_half < 0.5, "fixture must be strictly below one half");
    assert_eq!(broken(below_half), 1.0, "the old formula's double rounding");
    assert_eq!(broken(-below_half), -1.0);
    assert_eq!(fixed(below_half), 0.0);
    assert_eq!(fixed(-below_half), -0.0);

    // Genuine ties still round AWAY from zero (not to even, which is what the
    // native SSE roundsd/cvtsd2si modes would have given).
    for (input, expected) in [
        (0.5, 1.0),
        (-0.5, -1.0),
        (1.5, 2.0),
        (-1.5, -2.0),
        (2.5, 3.0),
        (-2.5, -3.0),
        (3.5, 4.0),
    ] {
        assert_eq!(
            fixed(input),
            expected,
            "tie {input} must round away from zero"
        );
    }

    // Ordinary values are unchanged, including either side of a half.
    for (input, expected) in [
        (0.0, 0.0),
        (0.4, 0.0),
        (0.6, 1.0),
        (2.4, 2.0),
        (2.6, 3.0),
        (-2.4, -2.0),
        (-2.6, -3.0),
        (1e15, 1e15),
        (-1e15, -1e15),
    ] {
        assert_eq!(fixed(input), expected, "round({input})");
    }

    // Beyond 2^52 every double is already an integer: f is zero, so the
    // correction term vanishes and the value passes through untouched.
    for big in [4.503_599_627_370_496e15_f64, 1e300, -1e300] {
        assert_eq!(fixed(big), big);
    }

    // The fix never disagrees with the old formula except on the family the old
    // one got wrong -- swept across a range of exponents.
    for exponent in -60..60 {
        let scale = 2.0f64.powi(exponent);
        for step in [0.0, 0.25, 0.5, 0.75, 1.0, 2.5, 7.5] {
            let x = step * scale;
            let t = x.trunc();
            let exact_half = (x - t).abs() == 0.5;
            if !exact_half && x.abs() < 4.503_599_627_370_496e15 {
                // Away from a genuine tie both agree, so the change is surgical.
                assert_eq!(fixed(x), broken(x), "disagreement at {x}");
            }
        }
    }
}

/// bug-295: byte-exact encodings of the replacement sequences, verified against
/// llvm-objdump's disassembly of the emitted bytes.
#[test]
fn ties_away_encodes_the_exact_fraction_sequence() {
    // pushq %rax ; cvttsd2si %xmm3,%r10 ; cvtsi2sd %r10,%xmm15 ;
    // subsd %xmm3,%xmm15 ; addsd %xmm15,%xmm15 ; roundsd $3,%xmm15,%xmm15 ;
    // cvttsd2si %xmm15,%rax ; subq %rax,%r10 ; popq %rax
    assert_eq!(
        bytes("fcvtas_x_from_d", &[("dst", "r10"), ("src", "xmm3")]),
        [
            0x50, 0xf2, 0x4c, 0x0f, 0x2c, 0xd3, 0xf2, 0x4d, 0x0f, 0x2a, 0xfa, 0xf2, 0x44, 0x0f,
            0x5c, 0xfb, 0xf2, 0x45, 0x0f, 0x58, 0xff, 0x66, 0x45, 0x0f, 0x3a, 0x0b, 0xff, 0x03,
            0xf2, 0x49, 0x0f, 0x2c, 0xc7, 0x49, 0x29, 0xc2, 0x58
        ]
    );
    // roundpd $3,%xmm3,%xmm2 ; movapd %xmm2,%xmm15 ; subpd %xmm3,%xmm15 ;
    // addpd %xmm15,%xmm15 ; roundpd $3,%xmm15,%xmm15 ; subpd %xmm15,%xmm2
    assert_eq!(
        bytes("frinta_v", &[("dst", "xmm2"), ("src", "xmm3")]),
        [
            0x66, 0x0f, 0x3a, 0x09, 0xd3, 0x03, 0x66, 0x44, 0x0f, 0x28, 0xfa, 0x66, 0x44, 0x0f,
            0x5c, 0xfb, 0x66, 0x45, 0x0f, 0x58, 0xff, 0x66, 0x45, 0x0f, 0x3a, 0x09, 0xff, 0x03,
            0x66, 0x41, 0x0f, 0x5c, 0xd7
        ]
    );
    // The commandeered GPR is chosen to differ from dst: with dst == rax it must not
    // push/pop the register it is about to return in.
    let with_rax_dst = bytes("fcvtas_x_from_d", &[("dst", "rax"), ("src", "xmm3")]);
    assert_eq!(
        with_rax_dst[0], 0x51,
        "dst == rax must commandeer rcx, not rax"
    );
    assert_eq!(*with_rax_dst.last().unwrap(), 0x59, "and restore rcx");
}

// --- plan-68-D7: previously-uncovered encode arms --------------------------

#[test]
fn sxtw_sign_extends_low_32() {
    // movsxd rax, ebx : REX.W 63 /r → 48 63 C3
    assert_eq!(
        bytes("sxtw", &[("dst", "rax"), ("src", "rbx")]),
        [0x48, 0x63, 0xC3]
    );
    // movsxd r8, r15d : REX.W.R.B 63 /r → 4D 63 C7
    assert_eq!(
        bytes("sxtw", &[("dst", "r8"), ("src", "r15")]),
        [0x4D, 0x63, 0xC7]
    );
}

#[test]
fn store_zero_immediate_by_width() {
    // str_u64 xzr, [rax+0] : mov qword [rax], 0 → 48 C7 80 00000000 00000000
    assert_eq!(
        bytes(
            "str_u64",
            &[("src", "xzr"), ("base", "rax"), ("offset", "0")]
        ),
        [0x48, 0xC7, 0x80, 0, 0, 0, 0, 0, 0, 0, 0]
    );
    // str_u32 xzr, [r8+0] : mov dword [r8], 0 (REX.B) → 41 C7 80 00000000 00000000
    assert_eq!(
        bytes(
            "str_u32",
            &[("src", "xzr"), ("base", "r8"), ("offset", "0")]
        ),
        [0x41, 0xC7, 0x80, 0, 0, 0, 0, 0, 0, 0, 0]
    );
    // str_u16 xzr, [r8+0] : mov word [r8], 0 (66 prefix + REX.B) → 66 41 C7 80 00000000 0000
    assert_eq!(
        bytes(
            "str_u16",
            &[("src", "xzr"), ("base", "r8"), ("offset", "0")]
        ),
        [0x66, 0x41, 0xC7, 0x80, 0, 0, 0, 0, 0, 0]
    );
    // str_u8 xzr, [r8+0] : mov byte [r8], 0 (REX.B) → 41 C6 80 00000000 00
    assert_eq!(
        bytes("str_u8", &[("src", "xzr"), ("base", "r8"), ("offset", "0")]),
        [0x41, 0xC6, 0x80, 0, 0, 0, 0, 0]
    );
}

#[test]
fn variable_shift_targeting_rcx_shifts_in_a_scratch() {
    // plan-85: Win64's aligned MFB result bank starts at rcx, so a variable-shifted
    // result can legitimately land on rcx. rcx carries the shift COUNT, so the shift
    // runs in a scratch register (the first reg != rcx/value/amount, here rbx),
    // preserved with push/pop, then the result is moved into rcx.
    // lslv rcx, rax, rdx: push rbx (53) ; mov rbx,rax (48 89 C3) ;
    // mov rcx,rdx (48 89 D1) ; shl rbx,cl (48 D3 E3) ; mov rcx,rbx (48 89 D9) ;
    // pop rbx (5B).
    assert_eq!(
        bytes("lslv", &[("dst", "rcx"), ("lhs", "rax"), ("rhs", "rdx")]),
        [0x53, 0x48, 0x89, 0xC3, 0x48, 0x89, 0xD1, 0x48, 0xD3, 0xE3, 0x48, 0x89, 0xD9, 0x5B]
    );
}

#[test]
fn variable_shift_reloads_value_aliasing_rcx() {
    // lslv rax, rcx, rdx : the value operand aliases rcx and is overwritten by
    // the count staging, so it is reloaded from [rsp].
    // push rcx (51) ; mov rcx,rdx (48 89 D1) ; mov rax,[rsp] (48 8B 04 24) ;
    // shl rax,cl (48 D3 E0) ; pop rcx (59)
    assert_eq!(
        bytes("lslv", &[("dst", "rax"), ("lhs", "rcx"), ("rhs", "rdx")]),
        [0x51, 0x48, 0x89, 0xD1, 0x48, 0x8B, 0x04, 0x24, 0x48, 0xD3, 0xE0, 0x59]
    );
}

#[test]
fn variable_shift_w_reloads_value_aliasing_rcx() {
    // rorv_w rax, rcx, rdx : 32-bit shift; value aliases rcx, reloaded from [rsp].
    // push rcx (51) ; mov ecx,edx (89 D1) ; mov eax,[rsp] (8B 04 24) ;
    // ror eax,cl (D3 C8) ; pop rcx (59)
    assert_eq!(
        bytes("rorv_w", &[("dst", "rax"), ("lhs", "rcx"), ("rhs", "rdx")]),
        [0x51, 0x89, 0xD1, 0x8B, 0x04, 0x24, 0xD3, 0xC8, 0x59]
    );
}

#[test]
fn add_carry_dst_aliases_rhs_no_carry_in() {
    // dst==rhs, carry_in=xzr, carry_out=xzr: add lhs in place (add commutes).
    // add rbx, rax (48 01 C3)
    assert_eq!(
        bytes(
            "add_carry",
            &[
                ("dst", "rbx"),
                ("carry_out", "xzr"),
                ("lhs", "rax"),
                ("rhs", "rbx"),
                ("carry_in", "xzr"),
            ]
        ),
        [0x48, 0x01, 0xC3]
    );
}

#[test]
fn add_carry_dst_aliases_rhs_with_carry_in() {
    // dst==rhs, carry_in=r10, carry_out=xzr: bt r10,0 ; adc rbx,rax.
    assert_eq!(
        bytes(
            "add_carry",
            &[
                ("dst", "rbx"),
                ("carry_out", "xzr"),
                ("lhs", "rax"),
                ("rhs", "rbx"),
                ("carry_in", "r10"),
            ]
        ),
        [0x49, 0x0F, 0xBA, 0xE2, 0x00, 0x48, 0x11, 0xC3]
    );
}

#[test]
fn add_carry_register_carry_in_distinct_operands() {
    // carry_in=r10, dst!=lhs!=rhs: bt r10,0 ; mov rbx,rax ; adc rbx,rcx.
    assert_eq!(
        bytes(
            "add_carry",
            &[
                ("dst", "rbx"),
                ("carry_out", "xzr"),
                ("lhs", "rax"),
                ("rhs", "rcx"),
                ("carry_in", "r10"),
            ]
        ),
        [0x49, 0x0F, 0xBA, 0xE2, 0x00, 0x48, 0x89, 0xC3, 0x48, 0x11, 0xCB]
    );
}

#[test]
fn add_carry_register_carry_in_zero_rhs() {
    // carry_in=r10, rhs=xzr, dst!=lhs: bt r10,0 ; mov rbx,rax ; adc rbx,0.
    assert_eq!(
        bytes(
            "add_carry",
            &[
                ("dst", "rbx"),
                ("carry_out", "xzr"),
                ("lhs", "rax"),
                ("rhs", "xzr"),
                ("carry_in", "r10"),
            ]
        ),
        [0x49, 0x0F, 0xBA, 0xE2, 0x00, 0x48, 0x89, 0xC3, 0x48, 0x81, 0xD3, 0, 0, 0, 0]
    );
}

#[test]
fn sub_borrow_zero_rhs_no_borrow_in_moves() {
    // borrow_in=xzr, rhs=xzr, dst!=lhs: mov rbx,rax (nothing to subtract).
    assert_eq!(
        bytes(
            "sub_borrow",
            &[
                ("dst", "rbx"),
                ("borrow_out", "xzr"),
                ("lhs", "rax"),
                ("rhs", "xzr"),
                ("borrow_in", "xzr"),
            ]
        ),
        [0x48, 0x89, 0xC3]
    );
}

#[test]
fn sub_borrow_dst_aliases_rhs_no_borrow_in() {
    // borrow_in=xzr, dst==rhs: neg rbx ; add rbx,rax  (dst = lhs - rhs).
    assert_eq!(
        bytes(
            "sub_borrow",
            &[
                ("dst", "rbx"),
                ("borrow_out", "xzr"),
                ("lhs", "rax"),
                ("rhs", "rbx"),
                ("borrow_in", "xzr"),
            ]
        ),
        [0x48, 0xF7, 0xDB, 0x48, 0x01, 0xC3]
    );
}

#[test]
fn sub_borrow_zero_rhs_register_borrow_in() {
    // borrow_in=r10, rhs=xzr, dst!=lhs: bt r10,0 ; mov rbx,rax ; sbb rbx,0.
    assert_eq!(
        bytes(
            "sub_borrow",
            &[
                ("dst", "rbx"),
                ("borrow_out", "xzr"),
                ("lhs", "rax"),
                ("rhs", "xzr"),
                ("borrow_in", "r10"),
            ]
        ),
        [0x49, 0x0F, 0xBA, 0xE2, 0x00, 0x48, 0x89, 0xC3, 0x48, 0x81, 0xDB, 0, 0, 0, 0]
    );
}

#[test]
fn sub_borrow_dst_aliases_rhs_with_register_borrow_in_is_rejected() {
    // dst==rhs with a register borrow-in has no scratch-free 2-op form.
    let mut ins = CodeInstruction::new("sub_borrow");
    ins = ins
        .physical_field("dst", "rbx")
        .physical_field("borrow_out", "xzr")
        .physical_field("lhs", "rax")
        .physical_field("rhs", "rbx")
        .physical_field("borrow_in", "r10");
    assert!(enc_err(&ins).contains("dst aliasing rhs"));
}

#[test]
fn umulh_stages_rhs_aliasing_rax() {
    // umulh rbx, rsi, rax : rhs is rax, which `mov rax,lhs` would clobber, so it
    // is staged on the stack and the multiply reads [rsp].
    // push rax ; push rdx ; sub rsp,8 ; mov [rsp],rax ; mov rax,rsi ;
    // mul qword [rsp] ; add rsp,8 ; mov rbx,rdx ; pop rdx ; pop rax
    assert_eq!(
        bytes("umulh", &[("dst", "rbx"), ("lhs", "rsi"), ("rhs", "rax")]),
        [
            0x50, 0x52, 0x48, 0x81, 0xEC, 0x08, 0x00, 0x00, 0x00, 0x48, 0x89, 0x04, 0x24, 0x48,
            0x89, 0xF0, 0x48, 0xF7, 0x24, 0x24, 0x48, 0x81, 0xC4, 0x08, 0x00, 0x00, 0x00, 0x48,
            0x89, 0xD3, 0x5A, 0x58
        ]
    );
}

/// bug-788: the binary32 conversions a `LINK` `CFloat` crosses.
#[test]
fn float_precision_conversions_encode() {
    // cvtsd2ss xmm0, xmm1 : F2 0F 5A C1
    assert_eq!(
        bytes("fcvt_s_from_d", &[("dst", "xmm0"), ("src", "xmm1")]),
        [0xF2, 0x0F, 0x5A, 0xC1]
    );
    // cvtss2sd xmm0, xmm1 : F3 0F 5A C1
    assert_eq!(
        bytes("fcvt_d_from_s", &[("dst", "xmm0"), ("src", "xmm1")]),
        [0xF3, 0x0F, 0x5A, 0xC1]
    );
    // cvtsd2ss xmm9, xmm1 : F2 44 0F 5A C9 (REX.R, prefix before REX)
    assert_eq!(
        bytes("fcvt_s_from_d", &[("dst", "xmm9"), ("src", "xmm1")]),
        [0xF2, 0x44, 0x0F, 0x5A, 0xC9]
    );
}
