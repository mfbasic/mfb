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
        &Aarch64RegisterModel,
        0,
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

/// No virtual-register sentinel may survive linear-scan coloring, and a value
/// whose live range crosses a call must be spilled (a slot allocated for it).
#[test]
fn linear_scan_colors_and_spills_across_call() {
    // v0 = mov_imm 5; bl helper (clobbers); use v0 (str v0). v0 is live across
    // the call, so it must spill.
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
        &Aarch64RegisterModel,
        64,
    );
    // The value crosses the call, so a spill slot is allocated.
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
        &Aarch64RegisterModel,
        0,
    );
    assert!(outcome.spill_slots.is_empty());
    // v0 colored to some allocatable physical; both operands match.
    let colored = instructions[1].get("dst").unwrap().to_string();
    assert!(colored.starts_with('x'));
    assert_eq!(instructions[2].get("lhs"), Some(colored.as_str()));
}
