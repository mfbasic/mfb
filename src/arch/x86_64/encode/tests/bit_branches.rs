//! plan-171-B: x86-64 `tbz`/`tbnz` (`test r, imm32` + `je`/`jne` for bits
//! 0–30, `bt r, imm8` + `jae`/`jb` for 31–63) and the refused AArch64-only
//! `cbz`/`cbnz`. The test/bt bytes are clang's (`clang -c -target
//! x86_64-linux-gnu`, Intel syntax, read back with `objdump -d`); `test rax`
//! is the general `F7 /0` form, which clang shortens to `A9`.

use super::*;

fn branch(op: &str, register: &str, bit: &str, target: &str) -> CodeInstruction {
    CodeInstruction::new(op)
        .physical_field("register", register)
        .field("bit", bit)
        .field("target", target)
}

fn text_of(instructions: Vec<CodeInstruction>) -> Vec<u8> {
    let plan = NativeCodePlan {
        target: "linux-x86_64".to_string(),
        build_mode: crate::target::NativeBuildMode::Console,
        arch: "x86_64".to_string(),
        project: "t".to_string(),
        entry_symbol: Some("_mfb_fn_f".to_string()),
        imports: Vec::new(),
        data_objects: Vec::new(),
        functions: vec![CodeFunction {
            name: "f".to_string(),
            symbol: "_mfb_fn_f".to_string(),
            params: Vec::new(),
            returns: "Nothing".to_string(),
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

#[test]
fn encodes_the_bit_tests_at_their_boundaries() {
    let text = text_of(vec![
        branch("tbz", "r10", "0", "l1"),
        branch("tbnz", "r11", "10", "l1"),
        branch("tbz", "rax", "11", "l1"),
        branch("tbnz", "r14", "30", "l1"),
        branch("tbz", "r12", "31", "l1"),
        branch("tbnz", "rdi", "63", "l1"),
        CodeInstruction::new("label").field("name", "l1"),
        CodeInstruction::new("ret"),
    ]);
    // 13 bytes per test+jcc, 11 per bt+jcc: `l1` is at 74, each rel32 counts
    // from the end of its own jump.
    let expected: Vec<u8> = [
        &[0x49, 0xF7, 0xC2, 0x01, 0, 0, 0, 0x0F, 0x84, 61, 0, 0, 0][..],
        &[0x49, 0xF7, 0xC3, 0x00, 0x04, 0, 0, 0x0F, 0x85, 48, 0, 0, 0],
        &[0x48, 0xF7, 0xC0, 0x00, 0x08, 0, 0, 0x0F, 0x84, 35, 0, 0, 0],
        &[0x49, 0xF7, 0xC6, 0, 0, 0, 0x40, 0x0F, 0x85, 22, 0, 0, 0],
        &[0x49, 0x0F, 0xBA, 0xE4, 0x1F, 0x0F, 0x83, 11, 0, 0, 0],
        &[0x48, 0x0F, 0xBA, 0xE7, 0x3F, 0x0F, 0x82, 0, 0, 0, 0],
        &[0xC3],
    ]
    .concat();
    assert_eq!(text, expected);
}

#[test]
fn a_bit_test_is_sized_by_its_encoding() {
    let low = branch("tbz", "r10", "30", "l");
    let high = branch("tbnz", "r10", "31", "l");
    assert_eq!(instruction_size(&low).unwrap(), 13);
    assert_eq!(instruction_size(&high).unwrap(), 11);
}

#[test]
fn refuses_cbz_and_a_bad_bit_test() {
    for op in ["cbz", "cbnz"] {
        let ins = CodeInstruction::new(op)
            .physical_field("register", "r10")
            .field("target", "l");
        let err = enc_err(&ins);
        assert!(err.contains("AArch64-only"), "{op}: {err}");
    }
    assert!(enc_err(&branch("tbz", "r10", "64", "l")).contains("outside 0..=63"));
    assert!(enc_err(&branch("tbz", "rsp", "1", "l")).contains("general-purpose"));
}
