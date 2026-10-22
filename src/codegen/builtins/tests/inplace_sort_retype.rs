//! The sort arms (`collection/assign/builder_inplace_sort.rs`: the in-place
//! `x = collections::sort(x)` / `sortBy(x, keyFn)` and bug-767's copying `sort` /
//! `sortBy` on a fresh copy) handed a module the front end would have rejected
//! (`nir_retype.rs`).
//!
//! The in-place arms DECLINE a binding that is not a list of an ordered element (the
//! copying lowering then refuses the type by name) and REFUSE a `sortBy` key function
//! that is not a function or returns a type with no `<`. The copying arms take the
//! list type and the key type from the value's static type, before lowering it; a
//! module whose global read disagrees with the global's declaration is refused once
//! the lowered type is known, rather than sorting a block at the wrong element width.

use super::nir_fault::walk_ops_mut;
use super::nir_retype::{list, lowered_with, refusal, retyped};
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;

const SRC: &str = r#"IMPORT collections
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

MUT gs AS List OF Float = [2.0, 1.0]
MUT kf AS FUNC(Float) AS Float = fident

FUNC so(xs AS List OF Integer) AS Integer
  MUT ys AS List OF Integer = xs
  ys = collections::sort(ys)
  RETURN len(ys)
END FUNC

FUNC sb(xs AS List OF Integer, k AS FUNC(Integer) AS Integer) AS Integer
  MUT ys AS List OF Integer = xs
  ys = collections::sortBy(ys, k)
  RETURN len(ys)
END FUNC

FUNC gso() AS Integer
  LET ys AS List OF Float = collections::sort(gs)
  RETURN len(ys)
END FUNC

FUNC gsb(xs AS List OF Float) AS Integer
  LET ys AS List OF Float = collections::sortBy(xs, kf)
  RETURN len(ys)
END FUNC

FUNC ident(n AS Integer) AS Integer
  RETURN n
END FUNC

FUNC fident(n AS Float) AS Float
  RETURN n
END FUNC

FUNC main() AS Integer
  io::print(toString(so([2, 1]) + sb([1], ident) + gso() + gsb([1.0])))
  RETURN 0
END FUNC
"#;

fn ty(name: &str) -> ParameterType {
    ParameterType::parse(name)
}

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

/// The in-place merge sorts a `List`: a `Set` is declined by both arms and the
/// copying lowering refuses it by name.
#[test]
fn sort_and_sort_by_decline_a_set() {
    let set = ty("Set OF Integer");
    let err = binding_as("so", set.clone());
    assert!(
        err.contains("native sort does not accept Set OF Integer"),
        "sort over a Set: {err}"
    );
    let err = binding_as("sb", set);
    assert!(
        err.contains("native sortBy does not accept Set OF Integer"),
        "sortBy over a Set: {err}"
    );
}

/// An element with no `<` (a `Boolean`, a union, a nested list) is not sorted in
/// place; the copying lowering refuses the item type.
#[test]
fn sort_declines_an_unordered_element() {
    for element in ["Boolean", "Tree", "List OF Integer"] {
        let err = binding_as("so", list(element));
        assert!(
            err.contains(&format!("native sort does not support item type {element}")),
            "sort over List OF {element}: {err}"
        );
    }
}

/// `sortBy`'s key function must be a function, and its keys must have `<`.
#[test]
fn sort_by_refuses_a_key_function_it_cannot_order_by() {
    let err = refusal(
        retyped(SRC, "sb", &[("k", ParameterType::Integer)]),
        "sortBy with an Integer key function",
    );
    assert!(
        err.contains("native in-place sortBy key function must be a function, got Integer"),
        "{err}"
    );
    for key in ["Boolean", "Tree"] {
        let k = ParameterType::func(vec![ParameterType::Integer], ty(key));
        let err = refusal(
            retyped(SRC, "sb", &[("k", k)]),
            &format!("sortBy with {key} keys"),
        );
        assert!(
            err.contains(&format!(
                "native in-place sortBy: keys of type {key} have no `<`"
            )),
            "{err}"
        );
    }
}

/// Re-declare every global read in `func` as `type_`.
fn global_read_as(func: &str, type_: ParameterType) -> String {
    refusal(
        lowered_with(SRC, func, |function| {
            walk_ops_mut(&mut function.body, &mut |_| {}, &mut |value| {
                if let NirValue::Global { type_: read, .. } = value {
                    *read = type_.clone();
                }
            });
        }),
        &format!("{func} with a global read as {type_}"),
    )
}

/// The copying `sort` sizes its merge from the value's static type; a global read
/// that claims `List OF Byte` over a `List OF Float` global is refused once lowered.
#[test]
fn the_copying_sort_refuses_a_value_that_lowers_to_another_list() {
    let err = global_read_as("gso", list("Byte"));
    assert!(
        err.contains("native copying sort: value lowered as List OF Float, expected List OF Byte"),
        "{err}"
    );
}

/// The copying `sortBy` orders by the key function's static return type; a key
/// function whose read claims `Byte` keys over a `Float`-returning global is refused
/// once lowered.
#[test]
fn the_copying_sort_by_refuses_a_key_function_that_lowers_to_another_key() {
    let err = global_read_as(
        "gsb",
        ParameterType::func(vec![ParameterType::Float], ParameterType::Byte),
    );
    assert!(
        err.contains(
            "native copying sortBy: keyFn lowered as FUNC(Float) AS Float, expected a \
             function returning Byte"
        ),
        "{err}"
    );
}
