//! Cold-block placement (`engine/builder/builder_emit_helpers.rs`,
//! `emit_cold_block`), driven on a builder directly.
//!
//! A check's raise block moves out of line only when it ends in an exit (`ret`, or a
//! branch to an error-return pad): nothing can then fall out of it, so moving it
//! after the body is exact. A block that ends any other way rejoins the code after
//! it, so it stays where it is, behind a jump that skips it — moving it would make
//! its fall-through land in whatever follows the body.

use crate::arch::ops::CodeOp;
use crate::codegen::engine::builder::CodeBuilder;
use crate::codegen::engine::tests::test_support::TestPlatform;
use crate::codegen::memory::arena::graph_copy::tests::{harness, model};
use crate::target::shared::abi;

fn with_builder<R>(body: impl FnOnce(&mut CodeBuilder<'_>) -> R) -> R {
    let model = model(Vec::new());
    let harness = harness(&model);
    let platform = TestPlatform;
    let mut builder = harness.builder("cold", &platform);
    builder.cold_blocks_enabled = true;
    body(&mut builder)
}

fn has_label(instructions: &[crate::codegen::engine::types::CodeInstruction], name: &str) -> bool {
    instructions.iter().any(|instruction| {
        instruction.op == CodeOp::Label && instruction.get("name").as_deref() == Some(name)
    })
}

#[test]
fn a_block_that_ends_in_ret_moves_out_of_line() {
    with_builder(|builder| {
        builder
            .emit_cold_block("exits", |builder| {
                builder.emit(abi::return_());
                Ok(())
            })
            .expect("the block emits");
        assert!(has_label(&builder.cold_instructions, "exits"));
        assert!(!has_label(&builder.instructions, "exits"));
    });
}

#[test]
fn a_block_that_rejoins_stays_inline_behind_a_skip() {
    with_builder(|builder| {
        let register = builder.allocate_register();
        builder
            .emit_cold_block("rejoins", |builder| {
                builder.emit(abi::move_immediate(register, "Integer", "1"));
                Ok(())
            })
            .expect("the block emits");
        assert!(builder.cold_instructions.is_empty(), "nothing moved");
        let at = builder
            .instructions
            .iter()
            .position(|instruction| {
                instruction.op == CodeOp::Label
                    && instruction.get("name").as_deref() == Some("rejoins")
            })
            .expect("the block stays in the body");
        assert!(at > 0, "a jump precedes the block");
        assert_eq!(
            builder.instructions[at - 1].op,
            CodeOp::Branch,
            "skipping it"
        );
    });
}
