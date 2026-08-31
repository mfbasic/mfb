//! `collections::intersection` — descriptor entry + generic MFBASIC source body.
//!
//! A source-generic member in the csv/json `func_*.rs` shape: the registered
//! descriptor carries the docs and the generic `__collections_intersection` body
//! (`Body::Mfb`), which renders into the injected source and is instantiated by
//! the monomorphizer per call site. Body byte-significant (2-space indent →
//! `.ncode` columns); do not reformat.

const INTRO: &str = r#"Return the set of elements present in both of two sets"#;

const DESC: &str = r#"`collections::intersection` returns a new `Set OF T` holding exactly the elements
that are in both `a` and `b`. It walks the elements of `a` and keeps each one
that `collections::contains` reports as present in `b`, so an element only
survives when it appears in both sets.

`intersection` is **pure**: it returns a new value and mutates neither argument.
Surviving elements keep the insertion order they had in `a`. The intersection of
disjoint sets is the empty set, and the intersection of a set with itself is a
set equal to it.

`intersection` raises no user-trappable error of its own. Running out of memory is not a trappable domain error, and the `add` it is built on is classified
infallible.

`intersection` is a generic implemented in MFBASIC source; a call is rewritten to
the internal `__collections_intersection` generic and instantiated for the
element type like any other generic function.

Both arguments must be the same `Set OF T`. `T` is inferred from the element type
and **must be comparable**, which every `Set OF T` already requires. A call whose
arguments are not both sets of the same element type does not resolve and is
rejected at compile time."#;

const EX: &str = r#"Elements common to two sets:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET both AS Set OF Integer = collections::intersection(Set OF Integer { 1, 2, 3 }, Set OF Integer { 2, 3, 4 })
  io::print(toString(len(both)))
  RETURN 0
END FUNC
```

Disjoint sets intersect to the empty set:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET both AS Set OF Integer = collections::intersection(Set OF Integer { 1, 2 }, Set OF Integer { 3, 4 })
  io::print(toString(len(both)))
  RETURN 0
END FUNC
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"' Every element present in both `a` and `b`.
FUNC __collections_intersection OF T(a AS Set OF T, b AS Set OF T) AS Set OF T
  MUT result AS Set OF T = Set OF T { }
  FOR EACH x IN a
    IF collections::contains(b, x) THEN
      result = collections::add(result, x)
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
        name: "intersection",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Set OF T, Set OF T"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "a",
                    desc: "The first set, walked to decide element order. Not modified. `T` must be a comparable type.",
                    aliases: &[],
                    ty: ParameterType::set_of(ParameterType::var("T")),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "b",
                    desc: "The second set, of the same type as `a`, tested for membership. Not modified.",
                    aliases: &[],
                    ty: ParameterType::set_of(ParameterType::var("T")),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::set_of(ParameterType::var("T")),
            errors: vec![],
            body: Body::mfb(BODY, "__collections_intersection"),
        }],
    });
}
