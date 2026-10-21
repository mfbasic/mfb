//! The decoder against the encoder: every operation `emit_instruction` accepts,
//! encoded, decodes word for word.

use super::decode;
use crate::arch::riscv64::encode::encode;
use crate::codegen::engine::types::{
    CodeDataObject, CodeFrame, CodeFunction, CodeInstruction, NativeCodePlan,
};
use crate::target::NativeBuildMode;

/// One operation with its fields.
fn op(name: &str, fields: &[(&'static str, &str)]) -> CodeInstruction {
    fields
        .iter()
        .fold(CodeInstruction::new(name), |instruction, (key, value)| {
            instruction.physical_field(key, value)
        })
}

/// The code `instructions` encode to, in a function `f` that starts with the
/// label `top` (a branch's target) beside a data object `d`.
fn text(instructions: Vec<CodeInstruction>) -> Vec<u8> {
    let mut body = vec![op("label", &[("name", "top")])];
    body.extend(instructions);
    let plan = NativeCodePlan {
        target: "linux-riscv64".to_string(),
        build_mode: NativeBuildMode::Console,
        arch: "riscv64".to_string(),
        project: "t".to_string(),
        entry_symbol: Some("f".to_string()),
        imports: Vec::new(),
        data_objects: vec![CodeDataObject {
            symbol: "d".to_string(),
            kind: "raw".to_string(),
            layout: String::new(),
            align: 8,
            size: 8,
            value: "0000000000000000".to_string(),
        }],
        functions: vec![CodeFunction {
            name: "f".to_string(),
            symbol: "f".to_string(),
            params: Vec::new(),
            returns: "Integer".to_string(),
            frame: CodeFrame {
                stack_size: 0,
                callee_saved: Vec::new(),
            },
            instructions: body,
            relocations: Vec::new(),
            stack_slots: Vec::new(),
        }],
    };
    encode(&plan).expect("the encoder takes it").text
}

/// Every operation `emit_instruction` accepts (and every vector operation
/// `emit_rv_vop` does), named, each with operands that reach its paths: both
/// sizes of an immediate, each branch condition and compare kind.
fn cases() -> Vec<(String, CodeInstruction)> {
    let mut cases = Vec::new();
    let mut add = |name: String, instruction: CodeInstruction| cases.push((name, instruction));
    let dst_src =
        |name: &str, dst: &'static str, src: &'static str| op(name, &[("dst", dst), ("src", src)]);
    let three = [("dst", "a0"), ("lhs", "a1"), ("rhs", "a2")];
    let three_f = [("dst", "fa0"), ("lhs", "fa1"), ("rhs", "fa2")];
    for value in [
        "5",
        "18446744073709551609",
        "305419896",
        "81985529216486895",
    ] {
        add(
            format!("mov_imm {value}"),
            op("mov_imm", &[("dst", "a0"), ("value", value)]),
        );
    }
    add("mov".into(), dst_src("mov", "a0", "a1"));
    for name in [
        "add", "sub", "and", "orr", "eor", "mul", "smulh", "umulh", "sdiv", "udiv", "rv.slt",
        "rv.sltu", "lslv", "lsrv", "asrv", "rorv", "rorv_w",
    ] {
        add(name.into(), op(name, &three));
    }
    add(
        "msub".into(),
        op(
            "msub",
            &[
                ("dst", "a0"),
                ("lhs", "a1"),
                ("rhs", "a2"),
                ("minuend", "a3"),
            ],
        ),
    );
    add(
        "add_carry".into(),
        op(
            "add_carry",
            &[
                ("dst", "a0"),
                ("carry_out", "a1"),
                ("lhs", "a2"),
                ("rhs", "a3"),
                ("carry_in", "a4"),
            ],
        ),
    );
    add(
        "sub_borrow".into(),
        op(
            "sub_borrow",
            &[
                ("dst", "a0"),
                ("borrow_out", "a1"),
                ("lhs", "a2"),
                ("rhs", "a3"),
                ("borrow_in", "a4"),
            ],
        ),
    );
    for name in ["mvn", "sxtw", "clz", "rbit", "rev_x", "rev_w"] {
        add(name.into(), dst_src(name, "a0", "a1"));
    }
    for name in ["lsl_imm", "lsr_imm", "asr_imm"] {
        add(
            name.into(),
            op(name, &[("dst", "a0"), ("src", "a1"), ("shift", "3")]),
        );
    }
    for imm in ["8", "100000"] {
        for name in ["add_imm", "sub_imm"] {
            add(
                format!("{name} {imm}"),
                op(name, &[("dst", "a0"), ("src", "a1"), ("imm", imm)]),
            );
        }
        for name in ["sub_sp", "add_sp"] {
            add(format!("{name} {imm}"), op(name, &[("imm", imm)]));
        }
        for name in ["ldr_u64", "ldr_u32", "ldr_u16", "ldr_u8"] {
            add(
                format!("{name} {imm}"),
                op(name, &[("dst", "a0"), ("base", "a1"), ("offset", imm)]),
            );
        }
        for name in ["str_u64", "str_u32", "str_u16", "str_u8"] {
            add(
                format!("{name} {imm}"),
                op(name, &[("src", "a0"), ("base", "a1"), ("offset", imm)]),
            );
        }
        add(
            format!("ldr_d {imm}"),
            op("ldr_d", &[("dst", "fa0"), ("base", "a1"), ("offset", imm)]),
        );
        add(
            format!("str_d {imm}"),
            op("str_d", &[("src", "fa0"), ("base", "a1"), ("offset", imm)]),
        );
    }
    add("b".into(), op("b", &[("target", "top")]));
    add("bl".into(), op("bl", &[("target", "f")]));
    add("blr".into(), op("blr", &[("register", "a5")]));
    for name in ["branch_self", "svc", "ret"] {
        add(name.into(), op(name, &[]));
    }
    for cond in ["eq", "ne", "lt", "ge", "ltu", "geu"] {
        add(
            format!("rv.br {cond}"),
            op(
                "rv.br",
                &[
                    ("lhs", "a0"),
                    ("rhs", "a1"),
                    ("cond", cond),
                    ("target", "top"),
                ],
            ),
        );
    }
    add("adrp".into(), op("adrp", &[("dst", "a0"), ("symbol", "d")]));
    add(
        "add_pageoff".into(),
        op("add_pageoff", &[("dst", "a0"), ("symbol", "d")]),
    );
    for name in [
        "fadd_d", "fsub_d", "fmul_d", "fdiv_d", "fminnm_d", "fmaxnm_d",
    ] {
        add(name.into(), op(name, &three_f));
    }
    for name in [
        "fmov_d_from_d",
        "fneg_d",
        "fabs_d",
        "fsqrt_d",
        "fcvt_s_from_d",
        "fcvt_d_from_s",
    ] {
        add(name.into(), dst_src(name, "fa0", "fa1"));
    }
    add(
        "fmov_x_from_d".into(),
        dst_src("fmov_x_from_d", "a0", "fa0"),
    );
    add(
        "fmov_d_from_x".into(),
        dst_src("fmov_d_from_x", "fa0", "a0"),
    );
    add(
        "scvtf_d_from_x".into(),
        dst_src("scvtf_d_from_x", "fa0", "a0"),
    );
    for name in [
        "fcvtzs_x_from_d",
        "fcvtms_x_from_d",
        "fcvtps_x_from_d",
        "fcvtas_x_from_d",
    ] {
        add(name.into(), dst_src(name, "a0", "fa0"));
    }
    for cmp in ["eq", "lt", "le"] {
        add(
            format!("rv.fcmp {cmp}"),
            op(
                "rv.fcmp",
                &[("dst", "a0"), ("lhs", "fa0"), ("rhs", "fa1"), ("cmp", cmp)],
            ),
        );
    }
    for name in ["fmadd_d", "fmsub_d", "fnmsub_d", "fnmadd_d"] {
        add(
            name.into(),
            op(
                name,
                &[
                    ("dst", "fa0"),
                    ("addend", "fa1"),
                    ("lhs", "fa2"),
                    ("rhs", "fa3"),
                ],
            ),
        );
    }
    let vector: &[(&str, &[(&'static str, &str)])] = &[
        ("vsetvli", &[("dst", "a0"), ("avl", "a1")]),
        ("vsetivli", &[("dst", "zero"), ("avl", "2")]),
        ("vfsqrt.v", &[("dst", "v1"), ("src", "v2")]),
        ("vfcvt.x.f.v", &[("dst", "v1"), ("src", "v2")]),
        ("vfcvt.rtz.x.f.v", &[("dst", "v1"), ("src", "v2")]),
        ("vfcvt.f.x.v", &[("dst", "v1"), ("src", "v2")]),
        ("vmv.v.i", &[("dst", "v1"), ("imm", "31")]),
        ("vmv.v.x", &[("dst", "v1"), ("gpr", "a0")]),
        ("vmerge.vim", &[("dst", "v1"), ("src", "v2"), ("imm", "31")]),
        ("vmv.x.s", &[("dst", "a0"), ("src", "v2")]),
        ("vmv.s.x", &[("dst", "v1"), ("gpr", "a0")]),
        ("vle64.v", &[("dst", "v1"), ("base", "a0")]),
        ("vse64.v", &[("src", "v1"), ("base", "a0")]),
    ];
    for (vop, fields) in vector {
        let mut all = vec![("vop", *vop)];
        all.extend_from_slice(fields);
        add((*vop).into(), op("rv.vop", &all));
    }
    for vop in [
        "vfadd.vv",
        "vfsub.vv",
        "vfmul.vv",
        "vfdiv.vv",
        "vfmin.vv",
        "vfmax.vv",
        "vfsgnjn.vv",
        "vfsgnjx.vv",
        "vmfeq.vv",
        "vmfle.vv",
        "vmflt.vv",
        "vadd.vv",
        "vsub.vv",
        "vand.vv",
        "vor.vv",
        "vxor.vv",
        "vmseq.vv",
        "vmsle.vv",
        "vmslt.vv",
        "vsll.vv",
        "vsra.vv",
        "vsrl.vv",
        "vfmacc.vv",
        "vfnmsac.vv",
    ] {
        add(
            vop.into(),
            op(
                "rv.vop",
                &[("vop", vop), ("dst", "v1"), ("lhs", "v2"), ("rhs", "v3")],
            ),
        );
    }
    // The sign-injection aliases (`vfneg.v`, `vfabs.v`) the compiler's
    // negation and absolute value emit, with both sources the same.
    for vop in ["vfsgnjn.vv", "vfsgnjx.vv"] {
        add(
            format!("{vop} same"),
            op(
                "rv.vop",
                &[("vop", vop), ("dst", "v1"), ("lhs", "v2"), ("rhs", "v2")],
            ),
        );
    }
    for vop in ["vsll.vx", "vsra.vx", "vsrl.vx", "vrsub.vx"] {
        add(
            vop.into(),
            op(
                "rv.vop",
                &[("vop", vop), ("dst", "v1"), ("lhs", "v2"), ("gpr", "a0")],
            ),
        );
    }
    add(
        "vrsub.vx zero".into(),
        op(
            "rv.vop",
            &[
                ("vop", "vrsub.vx"),
                ("dst", "v1"),
                ("lhs", "v2"),
                ("gpr", "zero"),
            ],
        ),
    );
    for vop in ["vsll.vi", "vsra.vi", "vsrl.vi", "vslidedown.vi"] {
        add(
            vop.into(),
            op(
                "rv.vop",
                &[("vop", vop), ("dst", "v1"), ("lhs", "v2"), ("imm", "3")],
            ),
        );
    }
    cases
}

/// The operation names `emit_instruction` and `emit_rv_vop` match on, read from
/// the emitter's source: the arms of the form `"name" =>`.
fn emitter_operations() -> std::collections::BTreeSet<String> {
    let source = include_str!("../encode/emitter.rs");
    let mut names = std::collections::BTreeSet::new();
    for line in source.lines() {
        let line = line.trim_start();
        let Some(arm) = line.split("=>").next().filter(|_| line.contains("=>")) else {
            continue;
        };
        let alternatives = arm.split('|').map(str::trim).collect::<Vec<_>>();
        if alternatives.iter().all(|alternative| {
            alternative.len() > 2 && alternative.starts_with('"') && alternative.ends_with('"')
        }) {
            names.extend(
                alternatives
                    .iter()
                    .map(|alternative| alternative.trim_matches('"').to_string()),
            );
        }
    }
    names
}

/// Each word of `code`, decoded, with the operation's name for a message.
fn decoded(name: &str, code: &[u8]) -> Vec<super::Instruction> {
    let mut at = 0;
    let mut instructions = Vec::new();
    while at < code.len() {
        let pc = at as u64;
        let instruction = decode(&code[at..], pc)
            .unwrap_or_else(|| panic!("{name}: no instruction at {at:#x}: {:02x?}", &code[at..]));
        assert_eq!(
            instruction.length,
            super::length(&code[at..]),
            "{name} at {at:#x}"
        );
        at += instruction.length;
        instructions.push(instruction);
    }
    instructions
}

#[test]
fn decodes_every_form_the_encoder_emits() {
    let cases = cases();
    // Every arm of the emitter is a case: a new operation must be added here.
    let covered = cases
        .iter()
        .flat_map(|(_, instruction)| {
            let op = instruction.op.mnemonic().to_string();
            let vop = instruction.get("vop");
            let cond = ["cond", "cmp"]
                .iter()
                .filter_map(|key| instruction.get(key))
                .collect::<Vec<_>>();
            std::iter::once(op).chain(vop).chain(cond)
        })
        .chain(["label".to_string()])
        .collect::<std::collections::BTreeSet<_>>();
    let missing = emitter_operations()
        .into_iter()
        .filter(|name| !covered.contains(name))
        .collect::<Vec<_>>();
    assert!(
        missing.is_empty(),
        "the emitter's operations with no case: {missing:?}"
    );
    for (name, instruction) in cases {
        let code = text(vec![instruction, op("ret", &[])]);
        let instructions = decoded(&name, &code);
        assert_eq!(
            instructions.last().map(|last| last.text.as_str()),
            Some("ret"),
            "{name}"
        );
    }
}

/// Write every word the encoder emits for [`cases`] to the file
/// `ENCODER_WORDS` names, as raw little-endian code, for
/// `tools/oracles/riscv64-decode/objdump-corpus.sh` to disassemble.
#[test]
#[ignore = "writes a file for the objdump oracle"]
fn writes_the_encoder_s_words() {
    let path = std::env::var("ENCODER_WORDS").expect("ENCODER_WORDS names a file to write");
    let mut words = Vec::new();
    for (_, instruction) in cases() {
        words.extend(text(vec![instruction, op("ret", &[])]));
    }
    std::fs::write(&path, words).expect("a writable file");
}
