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
    builder.emit_values_content_equal_branch(
        &ParameterType::declared("Shape"),
        left,
        right,
        "probe_equal",
        "probe_unequal",
    )
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
