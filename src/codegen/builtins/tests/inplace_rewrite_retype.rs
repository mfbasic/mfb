//! The length-keeping in-place arms (`collection/assign/builder_inplace_rewrite.rs`:
//! `x = math::f(x)`, `x = collections::replace(x, old, new)`,
//! `x = collections::transform(x, f)`) handed a function the front end would have
//! rejected, retyped consistently in NIR (`nir_retype.rs`).
//!
//! Two contracts. An arm whose gate does not hold for the binding's type (a `Set`
//! where it rewrites a `List`, a `List OF String` under element-wise `math`)
//! DECLINES, and the statement goes to the copying lowering, which refuses the type
//! by name. An arm whose gate holds but whose operand the checker would have stopped
//! (an incomparable `replace` element, an element type nothing declares, a
//! `transform` callback that is not a function or maps to another element type)
//! REFUSES with a sentence instead of comparing pointer words or writing the wrong
//! element width.

use super::nir_retype::{list, lowered_with, refusal, retyped};
use crate::types::ParameterType;

const SRC: &str = r#"IMPORT collections
IMPORT io
IMPORT math

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

FUNC upd(xs AS List OF Integer, a AS Integer, b AS Integer) AS Integer
  MUT ys AS List OF Integer = xs
  ys = collections::replace(ys, a, b)
  RETURN len(ys)
END FUNC

FUNC tr(xs AS List OF Integer, f AS FUNC(Integer) AS Integer) AS Integer
  MUT ys AS List OF Integer = xs
  ys = collections::transform(ys, f)
  RETURN len(ys)
END FUNC

FUNC trp(xs AS List OF Integer) AS Integer
  MUT ys AS List OF Integer = xs
  ys = collections::transform(ys, pick())
  RETURN len(ys)
END FUNC

FUNC pick() AS FUNC(Integer) AS Integer
  RETURN twice
END FUNC

FUNC mth(xs AS List OF Float) AS Integer
  MUT ys AS List OF Float = xs
  ys = math::abs(ys)
  RETURN len(ys)
END FUNC

FUNC twice(n AS Integer) AS Integer
  RETURN n * 2
END FUNC

FUNC main() AS Integer
  io::print(toString(upd([1, 2], 1, 3) + tr([1], twice) + trp([2]) + mth([1.0])))
  RETURN 0
END FUNC
"#;

fn ty(name: &str) -> ParameterType {
    ParameterType::parse(name)
}

/// `ys` and its source `xs` re-declared as `collection`.
fn binding_as(func: &str, collection: ParameterType) -> String {
    refusal(
        retyped(
            SRC,
            func,
            &[("xs", collection.clone()), ("ys", collection.clone())],
        ),
        &format!("{func} over {collection}"),
    )
}

/// `math` element-wise over a `Set` or over a `List` of a non-numeric element: the
/// in-place arm declines (it rewrites 8-byte list lanes only) and the copying
/// lowering names the type it does not accept.
#[test]
fn math_declines_a_set_and_a_non_numeric_list() {
    let err = binding_as("mth", ty("Set OF Float"));
    assert!(
        err.contains("math.abs does not accept Set OF Float"),
        "a Set is not rewritten in place: {err}"
    );
    let err = binding_as("mth", list("String"));
    assert!(
        err.contains("math.abs array overload does not accept List OF String"),
        "a String list has no 8-byte lanes: {err}"
    );
}

/// `replace` and `transform` rewrite a `List`'s elements; over a `Set` both arms
/// decline and the copying lowering refuses the collection.
#[test]
fn replace_and_transform_decline_a_set() {
    let set = ty("Set OF Integer");
    let err = binding_as("upd", set.clone());
    assert!(
        err.contains("got Set OF Integer"),
        "replace over a Set: {err}"
    );
    let err = binding_as("tr", set);
    assert!(
        err.contains("native collection transform does not accept Set OF Integer"),
        "transform over a Set: {err}"
    );
}

/// `replace` compares elements; an incomparable element (a nested collection, a
/// union, a function) is refused by the element compare, by type name, and an
/// element type no declaration defines is refused by the payload sizing.
#[test]
fn replace_refuses_an_element_it_cannot_compare_or_size() {
    for element in [
        "List OF String",
        "Shape",
        "FUNC(Integer) AS Integer",
        "Nope",
    ] {
        let err = refusal(
            retyped(
                SRC,
                "upd",
                &[
                    ("xs", list(element)),
                    ("ys", list(element)),
                    ("a", ty(element)),
                    ("b", ty(element)),
                ],
            ),
            &format!("replace over List OF {element}"),
        );
        assert!(
            err.contains(&format!(
                "native collection packed payload does not support type '{element}'"
            )),
            "replace over List OF {element}: {err}"
        );
    }
}

/// `transform`'s callback must be a function returning the list's element type.
#[test]
fn transform_refuses_a_callback_that_is_not_a_function() {
    let err = refusal(
        retyped(SRC, "tr", &[("f", ParameterType::Integer)]),
        "transform with an Integer callback",
    );
    assert!(
        err.contains("native in-place transform action must be a function, got Integer"),
        "{err}"
    );
}

/// A callback whose static type is unknown before lowering (a call returning a
/// function) is checked once lowered: mapping to another element type is refused,
/// since the arm writes results over `x`'s own elements.
#[test]
fn transform_refuses_a_callback_mapping_to_another_element_type() {
    let err = refusal(
        lowered_with(SRC, "pick", |function| {
            function.returns = ParameterType::func(vec![ParameterType::Integer], ty("String"));
        }),
        "transform with a callback returning String",
    );
    assert!(
        err.contains(
            "native in-place transform of List OF Integer needs a FUNC returning Integer, \
             got String"
        ),
        "{err}"
    );
}
