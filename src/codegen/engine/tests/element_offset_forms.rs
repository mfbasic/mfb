//! plan-57-D: where element `i` of a list lives (`emit_element_value_offset`,
//! `collection/layout/builder_collection_layout/access.rs`), driven directly.
//!
//! A fixed-width-scalar list is entry-free: element `i` is at `i * payloadSize` with
//! a fixed length — two instructions and no load. Any other list reads the element's
//! lookup entry. Called with `&VirtualRegister` operands, as `collections::flatten`,
//! `sort` and `groupBy` call it, so the rows exercise the instantiation production
//! uses.

use crate::arch::ops::CodeOp;
use crate::codegen::engine::tests::test_support::{BuilderHarness, TestPlatform};
use crate::codegen::engine::types::CodeInstruction;
use crate::types::ParameterType;

fn offset_of(element: &ParameterType) -> Vec<CodeInstruction> {
    let harness = BuilderHarness::default();
    let platform = TestPlatform;
    let mut builder = harness.builder("_mfb_element_offset_probe", &platform);
    let regs: Vec<_> = (0..6).map(|_| builder.temporary_vreg()).collect();
    let start = builder.instructions.len();
    builder.emit_element_value_offset(
        &regs[0], &regs[1], &regs[2], &regs[3], &regs[4], &regs[5], element,
    );
    builder.instructions.split_off(start)
}

#[test]
fn a_fixed_width_element_is_at_index_times_its_width_with_no_load() {
    for (element, width) in [
        (ParameterType::Integer, "8"),
        (ParameterType::Byte, "1"),
        (ParameterType::parse("Scalar"), "4"),
    ] {
        let emitted = offset_of(&element);
        assert_eq!(emitted.len(), 2, "{element}: two instructions");
        assert_eq!(emitted[0].op, CodeOp::MovImm);
        assert_eq!(
            emitted[0].get("value").as_deref(),
            Some(width),
            "{element}: the length is the payload width"
        );
        assert_eq!(
            emitted[1].op,
            CodeOp::Mul,
            "{element}: offset = index * width"
        );
    }
    let entry = offset_of(&ParameterType::String);
    assert!(
        entry.iter().any(|i| i.op == CodeOp::LdrU64),
        "a String element's offset is read from its lookup entry"
    );
}
