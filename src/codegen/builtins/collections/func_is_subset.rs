//! `collections::isSubset` — descriptor entry + generic MFBASIC source body.
//!
//! A source-generic member in the csv/json `func_*.rs` shape: the registered
//! descriptor carries the docs and the generic `__collections_isSubset` body
//! (`Body::Mfb`), which renders into the injected source and is instantiated by
//! the monomorphizer per call site. Body byte-significant (2-space indent →
//! `.ncode` columns); do not reformat.

const INTRO: &str = r#"Test whether every element of the first set is in the second"#;

const DESC: &str = r#"`collections::isSubset` returns `TRUE` when every element of `a` is also in `b`,
and `FALSE` otherwise. It walks the elements of `a` and returns `FALSE` as soon as
`collections::contains` reports one that is absent from `b`; if the walk finishes
with no such element, it returns `TRUE`.

`isSubset` is **pure**: it inspects both arguments and mutates neither. The empty
set is a subset of every set, so `isSubset(Set OF T { }, b)` is always `TRUE`. A
set is a subset of itself, and equal sets are subsets of each other.

`isSubset` raises no user-trappable error of its own.

`isSubset` is a generic implemented in MFBASIC source; a call is rewritten to the
internal `__collections_isSubset` generic and instantiated for the element type
like any other generic function.

Both arguments must be the same `Set OF T`. `T` is inferred from the element type
and **must be comparable**, which every `Set OF T` already requires. A call whose
arguments are not both sets of the same element type does not resolve and is
rejected at compile time."#;

const EX: &str = r#"A smaller set contained in a larger one:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET yes AS Boolean = collections::isSubset(Set OF Integer { 1, 2 }, Set OF Integer { 1, 2, 3 })
  io::print(toString(yes))
  RETURN 0
END FUNC
```

An element outside the second set makes it false:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET no AS Boolean = collections::isSubset(Set OF Integer { 1, 9 }, Set OF Integer { 1, 2, 3 })
  io::print(toString(no))
  RETURN 0
END FUNC
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"' TRUE iff every element of `a` is in `b`.
FUNC __collections_isSubset OF T(a AS Set OF T, b AS Set OF T) AS Boolean
  FOR EACH x IN a
    IF NOT collections::contains(b, x) THEN
      RETURN FALSE
    END IF
  NEXT
  RETURN TRUE
END FUNC"#;

pub(crate) fn register(pkg: &mut crate::codegen::registry::RegistryPackage) {
    use crate::codegen::registry::{
        Body, DefaultValue, Implementation, Parameter, RegistryFunction,
    };
    use crate::types::ParameterType;

    pkg.add_function(RegistryFunction {
        name: "isSubset",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Set OF T, Set OF T"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "a",
                    desc: "The candidate subset, whose elements are each tested for membership in `b`. Not modified. `T` must be a comparable type.",
                    aliases: &[],
                    ty: ParameterType::set_of(ParameterType::var("T")),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "b",
                    desc: "The candidate superset, of the same type as `a`. Not modified.",
                    aliases: &[],
                    ty: ParameterType::set_of(ParameterType::var("T")),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::mfb(BODY, "__collections_isSubset"),
        }],
    });
}
