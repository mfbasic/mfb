//! A `Bind` whose fast form was approved but whose value does not fit it falls back to
//! the ordinary form (`engine/control/builder_control/bind_op.rs`), driven on a builder.
//!
//! plan-01-vector: a small-vector binding the escape analysis approved
//! (`promotable_vector_locals`) keeps its lanes in registers and owns no block. If
//! its initializer does not lower to a register-native value after all — the
//! analysis over-approved — the binding is stored as an ordinary block, which it
//! then OWNS: an `OwnedValue` cleanup at its slot and an entry in the owned slots.
//! Without the fallback the block would be lost (a leak) or, worse, the binding
//! would read lanes that were never set.

use crate::arch::ops::CodeOp;
use crate::codegen::engine::builder::{LocalValue, TypeModel};
use crate::codegen::engine::tests::test_support::TestPlatform;
use crate::codegen::engine::types::CodeInstruction;
use crate::codegen::memory::arena::graph_copy::tests::harness;
use crate::target::shared::nir::{NirOp, NirValue};
use crate::target::NativeBuildMode::Console;
use crate::testutil::{nir_for_src, CodeTarget};
use crate::types::ParameterType;

/// A program that uses `vector::Float2`, so the module's type model has its layout.
const VECTORS: &str = "\
IMPORT vector
IMPORT io

FUNC main() AS Integer
  LET p AS vector::Float2 = vector::Float2[x := 1.0, y := 2.0]
  io::print(toString(p.x))
  RETURN 0
END FUNC
";

fn float2() -> ParameterType {
    ParameterType::declared("vector.Float2")
}

fn bind(name: &str, value: NirValue) -> NirOp {
    NirOp::Bind {
        mutable: false,
        name: name.to_string(),
        type_: float2(),
        value: Some(value),
    }
}

#[test]
fn an_over_approved_vector_promotion_falls_back_to_an_owned_block() {
    let module = nir_for_src(VECTORS, CodeTarget::LinuxX86_64, Console)
        .unwrap_or_else(|err| panic!("the probe lowers to NIR: {err}"));
    let model = TypeModel::from_module(&module).expect("the probe's type model builds");
    let harness = harness(&model);
    let platform = TestPlatform;
    let mut builder = harness.builder("bind", &platform);

    // A constructor lowers register-native: the approved binding keeps its lanes.
    builder.promotable_vector_locals.insert("u".to_string());
    let lane = |value: &str| NirValue::Const {
        type_: ParameterType::Float,
        value: value.to_string(),
    };
    builder
        .lower_ops(&[bind(
            "u",
            NirValue::Constructor {
                type_: float2(),
                args: vec![lane("1.0"), lane("2.0")],
            },
        )])
        .expect("a native initializer binds");
    assert!(builder.promoted_vector_locals.contains_key("u"));
    let lanes_slot = builder.locals.get("u").map(|local| local.stack_offset);
    assert!(
        lanes_slot.is_none_or(|slot| !builder.owned_value_slots.contains(&slot)),
        "a binding in lanes owns no block"
    );

    // A block held in another local does not: the binding falls back to that block
    // and owns it.
    let block_slot = builder.allocate_stack_object("w", 8);
    builder.locals.insert(
        "w".to_string(),
        LocalValue {
            type_: float2(),
            stack_offset: block_slot,
            constant: None,
            by_ref: false,
        },
    );
    builder.promotable_vector_locals.insert("v".to_string());
    let start = builder.instructions.len();
    builder
        .lower_ops(&[bind("v", NirValue::Local("w".to_string()))])
        .expect("a non-native initializer still binds");
    assert!(
        !builder.promoted_vector_locals.contains_key("v"),
        "a value with no lanes is not recorded as lanes"
    );
    // Owning it means the scope's end frees it: closing the scope `lower_ops` opened
    // reloads `v`'s slot to drop the block it stored there.
    let slot = builder.locals.get("v").expect("v is bound").stack_offset;
    let at_slot = |instruction: &CodeInstruction, op: CodeOp| {
        instruction.op == op && instruction.get("offset") == Some(slot.to_string())
    };
    let stored = builder.instructions[start..]
        .iter()
        .position(|instruction| at_slot(instruction, CodeOp::StrU64))
        .expect("the fallback stores the block in v's slot");
    assert!(
        builder.instructions[start + stored + 1..]
            .iter()
            .any(|instruction| at_slot(instruction, CodeOp::LdrU64)),
        "the scope end reloads v's slot to free the block it owns"
    );
    assert!(
        builder.owned_value_slots.contains(&slot),
        "and the slot is on the entry zero-fill list"
    );
}
