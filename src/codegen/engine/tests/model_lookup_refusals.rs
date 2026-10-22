//! The type-model lookups the thread-transfer copy, the graph drop and the default
//! value share refuse a type the model does not describe, by name.
//!
//! `record_pointer_edges` and `union_variants_by_tag`
//! (`memory/arena/builder_arena_transfer/transfer.rs`) are the one enumeration of a
//! record's / union's pointer edges and variant tags that the copy and the drop both
//! walk; `lower_default_value` (`memory/value/builder_value_semantics/default_value.rs`)
//! builds a data union's default around its first defaultable variant's tag. A
//! program's model always describes every type it names; these rows hand each lookup
//! a type the model lacks — a non-record, a variant whose tag entry is missing — and
//! pin that the answer is a refusal naming it, never an empty edge list or tag 0
//! (which would copy a record shallowly, or default to the wrong variant).

use crate::codegen::engine::builder::{CodeBuilder, TypeModel};
use crate::codegen::engine::tests::test_support::{BuilderHarness, TestPlatform};
use crate::testutil::{nir_for_src, CodeTarget};
use crate::types::ParameterType;

const PROGRAM: &str = "\
IMPORT io

TYPE Circle
  r AS Integer
END TYPE

TYPE Label
  text AS String
END TYPE

UNION Shape
  Circle
  Label
END UNION

FUNC main() AS Integer
  io::print(\"ok\")
  RETURN 0
END FUNC
";

/// Run `body` on a builder over the program's model, after `edit` has changed it.
fn with_model<R>(
    edit: impl FnOnce(&mut TypeModel),
    body: impl FnOnce(&mut CodeBuilder<'_>) -> R,
) -> R {
    let module = nir_for_src(
        PROGRAM,
        CodeTarget::LinuxX86_64,
        crate::target::NativeBuildMode::Console,
    )
    .expect("the program lowers");
    let mut model = TypeModel::from_module(&module).expect("the model builds");
    edit(&mut model);
    let harness = BuilderHarness {
        type_model: model,
        ..BuilderHarness::default()
    };
    let platform = TestPlatform;
    let mut builder = harness.builder("_mfb_model_lookup_probe", &platform);
    body(&mut builder)
}

fn shape() -> ParameterType {
    ParameterType::declared("Shape")
}

/// A record's pointer edges are read from its declared fields; a type with none
/// is refused, not answered with an empty edge list.
#[test]
fn record_pointer_edges_refuse_a_type_the_model_has_no_fields_for() {
    with_model(
        |_| {},
        |builder| {
            let Err(err) = builder.record_pointer_edges(&ParameterType::Integer) else {
                panic!("an Integer has no record fields");
            };
            assert!(
                err.contains("native thread transfer record type 'Integer' does not resolve"),
                "{err}"
            );
            let edges = builder
                .record_pointer_edges(&ParameterType::declared("Label"))
                .expect("a declared record resolves");
            assert!(
                edges
                    .iter()
                    .all(|(_, field)| *field != ParameterType::Integer),
                "only pointer fields are edges: {edges:?}"
            );
        },
    );
}

/// A union's variants are dispatched by tag; a variant with no tag entry is
/// refused by name.
#[test]
fn union_variants_by_tag_refuse_a_variant_without_a_tag() {
    with_model(
        |model| {
            model
                .union_variant_tags
                .remove(&ParameterType::declared("Label"));
        },
        |builder| {
            let Err(err) = builder.union_variants_by_tag(&shape()) else {
                panic!("Label has no tag in this model");
            };
            assert!(
                err.contains("native thread transfer union variant 'Label' has no tag"),
                "{err}"
            );
        },
    );
    with_model(
        |_| {},
        |builder| {
            let variants = builder
                .union_variants_by_tag(&shape())
                .expect("every variant has a tag");
            let tags: Vec<usize> = variants.iter().map(|(_, tag, _)| *tag).collect();
            let mut sorted = tags.clone();
            sorted.sort_unstable();
            assert_eq!(tags, sorted, "variants come back in tag order");
        },
    );
}

/// A data union's default is built around its first defaultable variant's tag; a
/// variant whose tag is missing is refused by name.
#[test]
fn a_union_default_refuses_a_variant_without_a_tag() {
    with_model(
        |model| {
            model.union_variant_tags.clear();
        },
        |builder| {
            let Err(err) = builder.lower_default_value(&shape()) else {
                panic!("no variant of Shape has a tag in this model");
            };
            assert!(
                err.contains("native code union variant '") && err.contains("' does not resolve"),
                "{err}"
            );
        },
    );
}
