//! The type model as the registry's kind oracle, and its refusal of a type kind
//! it does not know (`engine/validation/validation.rs`).
//!
//! `TypeModel` answers `TypeKinds` for codegen-side callers that hold a module's
//! declarations: `is_enum` from the declaration's kind (never "is a declared
//! type" — a record answering `true` would lose a user override), and
//! `union_accepts` for a user union's variants as well as a built-in package's.
//! A module type whose kind is none of record / enum / union / resource is a
//! lowering defect, refused by name rather than laid out by a guess.

use crate::codegen::builtins::TypeKinds;
use crate::codegen::engine::builder::TypeModel;
use crate::target::NativeBuildMode::Console;
use crate::testutil::{nir_for_src, CodeTarget};
use crate::types::ParameterType;

const SOURCE: &str = "\
IMPORT io

ENUM Color
  Red, Blue
END ENUM

TYPE Circle
  r AS Integer
END TYPE

TYPE Square
  side AS Integer
END TYPE

UNION Shape
  Circle
  Square
END UNION

FUNC area(s AS Shape) AS Integer
  MATCH s
    CASE Circle(c)
      RETURN c.r
    CASE Square(q)
      RETURN q.side
  END MATCH
END FUNC

FUNC main() AS Integer
  LET c AS Color = Color.Blue
  io::print(toString(area(Circle[r := 2])) & toString(c))
  RETURN 0
END FUNC
";

#[test]
fn the_type_model_answers_enum_kind_and_union_membership() {
    let module = nir_for_src(SOURCE, CodeTarget::LinuxX86_64, Console).expect("lowers to NIR");
    let model = TypeModel::from_module(&module).expect("the type model builds");
    let declared = ParameterType::declared;
    assert!(model.is_enum(&declared("Color")));
    assert!(
        !model.is_enum(&declared("Circle")),
        "a record is not an enum"
    );
    assert!(!model.is_enum(&ParameterType::Integer));
    assert!(model.union_accepts(&declared("Shape"), &declared("Circle")));
    assert!(model.union_accepts(&declared("Shape"), &declared("Square")));
    assert!(!model.union_accepts(&declared("Shape"), &declared("Color")));
    assert!(!model.union_accepts(&declared("Shape"), &ParameterType::Integer));
}

#[test]
fn a_module_type_of_an_unknown_kind_is_refused_by_name() {
    let mut module = nir_for_src(SOURCE, CodeTarget::LinuxX86_64, Console).expect("lowers to NIR");
    let square = module
        .types
        .iter_mut()
        .find(|type_| type_.name == "Square")
        .expect("Square is a module type");
    square.kind = "mystery".to_string();
    let Err(error) = TypeModel::from_module(&module) else {
        panic!("a type of kind 'mystery' was modelled");
    };
    assert!(
        error.contains("does not know type kind 'mystery'"),
        "{error}"
    );
}
