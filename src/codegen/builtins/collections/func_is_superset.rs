//! `collections::isSuperset` — descriptor entry + generic MFBASIC source body.
//!
//! A source-generic member in the csv/json `func_*.rs` shape: the registered
//! descriptor carries the docs and the generic `__collections_isSuperset` body
//! (`Body::Mfb`), which renders into the injected source and is instantiated by
//! the monomorphizer per call site. Body byte-significant (2-space indent →
//! `.ncode` columns); do not reformat.

const INTRO: &str = r#"Test whether the first set contains every element of the second"#;

const DESC: &str = r#"`collections::isSuperset` returns `TRUE` when every element of `b` is also in
`a`, and `FALSE` otherwise. It is `isSubset` with the arguments swapped: it walks
the elements of `b` and returns `FALSE` as soon as `collections::contains` reports
one that is absent from `a`, returning `TRUE` if the walk finds no such element.

`isSuperset` is **pure**: it inspects both arguments and mutates neither. Every
set is a superset of the empty set, so `isSuperset(a, Set OF T { })` is always
`TRUE`. A set is a superset of itself, and equal sets are supersets of each other.

`isSuperset` raises no user-trappable error of its own.Both arguments must be the same `Set OF T`. `T` is inferred from the element type
and **must be comparable**, which every `Set OF T` already requires. A call whose
arguments are not both sets of the same element type does not resolve and is
rejected at compile time."#;

const EX: &str = r#"A larger set containing a smaller one:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET yes AS Boolean = collections::isSuperset(Set OF Integer { 1, 2, 3 }, Set OF Integer { 1, 2 })
  io::print(toString(yes))
  RETURN 0
END FUNC
```

A missing element makes it false:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET no AS Boolean = collections::isSuperset(Set OF Integer { 1, 2, 3 }, Set OF Integer { 1, 9 })
  io::print(toString(no))
  RETURN 0
END FUNC
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"' TRUE iff every element of `b` is in `a` (isSubset with arguments swapped).
FUNC __collections_isSuperset OF T(a AS Set OF T, b AS Set OF T) AS Boolean
  FOR EACH x IN b
    IF NOT collections::contains(a, x) THEN
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
        name: "isSuperset",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Set OF T, Set OF T"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "a",
                    desc: "The candidate superset, tested against every element of `b`. Not modified. `T` must be a comparable type.",
                    aliases: &[],
                    ty: ParameterType::set_of(ParameterType::var("T")),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "b",
                    desc: "The candidate subset, of the same type as `a`, whose elements are each tested for membership in `a`. Not modified.",
                    aliases: &[],
                    ty: ParameterType::set_of(ParameterType::var("T")),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::mfb(BODY, "__collections_isSuperset"),
        }],
    });
}
