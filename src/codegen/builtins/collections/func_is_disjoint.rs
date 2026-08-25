//! `collections::isDisjoint` — descriptor entry + generic MFBASIC source body.
//!
//! A source-generic member in the csv/json `func_*.rs` shape: the registered
//! descriptor carries the docs and the generic `__collections_isDisjoint` body
//! (`Body::Mfb`), which renders into the injected source and is instantiated by
//! the monomorphizer per call site. Body byte-significant (2-space indent →
//! `.ncode` columns); do not reformat.

const INTRO: &str = r#"Test whether two sets share no element"#;

const DESC: &str = r#"`collections::isDisjoint` returns `TRUE` when `a` and `b` have no element in
common, and `FALSE` otherwise. It walks the elements of `a` and returns `FALSE`
as soon as `collections::contains` reports one that is also in `b`; if the walk
finds no shared element, it returns `TRUE`. Equivalently, two sets are disjoint
exactly when their intersection is empty.

`isDisjoint` is **pure**: it inspects both arguments and mutates neither. The
empty set is disjoint from every set, so a call with an empty argument is always
`TRUE`. The relation is symmetric: `isDisjoint(a, b)` equals `isDisjoint(b, a)`.

`isDisjoint` raises no user-trappable error of its own.

`isDisjoint` is a generic implemented in MFBASIC source; a call is rewritten to
the internal `__collections_isDisjoint` generic and instantiated for the element
type like any other generic function.

Both arguments must be the same `Set OF T`. `T` is inferred from the element type
and **must be comparable**, which every `Set OF T` already requires. A call whose
arguments are not both sets of the same element type does not resolve and is
rejected at compile time."#;

const EX: &str = r#"Two sets with no common element:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET yes AS Boolean = collections::isDisjoint(Set OF Integer { 1, 2 }, Set OF Integer { 3, 4 })
  io::print(toString(yes))
  RETURN 0
END FUNC
```

A shared element makes it false:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET no AS Boolean = collections::isDisjoint(Set OF Integer { 1, 2 }, Set OF Integer { 2, 3 })
  io::print(toString(no))
  RETURN 0
END FUNC
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"' TRUE iff `a` and `b` share no element.
FUNC __collections_isDisjoint OF T(a AS Set OF T, b AS Set OF T) AS Boolean
  FOR EACH x IN a
    IF collections::contains(b, x) THEN
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
        name: "isDisjoint",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Set OF T, Set OF T"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "a",
                    desc: "The first set, walked element by element. Not modified. `T` must be a comparable type.",
                    aliases: &[],
                    ty: ParameterType::set_of(ParameterType::var("T")),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "b",
                    desc: "The second set, of the same type as `a`, tested for shared membership. Not modified.",
                    aliases: &[],
                    ty: ParameterType::set_of(ParameterType::var("T")),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::mfb(BODY, "__collections_isDisjoint"),
        }],
    });
}
