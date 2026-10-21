//! The AArch64 encoder's refusals and pass-throughs the happy-path forms never reach.

use super::*;
use crate::codegen::engine::types::{CodeFrame, CodeFunction, NativeCodePlan};

/// A right shift of a `.2d` lane by zero has no encoding (`sshr`/`ushr` take 1..=63;
/// a zero shift is a plain move), and the encoder says which instruction and amount.
#[test]
fn a_zero_right_shift_of_a_2d_lane_is_refused_by_name() {
    for (op, name) in [("sshr_v", "sshr.2d"), ("ushr_v", "ushr.2d")] {
        let mut encoder = fresh_encoder();
        let error = encoder
            .emit_instruction(
                &CodeInstruction::new(op)
                    .physical_field("dst", "v0")
                    .physical_field("src", "v1")
                    .field("shift", "0"),
            )
            .unwrap_err();
        assert_eq!(error, format!("AArch64 {name} shift 0 is out of range"));
    }
}

/// Branch relaxation moves only targets that exist: a conditional branch to a
/// label no instruction defines is left exactly as it is, for the encoder to
/// diagnose as the unresolved label it is.
#[test]
fn relaxation_leaves_a_branch_to_an_unresolved_label_for_the_encoder_to_refuse() {
    let function = CodeFunction {
        name: "main".to_string(),
        symbol: "main".to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        instructions: vec![
            CodeInstruction::new("b.eq").field("target", "nowhere"),
            CodeInstruction::new("ret"),
        ],
        relocations: Vec::new(),
        stack_slots: Vec::new(),
    };
    let mut plan = NativeCodePlan {
        target: "linux-aarch64".to_string(),
        build_mode: crate::target::NativeBuildMode::Console,
        arch: "aarch64".to_string(),
        project: "t".to_string(),
        entry_symbol: Some("main".to_string()),
        imports: Vec::new(),
        data_objects: Vec::new(),
        functions: vec![function],
    };
    crate::arch::aarch64::encode::relax_conditional_branches(&mut plan).expect("relaxation");
    let mnemonics: Vec<&str> = plan.functions[0]
        .instructions
        .iter()
        .map(|instruction| instruction.op.mnemonic())
        .collect();
    assert_eq!(mnemonics, ["b.eq", "ret"]);
    let error = crate::arch::aarch64::encode::encode(&plan)
        .err()
        .expect("the encoder refuses the unresolved label");
    assert!(error.contains("nowhere"), "{error}");
}
