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

/// Each vector/FP family emitter encodes only its own ops: handed an op of
/// another family it refuses by name and emits nothing, rather than OR-ing the
/// registers into some other instruction's base word. (`emit_instruction`
/// routes each mnemonic to exactly one family, so only a direct call can hand
/// one the wrong op.)
#[test]
fn a_vector_family_emitter_refuses_an_op_of_another_family() {
    let mut encoder = fresh_encoder();
    assert_eq!(
        encoder.emit_v_three_same(CodeOp::FAbsV, 0, 1, 2),
        Err("fabs_v is not a three-same NEON op".to_string())
    );
    assert_eq!(
        encoder.emit_v_two_misc(CodeOp::FAddV, 0, 1),
        Err("fadd_v is not a two-reg-misc NEON op".to_string())
    );
    assert_eq!(
        encoder.emit_v_shift_imm(CodeOp::FAddV, 0, 1, 3),
        Err("fadd_v is not a NEON shift-imm op".to_string())
    );
    assert_eq!(
        encoder.emit_fma_d(CodeOp::FAddV, 0, 1, 2, 3),
        Err("fadd_v is not a scalar FMA op".to_string())
    );
    assert!(encoder.text.is_empty(), "a refusal emits no word");
}

/// `shl.2d` takes a left shift of 0..=63. The operand parser already refuses an
/// amount of 64 or more, and the emitter holds the same bound itself; 63 is the
/// last encodable shift.
#[test]
fn a_2d_left_shift_past_63_is_refused_by_the_emitter() {
    let mut encoder = fresh_encoder();
    assert_eq!(
        encoder.emit_v_shift_imm(CodeOp::ShlV, 0, 1, 64),
        Err("AArch64 shl.2d shift 64 is out of range".to_string())
    );
    assert!(encoder.text.is_empty());
    encoder
        .emit_v_shift_imm(CodeOp::ShlV, 0, 1, 63)
        .expect("63 encodes");
    assert_eq!(encoder.text.len(), 4);
}
