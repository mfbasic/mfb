//! The default value of a DATA union, and of a record that carries one.
//!
//! The front end never asks for it from source: a bare `UNION` is not defaultable
//! (`TYPE_MUT_REQUIRES_DEFAULTABLE_TYPE`). But an IMPORTED record is optimistically
//! defaultable (`is_defaultable`'s `imported_types_unknown`), and a record from a
//! package may well carry a union field, so the back end has to materialize one
//! when the trap temp of a fallible call needs it.
//!
//! `lower_default_value` must do three things that no front-end rule backs up:
//!
//!   * pick the first variant whose payload is itself defaultable, and wrap it in
//!     the real flat union layout (the tag it stores is that variant's own tag);
//!   * skip a variant that would loop back into the union being defaulted, so a
//!     self-reachable union terminates instead of recursing forever;
//!   * refuse, with a message naming the union, when no variant can be built.
//!
//! These drive the emitter directly on the type model of a real program, since no
//! program reaches it.

use crate::arch::ops::CodeOp;
use crate::codegen::engine::builder::{TypeModel, ValueResult};
use crate::codegen::engine::tests::test_support::{BuilderHarness, TestPlatform};
use crate::testutil::{nir_for_src, CodeTarget};
use crate::types::ParameterType;

const PROGRAM: &str = "\
IMPORT fs
IMPORT io
IMPORT tcp

TYPE Dot
  x AS Integer
END TYPE

TYPE Tag
  name AS String
END TYPE

UNION Shape
  Dot
  Tag
END UNION

TYPE Holder
  s AS Shape
  n AS Integer
END TYPE

TYPE Node
  inner AS Tree
END TYPE

TYPE Leaf
  v AS Integer
END TYPE

UNION Tree
  Node
  Leaf
END UNION

TYPE Cyclic
  back AS Cycle
END TYPE

UNION Cycle
  Cyclic
END UNION

TYPE AFn
  cb AS FUNC(Integer) AS Integer
  n AS Integer
END TYPE

TYPE ANested
  inner AS Holder
END TYPE

TYPE Zplain
  n AS Integer
END TYPE

TYPE Cursor
  pos AS Integer
END TYPE

UNION Stream
  fs::File
  tcp::Socket
END UNION

UNION Callable
  AFn
  Zplain
END UNION

UNION Wrapper
  ANested
  Zplain
END UNION

FUNC main() AS Integer
  io::print(\"ok\")
  RETURN 0
END FUNC
";

fn model() -> TypeModel {
    let module = nir_for_src(
        PROGRAM,
        CodeTarget::LinuxX86_64,
        crate::target::NativeBuildMode::Console,
    )
    .expect("the program lowers");
    TypeModel::from_module(&module).expect("the model builds")
}

/// Lower the default of `name`; return the model, the result and the union tags
/// the emitted code stored.
fn default_of(name: &str) -> (TypeModel, Result<ValueResult, String>, Vec<String>) {
    default_of_type(&ParameterType::declared(name))
}

/// [`default_of`] for an arbitrary type, such as a union carrying a `STATE` clause.
fn default_of_type(type_: &ParameterType) -> (TypeModel, Result<ValueResult, String>, Vec<String>) {
    // The allocation's failure arm raises `ErrOutOfMemory`, whose message is a data
    // object a whole program interns; a bare builder is told where it is.
    let mut harness = BuilderHarness {
        type_model: model(),
        ..BuilderHarness::default()
    };
    harness
        .string_symbols
        .insert("Allocation failed.".to_string(), "_mfb_str_oom".to_string());
    let platform = TestPlatform;
    let mut builder = harness.builder("_mfb_default_probe", &platform);
    let result = builder.lower_default_value(type_);
    let tags = builder
        .instructions
        .iter()
        .filter(|i| i.op == CodeOp::MovImm && i.get("type").as_deref() == Some("UnionTag"))
        .filter_map(|i| i.get("value"))
        .collect();
    (harness.type_model.clone(), result, tags)
}

fn tag_of(model: &TypeModel, variant: &str) -> String {
    model.union_variant_tags[&ParameterType::declared(variant)].to_string()
}

#[test]
fn a_union_defaults_to_its_first_variant_in_the_real_layout() {
    let (model, result, tags) = default_of("Shape");
    let result = result.expect("a union of defaultable records has a default");
    assert_eq!(result.type_, ParameterType::declared("Shape"));
    assert_eq!(
        tags,
        vec![tag_of(&model, "Dot")],
        "the default wraps the FIRST variant, `Dot`, with that variant's own tag"
    );
}

#[test]
fn a_record_with_a_union_field_defaults_that_field_too() {
    let (model, result, tags) = default_of("Holder");
    let result = result.expect("a record whose union field has a default has a default");
    assert_eq!(result.type_, ParameterType::declared("Holder"));
    assert_eq!(tags, vec![tag_of(&model, "Dot")]);
}

#[test]
fn a_variant_holding_a_function_is_still_buildable() {
    // A function value defaults to the null word, so it does not disqualify its
    // variant: the union defaults to its FIRST one.
    let (model, result, tags) = default_of("Callable");
    result.expect("`Callable` has a default");
    assert_eq!(tags, vec![tag_of(&model, "AFn")]);
}

#[test]
fn a_variant_holding_a_record_with_a_union_field_defaults_both_levels() {
    let (model, result, mut tags) = default_of("Wrapper");
    result.expect("`ANested` is buildable through its `Holder`'s own union field");
    tags.sort();
    let mut want = vec![tag_of(&model, "ANested"), tag_of(&model, "Dot")];
    want.sort();
    assert_eq!(
        tags, want,
        "the outer union wraps `ANested`, whose record's union field wraps `Dot`"
    );
}

#[test]
fn a_variant_that_loops_back_into_the_union_is_skipped() {
    let (model, result, tags) = default_of("Tree");
    result.expect("`Leaf` is a variant that does not recurse, so `Tree` has a default");
    assert_eq!(
        tags,
        vec![tag_of(&model, "Leaf")],
        "`Node` holds a bare `Tree`, so defaulting it would never terminate; the \
         default must be the non-recursive `Leaf`"
    );
}

#[test]
fn a_union_with_no_buildable_variant_is_refused_by_name() {
    let (_, result, tags) = default_of("Cycle");
    let message = match result {
        Ok(_) => panic!("every variant of `Cycle` leads back to `Cycle`: nothing to build"),
        Err(message) => message,
    };
    assert!(
        message.contains("Cycle"),
        "the refusal names the union: {message}"
    );
    assert!(
        tags.is_empty(),
        "no tag may be stored for a default that was refused"
    );
}

/// A resource union's default is a closed record in a two-word box, and when the
/// binding carries a `STATE` clause the record's `STATE` slot is initialized with a
/// default state record too, so a `RECOVER`ed union's `.state` never reads null.
#[test]
fn a_stateful_resource_union_default_initializes_its_state() {
    let plain = ParameterType::declared("Stream");
    let stateful = plain.with_state(&ParameterType::declared("Cursor"));
    let (_, plain_result, _) = default_of_type(&plain);
    plain_result.expect("a resource union has a closed default");
    let harness_len = |type_: &ParameterType| {
        let harness = BuilderHarness {
            type_model: model(),
            string_symbols: [("Allocation failed.".to_string(), "_mfb_str_oom".to_string())]
                .into_iter()
                .collect(),
            ..BuilderHarness::default()
        };
        let platform = TestPlatform;
        let mut builder = harness.builder("_mfb_default_probe", &platform);
        builder.lower_default_value(type_).expect("has a default");
        builder.instructions.len()
    };
    assert!(
        harness_len(&stateful) > harness_len(&plain),
        "the stateful default also allocates and stores the state record"
    );
}
