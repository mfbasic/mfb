//! `collections::symmetricDifference` — descriptor entry + generic MFBASIC source body.
//!
//! A source-generic member in the csv/json `func_*.rs` shape: the registered
//! descriptor carries the docs and the generic `__collections_symmetricDifference` body
//! (`Body::Mfb`), which renders into the injected source and is instantiated by
//! the monomorphizer per call site. Body byte-significant (2-space indent →
//! `.ncode` columns); do not reformat.

const INTRO: &str = r#"Return the set of elements in exactly one of two sets"#;

const DESC: &str = r#"`collections::symmetricDifference` returns a new `Set OF T` holding the elements
that are in exactly one of `a` and `b` — every element of their union that is not
in their intersection. It is computed as a two-pass fold: it keeps each element of
`a` that `collections::contains` reports as absent from `b`, then adds each
element of `b` that is absent from `a`. Unlike `difference`, the operation is
symmetric: `symmetricDifference(a, b)` and `symmetricDifference(b, a)` are equal.

`symmetricDifference` is **pure**: it returns a new value and mutates neither
argument. Element insertion order follows the surviving elements of `a` first,
then the surviving elements of `b`. The symmetric difference of two equal sets is
the empty set, and of a set with the empty set is a copy of that set.

`symmetricDifference` raises no user-trappable error of its own. Running out of memory is not a trappable domain error, and the `add` it is built on is
classified infallible.

`symmetricDifference` is a generic implemented in MFBASIC source; a call is
rewritten to the internal `__collections_symmetricDifference` generic and
instantiated for the element type like any other generic function.

Both arguments must be the same `Set OF T`. `T` is inferred from the element type
and **must be comparable**, which every `Set OF T` already requires. A call whose
arguments are not both sets of the same element type does not resolve and is
rejected at compile time."#;

const EX: &str = r#"Elements in exactly one of two sets:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET d AS Set OF Integer = collections::symmetricDifference(Set OF Integer { 1, 2, 3 }, Set OF Integer { 2, 3, 4 })
  io::print(toString(len(d)))
  RETURN 0
END FUNC
```

Two equal sets have an empty symmetric difference:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET d AS Set OF Integer = collections::symmetricDifference(Set OF Integer { 1, 2 }, Set OF Integer { 1, 2 })
  io::print(toString(len(d)))
  RETURN 0
END FUNC
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"' Every element in exactly one of `a` or `b` (an inline two-pass fold).
FUNC __collections_symmetricDifference OF T(a AS Set OF T, b AS Set OF T) AS Set OF T
  MUT result AS Set OF T = Set OF T { }
  FOR EACH x IN a
    IF NOT collections::contains(b, x) THEN
      result = collections::add(result, x)
    END IF
  NEXT
  FOR EACH y IN b
    IF NOT collections::contains(a, y) THEN
      result = collections::add(result, y)
    END IF
  NEXT
  RETURN result
END FUNC"#;

pub(crate) fn register(pkg: &mut crate::codegen::registry::RegistryPackage) {
    use crate::codegen::registry::{
        Body, DefaultValue, Implementation, Parameter, RegistryFunction,
    };
    use crate::types::ParameterType;

    pkg.add_function(RegistryFunction {
        name: "symmetricDifference",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Set OF T, Set OF T"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "a",
                    desc: "The first set. Not modified. `T` must be a comparable type.",
                    aliases: &[],
                    ty: ParameterType::set_of(ParameterType::var("T")),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "b",
                    desc: "The second set, of the same type as `a`. Not modified.",
                    aliases: &[],
                    ty: ParameterType::set_of(ParameterType::var("T")),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::set_of(ParameterType::var("T")),
            errors: vec![],
            body: Body::mfb(BODY, "__collections_symmetricDifference"),
        }],
    });
}
