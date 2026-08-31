//! `collections::union` — descriptor entry + generic MFBASIC source body.
//!
//! A source-generic member in the csv/json `func_*.rs` shape: the registered
//! descriptor carries the docs and the generic `__collections_union` body
//! (`Body::Mfb`), which renders into the injected source and is instantiated by
//! the monomorphizer per call site. Body byte-significant (2-space indent →
//! `.ncode` columns); do not reformat.

const INTRO: &str = r#"Return the set of elements present in either of two sets"#;

const DESC: &str = r#"`collections::union` returns a new `Set OF T` holding every element that is in
`a`, in `b`, or in both. It starts from the elements of `a` and adds each element
of `b`; because `collections::add` is idempotent, an element already present is
not duplicated, so the result contains each distinct element exactly once.

`union` is **pure**: it returns a new value and mutates neither argument. Element
insertion order follows the elements of `a` first, then the elements of `b` that
were not already in `a`. The union of a set with the empty set is a copy of that
set, and the union of two equal sets is a set equal to either one.

`union` raises no user-trappable error of its own. While building the result it needs memory, but running out of memory is not a trappable domain error, and the
`add` it is built on is classified infallible for exactly that reason.

`union` is a generic implemented in MFBASIC source; a call is rewritten to the
internal `__collections_union` generic and instantiated for the element type like
any other generic function.

Both arguments must be the same `Set OF T`. `T` is inferred from the element type
and **must be comparable**, which every `Set OF T` already requires. A call whose
arguments are not both sets of the same element type does not resolve and is
rejected at compile time."#;

const EX: &str = r#"Combine two sets:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET u AS Set OF Integer = collections::union(Set OF Integer { 1, 2 }, Set OF Integer { 2, 3 })
  io::print(toString(len(u)))
  RETURN 0
END FUNC
```

Union with an empty set is a copy:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET u AS Set OF Integer = collections::union(Set OF Integer { 4, 5 }, Set OF Integer { })
  io::print(toString(len(u)))
  RETURN 0
END FUNC
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"' Every element of `a` and `b` (add is idempotent, so duplicates collapse).
FUNC __collections_union OF T(a AS Set OF T, b AS Set OF T) AS Set OF T
  MUT result AS Set OF T = a
  FOR EACH x IN b
    result = collections::add(result, x)
  NEXT
  RETURN result
END FUNC"#;

pub(crate) fn register(pkg: &mut crate::codegen::registry::RegistryPackage) {
    use crate::codegen::registry::{
        Body, DefaultValue, Implementation, Parameter, RegistryFunction,
    };
    use crate::types::ParameterType;

    pkg.add_function(RegistryFunction {
        name: "union",
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
            body: Body::mfb(BODY, "__collections_union"),
        }],
    });
}
