//! `lower_value`'s edge arms (`engine/value/builder_values/lower_inner.rs`), on a
//! builder directly.
//!
//! * A union VARIANT's name read as a value is its tag (`VariantTag`), the operand a
//!   `MATCH` pattern compares against — not a load of some local of that name.
//! * A `typeName` call the static resolver cannot type has no runtime fallback, so it
//!   is refused rather than lowered to a call of a helper that does not exist.
//! * A `Constructor` of a type that is not a record names nothing a constructor can
//!   build (bug-548: a union variant is a record, a resource has no constructor), so
//!   it fails closed rather than guessing a layout.
//! * bug-744: the payload of a `Result OF Nothing` is the one `Nothing` value, 0 —
//!   never the payload word, which a `Nothing` callee left unset.

use crate::codegen::engine::builder::{CodeBuilder, LocalValue, TypeModel};
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::tests::test_support::TestPlatform;
use crate::codegen::memory::arena::graph_copy::tests::{field, harness, model, record, union};
use crate::target::shared::abi;
use crate::target::shared::nir::{NirSourceLoc, NirValue};
use crate::target::shared::runtime::RuntimeHelper;
use crate::types::ParameterType;

/// `UNION Shape { Circle }` over `TYPE Circle { r AS Integer }`.
fn shapes() -> TypeModel {
    model(vec![
        record("Circle", vec![field("r", "Integer")]),
        union("Shape", vec![("Circle", vec![field("r", "Integer")])]),
    ])
}

fn with_builder<R>(body: impl FnOnce(&mut CodeBuilder<'_>) -> R) -> R {
    let model = shapes();
    let harness = harness(&model);
    let platform = TestPlatform;
    let mut builder = harness.builder("value_edges", &platform);
    body(&mut builder)
}

fn refused<T>(result: Result<T, String>, what: &str) -> String {
    match result {
        Err(error) => error,
        Ok(_) => panic!("{what} must be refused"),
    }
}

fn local(name: &str) -> NirValue {
    NirValue::Local(name.to_string())
}

#[test]
fn a_variant_name_read_as_a_value_is_its_tag() {
    with_builder(|builder| {
        let tag = builder
            .lower_value(&local("Circle"))
            .expect("a variant tag");
        assert_eq!(tag.type_, ParameterType::named("VariantTag"));
        assert_eq!(tag.location, Operand::from("Circle".to_string()));
        assert!(
            builder.lower_value(&local("Square")).is_err(),
            "a name that is neither a variant nor a bound local is still refused"
        );
    });
}

#[test]
fn a_type_name_the_resolver_cannot_type_is_refused() {
    with_builder(|builder| {
        let call = NirValue::RuntimeCall {
            helper: RuntimeHelper::General,
            target: "typeName".to_string(),
            args: vec![local("untyped")],
            loc: NirSourceLoc::default(),
        };
        assert_eq!(
            refused(
                builder.lower_value(&call),
                "typeName of an untypable argument"
            ),
            "native code cannot determine typeName argument type"
        );
    });
}

#[test]
fn a_constructor_of_a_type_that_is_not_a_record_is_refused() {
    with_builder(|builder| {
        let construct = NirValue::Constructor {
            type_: ParameterType::declared("Shape"),
            args: Vec::new(),
        };
        assert_eq!(
            refused(builder.lower_value(&construct), "a union constructor"),
            "native code constructor type 'Shape' is not a record; a union variant is either \
             a record (constructed above) or a resource (which has no constructor)"
        );
    });
}

#[test]
fn the_payload_of_a_result_of_nothing_is_nothing_itself() {
    with_builder(|builder| {
        let stack_offset = builder.allocate_stack_object("r", 8);
        builder.locals.insert(
            "r".to_string(),
            LocalValue {
                type_: ParameterType::ResultOf(Box::new(ParameterType::Nothing)),
                stack_offset,
                constant: None,
                by_ref: false,
            },
        );
        let before = builder.instructions.len();
        let read = builder
            .lower_value(&NirValue::ResultValue {
                value: Box::new(local("r")),
            })
            .expect("a Result OF Nothing's payload reads");
        assert_eq!(read.type_, ParameterType::Nothing);
        let zero = abi::move_immediate(abi::Reg::from(&read.location), "Integer", "0");
        let last = builder.instructions.last().expect("an instruction");
        assert_eq!(last.op, zero.op, "the payload is materialized, not loaded");
        assert_eq!(
            last.get("value").as_deref(),
            Some("0"),
            "and it is NOTHING (0)"
        );
        assert!(
            builder.instructions[before..]
                .iter()
                .all(|instruction| instruction.get("offset").as_deref() != Some("16")),
            "no read of the payload word at +16"
        );
    });
}
