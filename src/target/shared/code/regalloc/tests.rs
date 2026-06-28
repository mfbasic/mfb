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
    let mut callee = Vec::new();
    allocate(
        RegallocKind::BumpAndReset,
        &mut instructions,
        &eager,
        &mut callee,
        &Aarch64RegisterModel,
    );
    assert_eq!(instructions[0].get("dst"), Some("x8"));
    assert_eq!(instructions[0].get("lhs"), Some("x10"));
    assert_eq!(instructions[0].get("rhs"), Some("x9"));
    assert_eq!(instructions[1].get("dst"), Some("x10"));
    assert_eq!(instructions[1].get("base"), Some("sp"));
    assert_eq!(instructions[1].get("offset"), Some("16"));
    assert!(callee.is_empty());
}
