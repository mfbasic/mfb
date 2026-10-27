//! plan-171-B: `cbz`/`cbnz`/`tbz`/`tbnz`. Oracle words: Apple clang 17
//! `clang -c -target arm64-apple-macos` of the spelled lines, read back with
//! `objdump -d`.

use super::*;
use crate::codegen::engine::types::{CodeFrame, CodeFunction};

fn branch(op: &str, register: &str, bit: Option<&str>, target: &str) -> CodeInstruction {
    let instruction = CodeInstruction::new(op).physical_field("register", register);
    let instruction = match bit {
        Some(bit) => instruction.field("bit", bit),
        None => instruction,
    };
    instruction.field("target", target)
}

fn function(instructions: Vec<CodeInstruction>) -> CodeFunction {
    CodeFunction {
        name: "main".to_string(),
        symbol: "main".to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        instructions,
        relocations: Vec::new(),
        stack_slots: Vec::new(),
    }
}

fn text_words(instructions: Vec<CodeInstruction>) -> Result<Vec<u32>, String> {
    let image = super::super::encode(&plan(
        vec![function(instructions)],
        vec![],
        vec![],
        Some("main"),
    ))?;
    Ok(image
        .text
        .chunks_exact(4)
        .map(|word| u32::from_le_bytes(word.try_into().unwrap()))
        .collect())
}

/// Every form, the bit boundaries 0/10/11/30/31/32/63 (`b5` flips at 32), low,
/// scratch-range and link registers, and a backward target.
#[test]
fn encodes_every_zero_and_bit_branch_form() {
    let words = text_words(vec![
        CodeInstruction::new("label").field("name", "start"),
        branch("cbz", "x0", None, "l1"),
        branch("cbnz", "x9", None, "l1"),
        branch("cbz", "x30", None, "l1"),
        branch("tbz", "x0", Some("0"), "l1"),
        branch("tbnz", "x15", Some("10"), "l1"),
        branch("tbz", "x21", Some("11"), "l1"),
        branch("tbnz", "x30", Some("30"), "l1"),
        branch("tbz", "x3", Some("31"), "l1"),
        branch("tbnz", "x27", Some("32"), "l1"),
        branch("tbz", "x8", Some("63"), "l1"),
        branch("tbnz", "x1", Some("63"), "start"),
        CodeInstruction::new("label").field("name", "l1"),
        CodeInstruction::new("ret"),
    ])
    .expect("encodes");
    assert_eq!(
        words,
        vec![
            0xb400_0160,
            0xb500_0149,
            0xb400_013e,
            0x3600_0100,
            0x3750_00ef,
            0x3658_00d5,
            0x37f0_00be,
            0x36f8_0083,
            0xb700_007b,
            0xb6f8_0048,
            0xb7ff_fec1,
            0xd65f_03c0,
        ]
    );
}

/// Each is one word, sized by the same emitter.
#[test]
fn a_zero_or_bit_branch_is_one_word() {
    for instruction in [
        branch("cbz", "x4", None, "l"),
        branch("tbnz", "x4", Some("40"), "l"),
    ] {
        assert_eq!(instruction_size(&instruction).unwrap(), 4);
    }
}

#[test]
fn refuses_a_bad_register_or_bit() {
    for (instruction, wanted) in [
        (branch("cbz", "sp", None, "l"), "general-purpose register"),
        (
            branch("tbz", "d3", Some("1"), "l"),
            "general-purpose register",
        ),
        (branch("tbnz", "x3", Some("64"), "l"), "outside 0..=63"),
    ] {
        let err = fresh_encoder()
            .emit_instruction(&instruction)
            .expect_err("refused");
        assert!(err.contains(wanted), "{err}");
    }
}

/// `tbz` reaches ±32 KiB (imm14 words); one word further is refused, not masked.
#[test]
fn a_tbz_past_32_kib_is_refused_and_one_within_encodes() {
    let padded = |words: usize| {
        let mut instructions = vec![branch("tbz", "x2", Some("5"), "far")];
        instructions.extend((0..words).map(|_| CodeInstruction::new("ret")));
        instructions.push(CodeInstruction::new("label").field("name", "far"));
        instructions.push(CodeInstruction::new("ret"));
        instructions
    };
    // Displacement (1 + words) * 4 bytes: 8191 words is the last in reach.
    let words = text_words(padded(8190)).expect("in reach");
    assert_eq!(words[0], 0x3628_0000 | (0x1fff << 5) | 2);
    let err = text_words(padded(8191)).expect_err("out of reach");
    assert!(err.contains("±32 KiB"), "{err}");
}

/// plan-171-B: a `tbz` 40 KiB from its target and a `cbz` 1.5 MiB from its
/// target are relaxed through bug-445's trampoline — the near hop keeps the
/// register and bit — and the plans encode.
#[test]
fn far_zero_and_bit_branches_relax_through_a_trampoline() {
    for (branch_op, bit, bytes) in [("tbz", Some("37"), 40 << 10), ("cbz", None, 3 << 19)] {
        let mut instructions = vec![branch(branch_op, "x5", bit, "far")];
        instructions.extend((0..bytes / 4).map(|_| CodeInstruction::new("ret")));
        instructions.push(CodeInstruction::new("label").field("name", "far"));
        instructions.push(CodeInstruction::new("ret"));
        let mut relaxed = plan(vec![function(instructions)], vec![], vec![], Some("main"));
        let err = super::super::encode(&relaxed)
            .err()
            .expect("out of reach unrelaxed");
        assert!(err.contains("exceeds"), "{err}");
        super::super::relax_conditional_branches(&mut relaxed).expect("relaxation");
        let hop = &relaxed.functions[0].instructions[0];
        assert_eq!(hop.op.mnemonic(), branch_op);
        assert_eq!(hop.get("register").as_deref(), Some("x5"));
        assert_eq!(hop.get("bit").as_deref(), bit);
        let image = super::super::encode(&relaxed).expect("the relaxed plan encodes");
        let first = u32::from_le_bytes(image.text[..4].try_into().unwrap());
        // The hop jumps +8 to the trampoline (two words: imm 2 at bit 5).
        let expected = if branch_op == "tbz" {
            0xb628_0000 | (2 << 5) | 5
        } else {
            0xb400_0000 | (2 << 5) | 5
        };
        assert_eq!(first, expected, "{branch_op}");
    }
}
