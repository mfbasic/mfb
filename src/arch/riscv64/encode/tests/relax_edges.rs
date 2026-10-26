//! rv64 branch relaxation beyond the forward case the pass was written against.

use super::*;
use crate::codegen::engine::types::CodeInstruction;

fn plan_of(instructions: Vec<CodeInstruction>) -> NativeCodePlan {
    NativeCodePlan {
        target: "linux-riscv64".to_string(),
        build_mode: NativeBuildMode::Console,
        arch: "riscv64".to_string(),
        project: "t".to_string(),
        entry_symbol: Some("main".to_string()),
        imports: Vec::new(),
        data_objects: Vec::new(),
        functions: vec![CodeFunction {
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
        }],
    }
}

/// A jump back past the `jal` reach: `far:`, then more than 1 MiB of one-word
/// instructions, then `b far`. The rungs run forward from the target here, the
/// mirror of the forward case, and the whole plan must then encode.
#[test]
fn a_backward_jump_past_the_jal_reach_is_relaxed_into_hops_and_encodes() {
    let words = (1usize << 20) / 4 + 64;
    let mut instructions = vec![CodeInstruction::new("label").field("name", "far")];
    instructions.extend((0..words).map(|_| CodeInstruction::new("ret")));
    instructions.push(CodeInstruction::new("b").field("target", "far"));
    instructions.push(CodeInstruction::new("ret"));
    let mut plan = plan_of(instructions);
    assert!(
        encode(&plan)
            .err()
            .is_some_and(|err| err.contains("exceeds \u{00b1}1 MiB")),
        "control: unrelaxed, the jump is out of reach"
    );
    relax_rv64_branches(&mut plan).expect("relaxation");
    assert!(
        plan.functions[0]
            .instructions
            .iter()
            .any(|instruction| instruction
                .get("name")
                .is_some_and(|name| name.starts_with("__mfb_rv_hop_"))),
        "the pass must have inserted hop labels"
    );
    encode(&plan).expect("the relaxed plan encodes");
}

/// A malformed plan is refused by name rather than relaxed around: a `label`
/// with no name cannot place a target, and a jump with no target is refused by
/// the offset walk, which sizes every instruction by encoding it — the
/// invariant the far-jump scan and `retarget` rely on.
#[test]
fn a_plan_with_a_nameless_label_or_a_targetless_jump_is_refused() {
    let refusal = |instructions: Vec<CodeInstruction>| {
        relax_rv64_branches(&mut plan_of(instructions)).expect_err("a malformed plan is refused")
    };
    assert_eq!(
        refusal(vec![
            CodeInstruction::new("label"),
            CodeInstruction::new("ret")
        ]),
        "rv64 relax: label without a name"
    );
    assert_eq!(
        refusal(vec![CodeInstruction::new("b"), CodeInstruction::new("ret")]),
        "instruction 'b' missing field 'target'"
    );
    assert_eq!(
        refusal(vec![
            CodeInstruction::new("rv.br")
                .field("cond", "eq")
                .physical_field("lhs", "a0")
                .physical_field("rhs", "a1"),
            CodeInstruction::new("ret"),
        ]),
        "instruction 'rv.br' missing field 'target'"
    );
}

/// Decode a `jal`'s signed byte displacement out of its J-type immediate.
fn jal_displacement(word: u32) -> i64 {
    assert_eq!(word & 0x7f, 0x6f, "not a jal: {word:#010x}");
    let imm20 = ((word >> 31) & 1) as i64;
    let imm10_1 = ((word >> 21) & 0x3ff) as i64;
    let imm11 = ((word >> 20) & 1) as i64;
    let imm19_12 = ((word >> 12) & 0xff) as i64;
    let value = (imm20 << 20) | (imm19_12 << 12) | (imm11 << 11) | (imm10_1 << 1);
    (value << 43) >> 43
}

/// The outermost rung of a backward ladder can be wanted exactly where the
/// function ends: an `rv.br` as the last instruction whose `jal` sits three hops
/// (3 x 512 KiB) after its target. That rung is appended after the last
/// instruction, and the chain from the `jal` still walks, hop by in-range hop,
/// back to `far:` at offset 0.
#[test]
fn a_rung_wanted_past_the_last_instruction_lands_at_the_function_end() {
    const HOP: usize = 1 << 19;
    // `far:` at 0, then `rets` words, then the 8-byte `rv.br`: its `jal` is at
    // 4 * rets + 4 = 3 * HOP.
    let rets = (3 * HOP - 4) / 4;
    let mut instructions = vec![CodeInstruction::new("label").field("name", "far")];
    instructions.extend((0..rets).map(|_| CodeInstruction::new("ret")));
    instructions.push(
        CodeInstruction::new("rv.br")
            .field("cond", "eq")
            .physical_field("lhs", "a0")
            .physical_field("rhs", "a1")
            .field("target", "far"),
    );
    let mut plan = plan_of(instructions);
    relax_rv64_branches(&mut plan).expect("relaxation");
    let relaxed = &plan.functions[0].instructions;
    // The function now ends in an island: `b over`, `hop:`, `b <inner rung>`, `over:`.
    let tail: Vec<String> = relaxed[relaxed.len() - 4..]
        .iter()
        .map(|instruction| instruction.op.mnemonic().to_string())
        .collect();
    assert_eq!(tail, ["b", "label", "b", "label"], "island at the end");
    assert!(relaxed[relaxed.len() - 3]
        .get("name")
        .is_some_and(|name| name.starts_with("__mfb_rv_hop_")));

    let text = encode(&plan).expect("the relaxed plan encodes").text;
    let words: Vec<u32> = text
        .chunks_exact(4)
        .map(|chunk| u32::from_le_bytes(chunk.try_into().unwrap()))
        .collect();
    // The `rv.br` is followed only by the final island's two words.
    let mut at = words.len() - 3;
    let mut hops = 0;
    while at != 0 {
        at = (at as i64 + jal_displacement(words[at]) / 4) as usize;
        hops += 1;
        assert!(hops <= 4, "the chain must reach far: within the ladder");
    }
    assert_eq!(
        hops, 4,
        "jal -> rung 3 (at the end) -> rung 2 -> rung 1 -> far"
    );
}

/// plan-171-B: a `tbz` 1.5 MiB from its target reaches it through hop islands
/// (its `jal` sits 8 bytes in, after the `andi`/`slli` and the short branch),
/// and the relaxed plan encodes with the `tbz` itself unchanged in size.
#[test]
fn a_far_tbz_is_relaxed_into_hops_and_encodes() {
    let words = (3usize << 19) / 4;
    let mut instructions = vec![CodeInstruction::new("tbz")
        .physical_field("register", "t3")
        .field("bit", "5")
        .physical_field("scratch", "t4")
        .field("target", "far")];
    instructions.extend((0..words).map(|_| CodeInstruction::new("ret")));
    instructions.push(CodeInstruction::new("label").field("name", "far"));
    instructions.push(CodeInstruction::new("ret"));
    let mut plan = plan_of(instructions);
    assert!(
        encode(&plan)
            .err()
            .is_some_and(|err| err.contains("exceeds \u{00b1}1 MiB")),
        "control: unrelaxed, the tbz's jal is out of reach"
    );
    relax_rv64_branches(&mut plan).expect("relaxation");
    let first = &plan.functions[0].instructions[0];
    assert_eq!(first.op.mnemonic(), "tbz");
    assert!(first
        .get("target")
        .is_some_and(|target| target.starts_with("__mfb_rv_hop_")));
    assert_eq!(first.get("bit").as_deref(), Some("5"));
    encode(&plan).expect("the relaxed plan encodes");
}
