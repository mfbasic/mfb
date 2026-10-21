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
