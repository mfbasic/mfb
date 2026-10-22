//! The record-field self-update arms (`collection/assign/builder_inplace_assign/field.rs`:
//! `r = WITH r { f := op(r.f, …) }`, growing the owner's block through `InlineGrow`)
//! handed a module the front end would have rejected (`nir_retype.rs`).
//!
//! Each arm lowers its operands and checks them against the FIELD's declared type
//! before it writes into the owner's block; a key, value or index of another type is
//! refused by name, and a field whose declared collection is not one `set` can update
//! is declined and refused by the copying lowering.

use super::nir_retype::{lowered_module_with, refusal, retype_field, retyped};
use crate::types::ParameterType;

const SRC: &str = r#"IMPORT collections
IMPORT io

TYPE LBox
  n AS Integer
  xs AS List OF Integer
END TYPE

TYPE MBox
  n AS Integer
  mp AS Map OF String TO Integer
END TYPE

FUNC fld(k AS String, v AS Integer, i AS Integer, w AS Integer) AS Integer
  MUT r AS LBox = LBox[1, [1, 2]]
  r = WITH r { xs := collections::insert(r.xs, i, w) }
  MUT q AS MBox = MBox[1, Map OF String TO Integer { "a" := 1 }]
  q = WITH q { mp := collections::set(q.mp, k, v) }
  RETURN len(r.xs) + len(q.mp)
END FUNC

FUNC main() AS Integer
  io::print(toString(fld("k", 1, 0, 2)))
  RETURN 0
END FUNC
"#;

fn operand_as(name: &str, type_: ParameterType) -> String {
    refusal(
        retyped(SRC, "fld", &[(name, type_.clone())]),
        &format!("`{name}` as {type_}"),
    )
}

/// A field `Map`'s `set` checks the key and the value against the field's types.
#[test]
fn a_field_map_set_refuses_a_key_or_value_of_another_type() {
    let err = operand_as("k", ParameterType::Integer);
    assert!(
        err.contains("native collection set map key must be String, got Integer"),
        "{err}"
    );
    let err = operand_as("v", ParameterType::String);
    assert!(
        err.contains("native collection set map value must be Integer, got String"),
        "{err}"
    );
}

/// A field `List`'s `insert` index must be an `Integer`.
#[test]
fn a_field_list_insert_refuses_a_non_integer_index() {
    let err = operand_as("i", ParameterType::String);
    assert!(
        err.contains("native collection insert index must be Integer, got String"),
        "{err}"
    );
}

/// `set` at a field updates a `List` or a `Map`; a field declared as a `Set` is
/// declined, and the copying lowering refuses it.
#[test]
fn a_field_set_declines_a_set_field() {
    let err = refusal(
        lowered_module_with(SRC, |module| {
            retype_field(module, "MBox", "mp", ParameterType::parse("Set OF String"))
        }),
        "set on a Set field",
    );
    assert!(
        err.contains("native collection set does not accept Set OF String"),
        "{err}"
    );
}
