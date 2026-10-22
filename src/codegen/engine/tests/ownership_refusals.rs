//! Ownership transfer at a store (`engine/value/builder_values/ownership.rs`), driven
//! on a builder directly.
//!
//! The lowering asks these entry points only with what its analyses established —
//! a moved source that is a bound local or a pointer field of one, a closure layout
//! recorded once per `LAMBDA`, a move site only where the move analysis ran. Each
//! still checks, and refuses by name or declines to move when the premise is
//! missing. The alternative to a refusal is zeroing a slot nothing owns or the
//! wrong word of a block; the alternative to a decline is freeing a block another
//! binding still reads.

use crate::arch::ops::CodeOp;
use crate::codegen::engine::analysis::last_use::{collect_last_use_moves, op_key, Place};
use crate::codegen::engine::builder::{
    ActiveCleanup, ClosureEnvLayout, ClosureEnvWord, CodeBuilder, LocalValue, OwnedValueCleanup,
    TypeModel,
};
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::tests::test_support::TestPlatform;
use crate::codegen::engine::types::CodeInstruction;
use crate::codegen::memory::arena::graph_copy::tests::{field, harness, model, record};
use crate::target::shared::abi;
use crate::target::shared::nir::{NirOp, NirValue};
use crate::target::NativeBuildMode::Console;
use crate::testutil::{nir_for_src, CodeTarget};
use crate::types::ParameterType;

/// Run `body` on a fresh builder over `model`.
fn with_builder<R>(model: &TypeModel, body: impl FnOnce(&mut CodeBuilder<'_>) -> R) -> R {
    let harness = harness(model);
    let platform = TestPlatform;
    let mut builder = harness.builder("ownership", &platform);
    body(&mut builder)
}

/// `TYPE Pt { x AS Integer, name AS String }`: `x` lives in its slot, `name` is an
/// inlined sub-block (not a separate allocation).
fn points() -> TypeModel {
    model(vec![record(
        "Pt",
        vec![field("x", "Integer"), field("name", "String")],
    )])
}

fn bind(builder: &mut CodeBuilder<'_>, name: &str, type_: ParameterType, by_ref: bool) -> usize {
    let stack_offset = builder.allocate_stack_object(name, 8);
    builder.locals.insert(
        name.to_string(),
        LocalValue {
            type_,
            stack_offset,
            constant: None,
            by_ref,
        },
    );
    stack_offset
}

fn local(name: &str) -> NirValue {
    NirValue::Local(name.to_string())
}

/// An instruction's opcode and rendered operands, for comparing two of them.
fn rendered(instruction: &CodeInstruction) -> (CodeOp, Vec<(&'static str, String)>) {
    (
        instruction.op,
        instruction
            .fields
            .iter()
            .map(|(role, operand)| (*role, operand.render()))
            .collect(),
    )
}

/// Whether `instructions` store zero into `slot` (the "moved-from" mark).
fn zeroes_slot(instructions: &[CodeInstruction], slot: usize) -> bool {
    let zero = rendered(&abi::store_u64(abi::ZERO, abi::stack_pointer(), slot));
    instructions
        .iter()
        .any(|instruction| rendered(instruction) == zero)
}

/// plan-134-G: the source of a move gives its block up — its slot, or its pointer
/// field's word, is zeroed. A source that is no bound local, a field that is not a
/// separate allocation, and anything that is neither a local nor a field of one is
/// refused by name rather than zeroing some other word.
#[test]
fn a_moved_source_the_store_cannot_release_is_refused() {
    let model = points();
    with_builder(&model, |builder| {
        let slot = bind(builder, "p", ParameterType::declared("Pt"), false);
        let result = builder.lower_value(&local("p")).expect("p lowers");
        builder
            .release_moved_source(&local("p"), &result, false)
            .expect("a bound local releases its slot");
        assert!(
            zeroes_slot(&builder.instructions, slot),
            "the moved local's slot is zeroed"
        );

        assert_eq!(
            builder
                .release_moved_source(&local("ghost"), &result, false)
                .expect_err("an unbound local cannot be released"),
            "moved local 'ghost' is not bound"
        );
        let name = NirValue::MemberAccess {
            target: Box::new(local("p")),
            member: "name".to_string(),
        };
        assert_eq!(
            builder
                .release_moved_source(&name, &result, false)
                .expect_err("an inlined field is no separate allocation to release"),
            "moved field 'name' of 'Pt' is not a record pointer field"
        );
        let constant = NirValue::Const {
            type_: ParameterType::Integer,
            value: "1".to_string(),
        };
        assert_eq!(
            builder
                .release_moved_source(&constant, &result, false)
                .expect_err("a constant is no place"),
            "a moved store source must be a local or a field of one"
        );
    });
}

fn layout(words: Vec<ClosureEnvWord>) -> ClosureEnvLayout {
    ClosureEnvLayout {
        symbol: "_mfb_closure_env_probe".to_string(),
        words,
        auxiliary_words: 0,
        resource_words: Vec::new(),
    }
}

/// bug-781: a `LAMBDA` created at several sites records one layout; the same one
/// again is kept once, a different one is refused naming both.
#[test]
fn a_closure_layout_recorded_twice_must_agree() {
    let model = points();
    with_builder(&model, |builder| {
        let owned = layout(vec![ClosureEnvWord::Owned(ParameterType::String)]);
        builder
            .record_closure_env_layout(owned.clone())
            .expect("first");
        builder
            .record_closure_env_layout(owned.clone())
            .expect("the same layout again");
        assert_eq!(builder.closure_env_layouts.len(), 1);
        let plain = layout(vec![ClosureEnvWord::Plain]);
        assert_eq!(
            builder
                .record_closure_env_layout(plain.clone())
                .expect_err("a second layout is refused"),
            format!(
                "closure environment `_mfb_closure_env_probe` is created with two layouts: \
                 {owned:?} and {plain:?}"
            )
        );
    });
}

/// bug-778: a delivery slot's read moves its block only when the slot holds the
/// block itself. A by-reference local holds another binding's slot address, so its
/// read is copied and the parent's slot is left as it is. In a builder with no
/// statement key (a synthesized function) an owned parameter is never moved either.
#[test]
fn a_by_reference_delivery_slot_is_copied_not_moved() {
    let model = points();
    with_builder(&model, |builder| {
        let slot = bind(builder, "t", ParameterType::String, true);
        builder.trap_delivery_moves.insert("t".to_string());
        let before = builder.instructions.len();
        let read = builder
            .lower_value_owned(&local("t"))
            .expect("a by-ref String lowers");
        assert!(
            !zeroes_slot(&builder.instructions[before..], slot),
            "a by-ref slot is not zeroed"
        );
        assert_ne!(
            read.location,
            Operand::from(abi::stack_pointer()),
            "the read is a fresh copy"
        );
    });
}

/// A recursive union, so the move analysis records a site for `MUT b = a` (kept a
/// bind, not forwarded, by the reassignment after it).
const MOVES: &str = "\
IMPORT io

TYPE Leaf
  v AS Integer
END TYPE

TYPE Node2
  left AS Tree
  right AS Tree
END TYPE

UNION Tree
  Leaf
  Node2
END UNION

FUNC main() AS Integer
  LET a AS Tree = Leaf[v := 1]
  MUT b AS Tree = a
  IF toString(1) = \"2\" THEN
    b = Leaf[v := 2]
  END IF
  MATCH b
    CASE Leaf(l)
      io::print(toString(l.v))
    CASE Node2(n)
      io::print(\"node\")
  END MATCH
  RETURN 0
END FUNC
";

/// plan-134-D/G: a store may hand a graph over only at its source's last read AND
/// when the source owns its block. Without the move analysis (a synthesized
/// builder) nothing moves; at a real last read, a source that is not a bound local,
/// or is a by-reference local, owns nothing and is not moved from.
#[test]
fn a_last_read_moves_only_from_an_owning_local() {
    let module = nir_for_src(MOVES, CodeTarget::LinuxX86_64, Console)
        .unwrap_or_else(|err| panic!("the probe lowers to NIR: {err}"));
    let model = TypeModel::from_module(&module).expect("the probe's type model builds");
    let main = module
        .functions
        .iter()
        .find(|function| function.name == "main")
        .expect("main");
    let read_of_a = main
        .body
        .iter()
        .find(|op| {
            matches!(op, NirOp::Bind { name, value: Some(NirValue::Local(source)), .. }
                if name == "b" && source == "a")
        })
        .expect("`MUT b = a` is a plain bind");
    let sites = collect_last_use_moves(main, &model);
    assert!(
        sites.is_last_use(op_key(read_of_a), &Place::Local("a".to_string())),
        "the premise: `MUT b = a` is `a`'s last read"
    );
    let tree = ParameterType::declared("Tree");
    with_builder(&model, |builder| {
        assert!(
            !builder.store_is_last_use(&local("a")),
            "no move analysis, no move"
        );
        builder.move_sites = Some(sites.clone());
        builder.current_op_key = Some(op_key(read_of_a));
        assert!(
            !builder.store_is_last_use(&local("a")),
            "`a` is not bound in this builder, so it owns nothing to give up"
        );
        // A live owned-value cleanup at `a`'s slot is what "owns its block" means;
        // with it, only the by-reference flag stands between `a` and the move.
        let slot = bind(builder, "a", tree.clone(), true);
        builder
            .active_cleanups
            .push(ActiveCleanup::OwnedValue(OwnedValueCleanup {
                type_: tree.clone(),
                stack_offset: slot,
                capacity_slot: None,
                loop_alias_slot: None,
                result_wrapper: None,
            }));
        assert!(
            !builder.store_is_last_use(&local("a")),
            "a by-reference `a` holds another binding's slot, not the block"
        );
        builder.locals.get_mut("a").expect("bound").by_ref = false;
        assert!(
            builder.store_is_last_use(&local("a")),
            "an owning `a` at its last read moves"
        );
    });
}
