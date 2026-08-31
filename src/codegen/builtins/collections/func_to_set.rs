//! `collections::toSet` — descriptor entry + generic MFBASIC source body.
//!
//! A source-generic member in the csv/json `func_*.rs` shape: the registered
//! descriptor carries the docs and the generic `__collections_toSet` body
//! (`Body::Mfb`), which renders into the injected source and is instantiated by
//! the monomorphizer per call site. Body byte-significant (2-space indent →
//! `.ncode` columns); do not reformat.

const INTRO: &str = r#"Build a set from the distinct elements of a list"#;

const DESC: &str = r#"`collections::toSet` returns a new `Set OF T` containing every distinct element
of the list `value`. It folds over `value` in order and adds each element to a
fresh set; because `collections::add` is idempotent, a repeated element is stored
only once, so the result holds each distinct element exactly once.

`toSet` is **pure**: it returns a new value and does not mutate `value`. Element
insertion order follows first occurrence in the list, so `toSet([2, 1, 2, 3])`
holds `2`, `1`, `3` in that order. Converting a list that is already free of
duplicates yields a set with the same elements; converting the empty list yields
the empty set.

`toSet` raises no user-trappable error of its own. While building the result it needs memory, but running out of memory is not a trappable domain error, and the `add` it
is built on is classified infallible for exactly that reason.

`toSet` is a generic implemented in MFBASIC source; a call is rewritten to the
internal `__collections_toSet` generic and instantiated for the element type like
any other generic function.

The argument must be a `List OF T` whose element type `T` is comparable (every
`Set OF T` requires it). A call on a non-list argument, or on a list whose element
type is not comparable, does not resolve and is rejected at compile time."#;

const EX: &str = r#"Collapse a list's duplicates into a set:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET s AS Set OF Integer = collections::toSet([5, 5, 6, 7, 6])
  io::print(toString(len(s)))
  RETURN 0
END FUNC
```

Round-trip a set through a list and back:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET original AS Set OF String = Set OF String { "a", "b", "c" }
  LET roundTripped AS Set OF String = collections::toSet(collections::toList(original))
  io::print(toString(len(roundTripped)))
  RETURN 0
END FUNC
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"' Build a set from a list's distinct elements (first-occurrence order).
FUNC __collections_toSet OF T(value AS List OF T) AS Set OF T
  MUT result AS Set OF T = Set OF T { }
  FOR EACH x IN value
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
        name: "toSet",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("List OF T"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "The list to draw elements from. Not modified. `T` must be a comparable type, since a `Set OF T` requires a comparable element.",
                    aliases: &[],
                    ty: ParameterType::list_of(ParameterType::var("T")),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::set_of(ParameterType::var("T")),
            errors: vec![],
            body: Body::mfb(BODY, "__collections_toSet"),
        }],
    });
}
