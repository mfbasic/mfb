//! The decoder against the encoder: every form `encode::emitter` writes is
//! encoded here and decoded back to its expected text. The texts follow the
//! LLVM disassembler lldb uses (checked against `otool -tV` over the example
//! programs), and the guard at the end fails when the encoder gains a form
//! this table does not cover.

use super::{decode, Flow};
use crate::arch::aarch64::encode::encode_instruction;
use crate::codegen::engine::types::CodeInstruction;

const PC: u64 = 0x1_0000_1000;

/// Encode `instruction` and decode each word it produced, at `PC` onwards.
fn round_trip(instruction: &CodeInstruction) -> Vec<String> {
    let bytes = encode_instruction(instruction).expect("the encoder accepts it");
    bytes
        .chunks_exact(4)
        .enumerate()
        .map(|(index, word)| {
            let word = u32::from_le_bytes(word.try_into().unwrap());
            decode(word, PC + index as u64 * 4)
                .unwrap_or_else(|| panic!("{word:#010x} does not decode"))
                .text
        })
        .collect()
}

fn op(mnemonic: &str, fields: &[(&'static str, &str)]) -> CodeInstruction {
    fields.iter().fold(
        CodeInstruction::new(mnemonic),
        |instruction, (name, value)| instruction.physical_field(name, *value),
    )
}

/// One form: its mnemonic, its operand fields, and the lines it decodes to.
type Case = (
    &'static str,
    Vec<(&'static str, &'static str)>,
    Vec<&'static str>,
);

/// Every form the encoder emits.
fn table() -> Vec<Case> {
    let three = |dst, lhs, rhs| vec![("dst", dst), ("lhs", lhs), ("rhs", rhs)];
    let two = |dst, src| vec![("dst", dst), ("src", src)];
    let load = |dst, base, offset| vec![("dst", dst), ("base", base), ("offset", offset)];
    let store = |src, base, offset| vec![("src", src), ("base", base), ("offset", offset)];
    vec![
        ("mov", two("x0", "x1"), vec!["mov    x0, x1"]),
        ("mov", two("sp", "x9"), vec!["mov    sp, x9"]),
        ("mov", two("x9", "sp"), vec!["mov    x9, sp"]),
        (
            "mov_imm",
            vec![("dst", "x3"), ("value", "74565")],
            vec!["mov    x3, #0x2345", "movk   x3, #0x1, lsl #16"],
        ),
        (
            "mov_imm",
            vec![("dst", "x8"), ("value", "9223372036854775808")],
            vec!["mov    x8, #0x0", "movk   x8, #0x8000, lsl #48"],
        ),
        ("add", three("x0", "x1", "x2"), vec!["add    x0, x1, x2"]),
        ("adds", three("x0", "x1", "x2"), vec!["adds   x0, x1, x2"]),
        ("sub", three("x0", "x1", "x2"), vec!["sub    x0, x1, x2"]),
        ("sub", three("x0", "xzr", "x2"), vec!["neg    x0, x2"]),
        ("subs", three("x0", "x1", "x2"), vec!["subs   x0, x1, x2"]),
        ("and", three("x0", "x1", "x2"), vec!["and    x0, x1, x2"]),
        ("orr", three("x0", "x1", "x2"), vec!["orr    x0, x1, x2"]),
        ("eor", three("x0", "x1", "x2"), vec!["eor    x0, x1, x2"]),
        ("mvn", two("x0", "x1"), vec!["mvn    x0, x1"]),
        ("mul", three("x0", "x1", "x2"), vec!["mul    x0, x1, x2"]),
        ("smulh", three("x0", "x1", "x2"), vec!["smulh  x0, x1, x2"]),
        ("umulh", three("x0", "x1", "x2"), vec!["umulh  x0, x1, x2"]),
        (
            "add_carry",
            vec![
                ("dst", "x0"),
                ("carry_out", "x1"),
                ("lhs", "x2"),
                ("rhs", "x3"),
                ("carry_in", "xzr"),
            ],
            vec!["adds   x0, x2, x3", "cset   x1, hs"],
        ),
        (
            "add_carry",
            vec![
                ("dst", "x0"),
                ("carry_out", "x1"),
                ("lhs", "x2"),
                ("rhs", "x3"),
                ("carry_in", "x4"),
            ],
            vec!["cmp    x4, #0x1", "adcs   x0, x2, x3", "cset   x1, hs"],
        ),
        (
            "sub_borrow",
            vec![
                ("dst", "x0"),
                ("borrow_out", "x1"),
                ("lhs", "x2"),
                ("rhs", "x3"),
                ("borrow_in", "x4"),
            ],
            vec!["cmp    xzr, x4", "sbcs   x0, x2, x3", "cset   x1, lo"],
        ),
        ("rorv", three("x0", "x1", "x2"), vec!["ror    x0, x1, x2"]),
        ("rorv_w", three("x0", "x1", "x2"), vec!["ror    w0, w1, w2"]),
        ("lslv", three("x0", "x1", "x2"), vec!["lsl    x0, x1, x2"]),
        ("lsrv", three("x0", "x1", "x2"), vec!["lsr    x0, x1, x2"]),
        ("asrv", three("x0", "x1", "x2"), vec!["asr    x0, x1, x2"]),
        ("clz", two("x0", "x1"), vec!["clz    x0, x1"]),
        ("rbit", two("x0", "x1"), vec!["rbit   x0, x1"]),
        ("rev_w", two("x0", "x1"), vec!["rev    w0, w1"]),
        ("rev_x", two("x0", "x1"), vec!["rev    x0, x1"]),
        ("sxtw", two("x0", "x1"), vec!["sxtw   x0, w1"]),
        ("sdiv", three("x0", "x1", "x2"), vec!["sdiv   x0, x1, x2"]),
        ("udiv", three("x0", "x1", "x2"), vec!["udiv   x0, x1, x2"]),
        (
            "msub",
            vec![
                ("dst", "x0"),
                ("lhs", "x1"),
                ("rhs", "x2"),
                ("minuend", "x3"),
            ],
            vec!["msub   x0, x1, x2, x3"],
        ),
        (
            "lsl_imm",
            vec![("dst", "x0"), ("src", "x1"), ("shift", "3")],
            vec!["lsl    x0, x1, #3"],
        ),
        (
            "lsr_imm",
            vec![("dst", "x0"), ("src", "x1"), ("shift", "5")],
            vec!["lsr    x0, x1, #5"],
        ),
        (
            "asr_imm",
            vec![("dst", "x0"), ("src", "x1"), ("shift", "63")],
            vec!["asr    x0, x1, #63"],
        ),
        (
            "add_imm",
            vec![("dst", "x0"), ("src", "x1"), ("imm", "16")],
            vec!["add    x0, x1, #0x10"],
        ),
        (
            "add_imm",
            vec![("dst", "x0"), ("src", "x1"), ("imm", "73728")],
            vec!["add    x0, x1, #0x12, lsl #12"],
        ),
        (
            "sub_imm",
            vec![("dst", "x0"), ("src", "x1"), ("imm", "16")],
            vec!["sub    x0, x1, #0x10"],
        ),
        ("sub_sp", vec![("imm", "32")], vec!["sub    sp, sp, #0x20"]),
        ("add_sp", vec![("imm", "32")], vec!["add    sp, sp, #0x20"]),
        (
            "cmp_imm",
            vec![("lhs", "x1"), ("rhs", "7")],
            vec!["cmp    x1, #0x7"],
        ),
        (
            "cmp_imm",
            vec![("lhs", "sp"), ("rhs", "16")],
            vec!["cmp    sp, #0x10"],
        ),
        (
            "cmp_imm",
            vec![("lhs", "x1"), ("rhs", "70000")],
            vec![
                "mov    x17, #0x1170",
                "movk   x17, #0x1, lsl #16",
                "cmp    x1, x17",
            ],
        ),
        (
            "cmp",
            vec![("lhs", "x1"), ("rhs", "x2")],
            vec!["cmp    x1, x2"],
        ),
        ("blr", vec![("register", "x9")], vec!["blr    x9"]),
        ("svc", vec![], vec!["svc    #0"]),
        ("branch_self", vec![], vec!["b      0x100001000"]),
        ("ret", vec![], vec!["ret"]),
        (
            "ldr_u64",
            load("x0", "x1", "16"),
            vec!["ldr    x0, [x1, #0x10]"],
        ),
        ("ldr_u64", load("x0", "sp", "0"), vec!["ldr    x0, [sp]"]),
        (
            "ldr_u64",
            load("x0", "x1", "40000"),
            vec![
                "add    x17, x1, #0x9, lsl #12",
                "add    x17, x17, #0xc40",
                "ldr    x0, [x17]",
            ],
        ),
        (
            "ldr_u32",
            load("x0", "x1", "4"),
            vec!["ldr    w0, [x1, #0x4]"],
        ),
        (
            "ldr_u16",
            load("x0", "x1", "2"),
            vec!["ldrh   w0, [x1, #0x2]"],
        ),
        (
            "ldr_u8",
            load("x0", "x1", "1"),
            vec!["ldrb   w0, [x1, #0x1]"],
        ),
        (
            "str_u64",
            store("x0", "sp", "24"),
            vec!["str    x0, [sp, #0x18]"],
        ),
        (
            "str_u32",
            store("x0", "x1", "4"),
            vec!["str    w0, [x1, #0x4]"],
        ),
        (
            "str_u16",
            store("x0", "x1", "2"),
            vec!["strh   w0, [x1, #0x2]"],
        ),
        (
            "str_u8",
            store("x0", "x1", "1"),
            vec!["strb   w0, [x1, #0x1]"],
        ),
        (
            "stlr_u64",
            vec![("src", "x0"), ("base", "x1")],
            vec!["stlr   x0, [x1]"],
        ),
        (
            "ldar_u64",
            vec![("dst", "x0"), ("base", "x1")],
            vec!["ldar   x0, [x1]"],
        ),
        (
            "ldar_u32",
            vec![("dst", "x0"), ("base", "x1")],
            vec!["ldar   w0, [x1]"],
        ),
        (
            "ldr_d",
            load("d0", "x1", "8"),
            vec!["ldr    d0, [x1, #0x8]"],
        ),
        (
            "str_d",
            store("d0", "x1", "8"),
            vec!["str    d0, [x1, #0x8]"],
        ),
        (
            "adrp",
            vec![("dst", "x3"), ("symbol", "_mfb_data")],
            vec!["adrp   x3, 0"],
        ),
        (
            "add_pageoff",
            vec![("dst", "x3"), ("symbol", "_mfb_data")],
            vec!["add    x3, x3, #0x0"],
        ),
        ("fmov_x_from_d", two("x0", "d1"), vec!["fmov   x0, d1"]),
        ("fmov_d_from_x", two("d0", "x1"), vec!["fmov   d0, x1"]),
        ("fmov_d_from_d", two("d0", "d1"), vec!["fmov   d0, d1"]),
        ("fadd_d", three("d0", "d1", "d2"), vec!["fadd   d0, d1, d2"]),
        ("fsub_d", three("d0", "d1", "d2"), vec!["fsub   d0, d1, d2"]),
        ("fmul_d", three("d0", "d1", "d2"), vec!["fmul   d0, d1, d2"]),
        ("fdiv_d", three("d0", "d1", "d2"), vec!["fdiv   d0, d1, d2"]),
        (
            "fminnm_d",
            three("d0", "d1", "d2"),
            vec!["fminnm d0, d1, d2"],
        ),
        (
            "fmaxnm_d",
            three("d0", "d1", "d2"),
            vec!["fmaxnm d0, d1, d2"],
        ),
        ("fneg_d", two("d0", "d1"), vec!["fneg   d0, d1"]),
        ("fabs_d", two("d0", "d1"), vec!["fabs   d0, d1"]),
        ("fsqrt_d", two("d0", "d1"), vec!["fsqrt  d0, d1"]),
        (
            "fcmp_d",
            vec![("lhs", "d0"), ("rhs", "d1")],
            vec!["fcmp   d0, d1"],
        ),
        ("fcmp_zero_d", vec![("src", "d3")], vec!["fcmp   d3, #0.0"]),
        ("scvtf_d_from_x", two("d0", "x1"), vec!["scvtf  d0, x1"]),
        ("fcvt_s_from_d", two("d0", "d1"), vec!["fcvt   s0, d1"]),
        ("fcvt_d_from_s", two("d0", "d1"), vec!["fcvt   d0, s1"]),
        ("fcvtzs_x_from_d", two("x0", "d1"), vec!["fcvtzs x0, d1"]),
        ("fcvtms_x_from_d", two("x0", "d1"), vec!["fcvtms x0, d1"]),
        ("fcvtps_x_from_d", two("x0", "d1"), vec!["fcvtps x0, d1"]),
        ("fcvtas_x_from_d", two("x0", "d1"), vec!["fcvtas x0, d1"]),
        (
            "ldr_q",
            load("q0", "x1", "32"),
            vec!["ldr    q0, [x1, #0x20]"],
        ),
        (
            "str_q",
            store("q0", "x1", "32"),
            vec!["str    q0, [x1, #0x20]"],
        ),
        (
            "fadd_v",
            three("v0", "v1", "v2"),
            vec!["fadd.2d v0, v1, v2"],
        ),
        (
            "fsub_v",
            three("v0", "v1", "v2"),
            vec!["fsub.2d v0, v1, v2"],
        ),
        (
            "fmul_v",
            three("v0", "v1", "v2"),
            vec!["fmul.2d v0, v1, v2"],
        ),
        (
            "fdiv_v",
            three("v0", "v1", "v2"),
            vec!["fdiv.2d v0, v1, v2"],
        ),
        (
            "fmla_v",
            three("v0", "v1", "v2"),
            vec!["fmla.2d v0, v1, v2"],
        ),
        (
            "fmls_v",
            three("v0", "v1", "v2"),
            vec!["fmls.2d v0, v1, v2"],
        ),
        (
            "fmin_v",
            three("v0", "v1", "v2"),
            vec!["fmin.2d v0, v1, v2"],
        ),
        (
            "fmax_v",
            three("v0", "v1", "v2"),
            vec!["fmax.2d v0, v1, v2"],
        ),
        (
            "fcmgt_v",
            three("v0", "v1", "v2"),
            vec!["fcmgt.2d v0, v1, v2"],
        ),
        (
            "fcmge_v",
            three("v0", "v1", "v2"),
            vec!["fcmge.2d v0, v1, v2"],
        ),
        (
            "fcmeq_v",
            three("v0", "v1", "v2"),
            vec!["fcmeq.2d v0, v1, v2"],
        ),
        ("add_v", three("v0", "v1", "v2"), vec!["add.2d v0, v1, v2"]),
        ("sub_v", three("v0", "v1", "v2"), vec!["sub.2d v0, v1, v2"]),
        (
            "cmgt_v",
            three("v0", "v1", "v2"),
            vec!["cmgt.2d v0, v1, v2"],
        ),
        (
            "cmge_v",
            three("v0", "v1", "v2"),
            vec!["cmge.2d v0, v1, v2"],
        ),
        (
            "cmeq_v",
            three("v0", "v1", "v2"),
            vec!["cmeq.2d v0, v1, v2"],
        ),
        (
            "sshl_v",
            three("v0", "v1", "v2"),
            vec!["sshl.2d v0, v1, v2"],
        ),
        (
            "ushl_v",
            three("v0", "v1", "v2"),
            vec!["ushl.2d v0, v1, v2"],
        ),
        ("and_v", three("v0", "v1", "v2"), vec!["and.16b v0, v1, v2"]),
        ("orr_v", three("v0", "v1", "v2"), vec!["orr.16b v0, v1, v2"]),
        ("orr_v", three("v0", "v1", "v1"), vec!["mov.16b v0, v1"]),
        ("eor_v", three("v0", "v1", "v2"), vec!["eor.16b v0, v1, v2"]),
        ("bsl_v", three("v0", "v1", "v2"), vec!["bsl.16b v0, v1, v2"]),
        ("bit_v", three("v0", "v1", "v2"), vec!["bit.16b v0, v1, v2"]),
        ("fabs_v", two("v0", "v1"), vec!["fabs.2d v0, v1"]),
        ("fneg_v", two("v0", "v1"), vec!["fneg.2d v0, v1"]),
        ("fsqrt_v", two("v0", "v1"), vec!["fsqrt.2d v0, v1"]),
        ("frintp_v", two("v0", "v1"), vec!["frintp.2d v0, v1"]),
        ("frintm_v", two("v0", "v1"), vec!["frintm.2d v0, v1"]),
        ("frinta_v", two("v0", "v1"), vec!["frinta.2d v0, v1"]),
        ("frintn_v", two("v0", "v1"), vec!["frintn.2d v0, v1"]),
        ("frintz_v", two("v0", "v1"), vec!["frintz.2d v0, v1"]),
        ("fcvtzs_v", two("v0", "v1"), vec!["fcvtzs.2d v0, v1"]),
        ("fcvtas_v", two("v0", "v1"), vec!["fcvtas.2d v0, v1"]),
        ("scvtf_v", two("v0", "v1"), vec!["scvtf.2d v0, v1"]),
        ("neg_v", two("v0", "v1"), vec!["neg.2d v0, v1"]),
        ("abs_v", two("v0", "v1"), vec!["abs.2d v0, v1"]),
        (
            "fcmgt_zero_v",
            two("v0", "v1"),
            vec!["fcmgt.2d v0, v1, #0.0"],
        ),
        (
            "fcmge_zero_v",
            two("v0", "v1"),
            vec!["fcmge.2d v0, v1, #0.0"],
        ),
        (
            "fcmeq_zero_v",
            two("v0", "v1"),
            vec!["fcmeq.2d v0, v1, #0.0"],
        ),
        (
            "fcmlt_zero_v",
            two("v0", "v1"),
            vec!["fcmlt.2d v0, v1, #0.0"],
        ),
        (
            "fcmle_zero_v",
            two("v0", "v1"),
            vec!["fcmle.2d v0, v1, #0.0"],
        ),
        ("cnt8b_v", two("v0", "v1"), vec!["cnt.8b v0, v1"]),
        ("addv8b_v", two("v0", "v1"), vec!["addv.8b b0, v1"]),
        (
            "shl_v",
            vec![("dst", "v0"), ("src", "v1"), ("shift", "3")],
            vec!["shl.2d v0, v1, #0x3"],
        ),
        (
            "sshr_v",
            vec![("dst", "v0"), ("src", "v1"), ("shift", "63")],
            vec!["sshr.2d v0, v1, #0x3f"],
        ),
        (
            "ushr_v",
            vec![("dst", "v0"), ("src", "v1"), ("shift", "1")],
            vec!["ushr.2d v0, v1, #0x1"],
        ),
        ("dup_v_from_x", two("v0", "x1"), vec!["dup.2d v0, x1"]),
        (
            "umov_x_from_v",
            vec![("dst", "x0"), ("src", "v1"), ("index", "1")],
            vec!["mov.d  x0, v1[1]"],
        ),
        (
            "fmadd_d",
            vec![
                ("dst", "d0"),
                ("addend", "d3"),
                ("lhs", "d1"),
                ("rhs", "d2"),
            ],
            vec!["fmadd  d0, d1, d2, d3"],
        ),
        (
            "fmsub_d",
            vec![
                ("dst", "d0"),
                ("addend", "d3"),
                ("lhs", "d1"),
                ("rhs", "d2"),
            ],
            vec!["fnmsub d0, d1, d2, d3"],
        ),
        (
            "fnmsub_d",
            vec![
                ("dst", "d0"),
                ("addend", "d3"),
                ("lhs", "d1"),
                ("rhs", "d2"),
            ],
            vec!["fmsub  d0, d1, d2, d3"],
        ),
        (
            "fnmadd_d",
            vec![
                ("dst", "d0"),
                ("addend", "d3"),
                ("lhs", "d1"),
                ("rhs", "d2"),
            ],
            vec!["fnmadd d0, d1, d2, d3"],
        ),
    ]
}

#[test]
fn every_encoded_form_decodes_to_its_text() {
    for (mnemonic, fields, expected) in table() {
        assert_eq!(
            round_trip(&op(mnemonic, &fields)),
            expected,
            "{mnemonic} {fields:?}"
        );
    }
}

/// Branches and calls are patched by the encoder's label pass or the linker,
/// so they are decoded from their final words.
#[test]
fn branches_decode_their_targets() {
    let decoded = |word: u32| decode(word, PC).unwrap();
    let b = decoded(0x1400_0002);
    assert_eq!(
        (b.text.as_str(), b.flow, b.target),
        ("b      0x100001008", Flow::Branch, Some(PC + 8))
    );
    let back = decoded(0x17ff_fffe);
    assert_eq!(back.target, Some(PC - 8));
    let bl = decoded(0x9400_0010);
    assert_eq!(
        (bl.text.as_str(), bl.flow),
        ("bl     0x100001040", Flow::Call)
    );
    for (code, name) in [
        (0x0, "eq"),
        (0x1, "ne"),
        (0xa, "ge"),
        (0xb, "lt"),
        (0xc, "gt"),
        (0xd, "le"),
        (0x7, "vc"),
        (0x6, "vs"),
        (0x8, "hi"),
        (0x3, "lo"),
        (0x4, "mi"),
        (0x9, "ls"),
    ] {
        let branch = decoded(0x5400_0040 | code);
        assert_eq!(branch.text, format!("b.{name:<4} 0x100001008"), "{code:#x}");
        assert_eq!((branch.flow, branch.target), (Flow::Branch, Some(PC + 8)));
    }
    assert_eq!(decoded(0x54ff_ffeb).target, Some(PC - 4));
    let ret = decoded(0xd65f_03c0);
    assert_eq!(ret.flow, Flow::Return);
}

#[test]
fn an_adrp_and_its_add_carry_the_address_they_form() {
    // adrp x3, 2 (pages); add x3, x3, #0x440.
    let adrp = decode(0x9000_0003 | (2 << 29), PC + 0x10).unwrap();
    assert_eq!(adrp.text, "adrp   x3, 2");
    assert_eq!(adrp.page, Some((3, 0x1_0000_3000)));
    let add = decode(0x9100_0000 | (0x440 << 10) | (3 << 5) | 3, PC + 0x14).unwrap();
    assert_eq!(add.page_offset, Some((3, 0x440)));
    let negative = decode(0x9000_0003 | (0x7ffff << 5) | (3 << 29), PC).unwrap();
    assert_eq!(negative.text, "adrp   x3, -1");
    assert_eq!(negative.page, Some((3, PC - 0x1000)));
    let wide = decode(0xd280_0000 | (0xde83 << 5) | 8, PC).unwrap();
    assert_eq!(wide.value, Some(0xde83));
}

#[test]
fn a_word_the_encoder_never_emits_does_not_decode() {
    // `pacibsp`, `stp x29, x30, [sp, #-16]!`, `cbz w0, …`, `br x16`.
    for word in [0xd503_237f, 0xa9bf_7bfd, 0x3400_0048, 0xd61f_0200] {
        assert_eq!(decode(word, PC), None, "{word:#010x}");
    }
}

/// Every mnemonic `emit_instruction` dispatches on appears in the table (or,
/// for the label branches and `bl`, in `branches_decode_their_targets`).
#[test]
fn the_table_covers_every_form_the_encoder_emits() {
    let source = include_str!("../encode/emitter.rs");
    let body = &source[source.find("match instruction.op.mnemonic()").unwrap()..];
    let body = &body[..body.find("other => Err(").unwrap()];
    let mut emitted = Vec::new();
    for line in body.lines() {
        let line = line.trim_start().trim_start_matches('|').trim_start();
        let mut rest = line;
        while let Some(start) = rest.strip_prefix('"') {
            let Some(end) = start.find('"') else { break };
            emitted.push(start[..end].to_string());
            rest = start[end + 1..]
                .trim_start()
                .trim_start_matches('|')
                .trim_start();
        }
    }
    let tested = table()
        .into_iter()
        .map(|(mnemonic, _, _)| mnemonic.to_string())
        .collect::<Vec<_>>();
    let by_word = [
        "label", "b.eq", "b.ne", "b.ge", "b.lt", "b.gt", "b.le", "b.vc", "b.vs", "b.hi", "b.lo",
        "b.mi", "b.ls", "b", "bl",
    ];
    let missing = emitted
        .iter()
        .filter(|mnemonic| !tested.contains(mnemonic) && !by_word.contains(&mnemonic.as_str()))
        .collect::<Vec<_>>();
    assert!(
        emitted.len() > 100,
        "the scan found only {} mnemonics",
        emitted.len()
    );
    assert!(missing.is_empty(), "not decoded by any test: {missing:?}");
}
