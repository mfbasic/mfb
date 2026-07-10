use super::*;
use crate::arch::aarch64::regmodel::Aarch64RegisterModel;

#[test]
fn vreg_roundtrips() {
    assert_eq!(parse_vreg(&vreg_name(0)), Some(0));
    assert_eq!(parse_vreg(&vreg_name(42)), Some(42));
    assert_eq!(parse_vreg("x9"), None);
    assert_eq!(parse_vreg("sp"), None);
    assert_eq!(parse_vreg("Integer"), None);
    assert_eq!(parse_vreg("loop_3"), None);
}

#[test]
fn parse_kind_known_and_unknown() {
    assert_eq!(parse_kind("bump"), Ok(RegallocKind::BumpAndReset));
    assert!(parse_kind("graph-coloring").is_err());
}

#[test]
fn bump_rewrite_substitutes_eager_physicals() {
    // dst = %v0, lhs = %v1, rhs = x9 (a hardcoded physical), offset untouched.
    let mut instructions = vec![
        CodeInstruction::new("add")
            .field("dst", &vreg_name(0))
            .field("lhs", &vreg_name(1))
            .field("rhs", "x9"),
        CodeInstruction::new("ldr_u64")
            .field("dst", &vreg_name(1))
            .field("base", "sp")
            .field("offset", "16"),
    ];
    let eager = vec!["x8".to_string(), "x10".to_string()];
    let outcome = allocate(
        RegallocKind::BumpAndReset,
        &mut instructions,
        &eager,
        &[],
        &Aarch64RegisterModel,
        0,
        &[],
    );
    assert_eq!(instructions[0].get("dst"), Some("x8"));
    assert_eq!(instructions[0].get("lhs"), Some("x10"));
    assert_eq!(instructions[0].get("rhs"), Some("x9"));
    assert_eq!(instructions[1].get("dst"), Some("x10"));
    assert_eq!(instructions[1].get("base"), Some("sp"));
    assert_eq!(instructions[1].get("offset"), Some("16"));
    assert!(outcome.spill_slots.is_empty());
    assert!(outcome.extra_callee_saved.is_empty());
}

/// A value live across a call must be colored to a callee-saved register the
/// call preserves (not a caller-saved one the call clobbers). A generic
/// (PCS-compliant) call preserves `x19`–`x28`, so the value stays in a register
/// rather than spilling; whatever register it gets, it must not be caller-saved.
#[test]
fn linear_scan_keeps_value_across_call_in_callee_saved() {
    // v0 = mov_imm 5; bl helper; use v0 (str v0). v0 is live across the call.
    let mut instructions = vec![
        CodeInstruction::new("label").field("name", "entry"),
        CodeInstruction::new("mov_imm")
            .field("dst", &vreg_name(0))
            .field("type", "Integer")
            .field("value", "5"),
        CodeInstruction::new("bl").field("target", "helper"),
        CodeInstruction::new("str_u64")
            .field("src", &vreg_name(0))
            .field("base", "sp")
            .field("offset", "0"),
        CodeInstruction::new("ret"),
    ];
    let outcome = allocate(
        RegallocKind::LinearScan,
        &mut instructions,
        &[String::new()],
        &[],
        &Aarch64RegisterModel,
        64,
        &[],
    );
    // No spill, and the chosen register is callee-saved (the call preserves it).
    assert!(outcome.spill_slots.is_empty());
    let colored = instructions[1].get("dst").unwrap().to_string();
    assert!(
        Aarch64RegisterModel.is_callee_saved(&colored),
        "value across a call must be in a callee-saved register, got {colored}"
    );
    // No sentinel survives anywhere in the rewritten stream.
    for instruction in &instructions {
        for (_field, value) in &instruction.fields {
            assert!(
                parse_vreg(value).is_none(),
                "virtual register {value} survived coloring"
            );
        }
    }
}

/// `_mfb_arena_alloc` is hand-written and tramples callee-saved `x20`–`x28` on top
/// of the caller-saved set, so no integer register survives it: an integer value
/// live across the call must spill (a slot is allocated for it).
#[test]
fn linear_scan_spills_integer_across_arena_alloc() {
    let mut instructions = vec![
        CodeInstruction::new("label").field("name", "entry"),
        CodeInstruction::new("mov_imm")
            .field("dst", &vreg_name(0))
            .field("type", "Integer")
            .field("value", "5"),
        CodeInstruction::new("bl").field("target", "_mfb_arena_alloc"),
        CodeInstruction::new("str_u64")
            .field("src", &vreg_name(0))
            .field("base", "sp")
            .field("offset", "0"),
        CodeInstruction::new("ret"),
    ];
    let outcome = allocate(
        RegallocKind::LinearScan,
        &mut instructions,
        &[String::new()],
        &[],
        &Aarch64RegisterModel,
        64,
        &[],
    );
    assert_eq!(outcome.spill_slots, vec![64]);
    // No sentinel survives anywhere in the rewritten stream.
    for instruction in &instructions {
        for (_field, value) in &instruction.fields {
            assert!(
                parse_vreg(value).is_none(),
                "virtual register {value} survived coloring"
            );
        }
    }
}

/// bug-54: when the caller-saved allocatable bank (`x8`–`x17`) is fully occupied
/// at a spill-reload point, the "genuinely free" scratch selection borrows a
/// callee-saved register (`x20`+) but emits no save/restore for it. That register
/// must therefore be recorded in the frame's callee-saved save set, or the
/// callee silently clobbers the caller's `x20`.
///
/// Construction: `v0` crosses `_mfb_arena_alloc` (which tramples every integer
/// register) so it must spill; ten short-lived colored vregs (`v1`–`v10`)
/// saturate `x8`–`x17` at the instruction that reloads `v0`, forcing its reload
/// scratch onto callee-saved `x20`.
#[test]
fn linear_scan_records_callee_saved_reload_scratch_int() {
    let mut instructions = vec![
        CodeInstruction::new("label").field("name", "entry"),
        // v0: defined, then live across the arena-alloc call -> spilled.
        CodeInstruction::new("mov_imm")
            .field("dst", &vreg_name(0))
            .field("type", "Integer")
            .field("value", "5"),
        CodeInstruction::new("bl").field("target", "_mfb_arena_alloc"),
    ];
    // v1..v10: defined after the call (never cross it), all live across the
    // reload of v0 below, so they occupy x8..x17 there.
    for k in 1..=10u32 {
        instructions.push(
            CodeInstruction::new("mov_imm")
                .field("dst", &vreg_name(k))
                .field("type", "Integer")
                .field("value", "1"),
        );
    }
    // Reload point: uses spilled v0 while v1..v10 are all live -> reload scratch
    // must come from the callee-saved bank (x20).
    instructions.push(
        CodeInstruction::new("str_u64")
            .field("src", &vreg_name(0))
            .field("base", "sp")
            .field("offset", "0"),
    );
    // Keep v1..v10 live past the reload (two per `add`, dst is a non-allocatable
    // physical so it adds no pressure).
    for pair in 0..5u32 {
        instructions.push(
            CodeInstruction::new("add")
                .field("dst", "x0")
                .field("lhs", &vreg_name(pair * 2 + 1))
                .field("rhs", &vreg_name(pair * 2 + 2)),
        );
    }
    instructions.push(CodeInstruction::new("ret"));

    // `LinearScan` ignores the eager (bump) assignment.
    let outcome = allocate(
        RegallocKind::LinearScan,
        &mut instructions,
        &[],
        &[],
        &Aarch64RegisterModel,
        64,
        &[],
    );
    // v0 spilled (a slot was allocated), and the callee-saved register borrowed
    // as its reload scratch is in the frame's save set.
    assert!(!outcome.spill_slots.is_empty(), "v0 must spill across the call");
    assert!(
        outcome.extra_callee_saved.contains(&"x20".to_string()),
        "callee-saved reload scratch x20 must be saved by the frame, got {:?}",
        outcome.extra_callee_saved
    );
}

/// bug-54, FP class: the same hole exists for `d8`–`d15`. Saturate the 24
/// caller-saved FP registers (`d0`–`d7`, `d16`–`d31`) with colored vregs at a
/// reload point, and force a spill via full-file physical pressure earlier in the
/// spilled value's live range, so its reload scratch lands on callee-saved `d8`.
#[test]
fn linear_scan_records_callee_saved_reload_scratch_fp() {
    // f200: the spilled value. Defined first; its live range spans a region where
    // every physical FP register is made busy (below), so it cannot be colored.
    let mut instructions = vec![
        CodeInstruction::new("label").field("name", "entry"),
        CodeInstruction::new("fabs_d")
            .field("dst", &fp_vreg_name(200))
            .field("src", "sp"),
    ];
    // Saturation region: make each physical d0..d31 busy once (dst only; `src=sp`
    // is not an FP operand, so no liveness chain forms). This forbids f200 from
    // every physical over its interval -> f200 spills. These are literal
    // physicals, not colored homes, so they do not enter the save set themselves.
    for r in 0..32u32 {
        instructions.push(
            CodeInstruction::new("fabs_d")
                .field("dst", &format!("d{r}"))
                .field("src", "sp"),
        );
    }
    // 24 colored fillers, defined after the saturation region so they color to
    // the 24 caller-saved FP registers, all live across the reload below.
    for k in 0..24u32 {
        instructions.push(
            CodeInstruction::new("fabs_d")
                .field("dst", &fp_vreg_name(k))
                .field("src", "sp"),
        );
    }
    // Reload point for f200: 24 caller-saved FP registers occupied -> scratch
    // must be callee-saved d8.
    instructions.push(
        CodeInstruction::new("fabs_d")
            .field("dst", "sp")
            .field("src", &fp_vreg_name(200)),
    );
    // Keep the 24 fillers live past the reload.
    for k in 0..24u32 {
        instructions.push(
            CodeInstruction::new("fabs_d")
                .field("dst", "sp")
                .field("src", &fp_vreg_name(k)),
        );
    }
    instructions.push(CodeInstruction::new("ret"));

    // `LinearScan` ignores the eager (bump) assignment.
    let outcome = allocate(
        RegallocKind::LinearScan,
        &mut instructions,
        &[],
        &[],
        &Aarch64RegisterModel,
        0,
        &[],
    );
    assert!(!outcome.spill_slots.is_empty(), "f200 must spill under full FP pressure");
    assert!(
        outcome.extra_callee_saved.contains(&"d8".to_string()),
        "callee-saved reload scratch d8 must be saved by the frame, got {:?}",
        outcome.extra_callee_saved
    );
}

/// A value with a short, call-free range is colored to a physical register, not
/// spilled.
#[test]
fn linear_scan_colors_short_range_in_register() {
    let mut instructions = vec![
        CodeInstruction::new("label").field("name", "entry"),
        CodeInstruction::new("mov_imm")
            .field("dst", &vreg_name(0))
            .field("type", "Integer")
            .field("value", "7"),
        CodeInstruction::new("add")
            .field("dst", "x0")
            .field("lhs", &vreg_name(0))
            .field("rhs", &vreg_name(0)),
        CodeInstruction::new("ret"),
    ];
    let outcome = allocate(
        RegallocKind::LinearScan,
        &mut instructions,
        &[String::new()],
        &[],
        &Aarch64RegisterModel,
        0,
        &[],
    );
    assert!(outcome.spill_slots.is_empty());
    // v0 colored to some allocatable physical; both operands match.
    let colored = instructions[1].get("dst").unwrap().to_string();
    assert!(colored.starts_with('x'));
    assert_eq!(instructions[2].get("lhs"), Some(colored.as_str()));
}
