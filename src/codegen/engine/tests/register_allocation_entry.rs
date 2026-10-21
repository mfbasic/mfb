//! plan-34-D at the register-allocation entry (`engine/regalloc/builder_registers.rs`):
//! the stream a builder hands to selection is shared MIR and must name no
//! physical register — every register the builders use is minted virtual at
//! its emit site. A physical name arriving there is a shared-lowering
//! regression, refused with the function named rather than colored around.

use crate::codegen::engine::tests::test_support::{BuilderHarness, TestPlatform};
use crate::codegen::engine::types::CodeInstruction;

#[test]
fn a_physical_register_in_the_shared_stream_is_refused() {
    let harness = BuilderHarness::default();
    let mut builder = harness.builder("probe_fn", &TestPlatform);
    builder.emit(CodeInstruction::new("label").field("name", "entry"));
    builder.emit(
        CodeInstruction::new("mov_imm")
            .physical_field("dst", "x9")
            .field("type", "Integer")
            .field("value", "1"),
    );
    builder.emit(CodeInstruction::new("ret"));
    let Err(error) = builder.run_register_allocation() else {
        panic!("a stream naming x9 was allocated");
    };
    assert!(
        error.contains(
            "shared lowering for 'probe_fn' violated the zero-physical-register invariant"
        ),
        "{error}"
    );
}
