//! Content equality (`collection/compare/builder_collection_compare/content.rs`) over a
//! data union compares the active variant's record field by field, so it reads each
//! variant's fields from the type model. A model that names a variant but describes
//! no fields for it is refused by name, rather than compared as an empty record
//! (which would call every two values of that variant equal).

use crate::codegen::engine::builder::TypeModel;
use crate::codegen::engine::tests::test_support::{BuilderHarness, TestPlatform};
use crate::testutil::{nir_for_src, CodeTarget};
use crate::types::ParameterType;

const PROGRAM: &str = "\
IMPORT io

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

FUNC main() AS Integer
  io::print(\"ok\")
  RETURN 0
END FUNC
";

fn compare(edit: impl FnOnce(&mut TypeModel)) -> Result<(), String> {
    compare_type(&ParameterType::declared("Shape"), edit).map(|_| ())
}

fn compare_type(
    type_: &ParameterType,
    edit: impl FnOnce(&mut TypeModel),
) -> Result<Vec<crate::codegen::engine::types::CodeInstruction>, String> {
    let module = nir_for_src(
        PROGRAM,
        CodeTarget::LinuxX86_64,
        crate::target::NativeBuildMode::Console,
    )
    .expect("the program lowers");
    let mut model = TypeModel::from_module(&module).expect("the model builds");
    edit(&mut model);
    let mut harness = BuilderHarness {
        type_model: model,
        ..BuilderHarness::default()
    };
    harness
        .string_symbols
        .insert("Allocation failed.".to_string(), "_mfb_str_oom".to_string());
    let platform = TestPlatform;
    let mut builder = harness.builder("_mfb_content_probe", &platform);
    let left = builder.temporary_vreg();
    let right = builder.temporary_vreg();
    builder.emit_values_content_equal_branch(type_, left, right, "probe_equal", "probe_unequal")?;
    Ok(builder.instructions)
}

#[test]
fn a_union_variant_without_fields_is_refused() {
    compare(|_| {}).expect("a described union compares");
    let tag = ParameterType::declared("Tag");
    let Err(err) = compare(|model| {
        model.record_fields.remove(&tag);
        model.union_variant_fields.remove(&tag);
    }) else {
        panic!("Tag has no fields in this model");
    };
    assert!(
        err.contains("native union variant 'Tag' does not resolve"),
        "{err}"
    );
}

/// Every fixed-width element (`list_element_is_fixed_width`'s whole domain bar the
/// unit type) is compared as one aligned byte range over the entry-free data: its
/// shape is a word, so equal bytes are equal content and different bytes different.
#[test]
fn every_fixed_width_list_is_compared_as_one_byte_range() {
    for element in [
        "Boolean", "Byte", "Scalar", "Integer", "Float", "Fixed", "Money",
    ] {
        let list = ParameterType::list_of(ParameterType::parse(element));
        let emitted = compare_type(&list, |_| {})
            .unwrap_or_else(|err| panic!("List OF {element} compares: {err}"));
        assert!(
            emitted
                .iter()
                .any(|i| i.op == crate::arch::ops::CodeOp::Label
                    && i.get("name")
                        .is_some_and(|name| name.starts_with("content_equal_range"))),
            "List OF {element} is compared as one range"
        );
    }
}
