//! `collections::difference` — descriptor entry + generic MFBASIC source body.
//!
//! A source-generic member in the csv/json `func_*.rs` shape: the registered
//! descriptor carries the docs and the generic `__collections_difference` body
//! (`Body::Mfb`), which renders into the injected source and is instantiated by
//! the monomorphizer per call site. Body byte-significant (2-space indent →
//! `.ncode` columns); do not reformat.

const INTRO: &str = r#"Return the set of elements in the first set but not the second"#;

const DESC: &str = r#"`collections::difference` returns a new `Set OF T` holding the elements that are
in `a` but not in `b`. It walks the elements of `a` and keeps each one that
`collections::contains` reports as **absent** from `b`, so the result is `a` with
every element of `b` removed. The operation is asymmetric:
`difference(a, b)` and `difference(b, a)` are generally different sets.

`difference` is **pure**: it returns a new value and mutates neither argument.
Surviving elements keep the insertion order they had in `a`. The difference of a
set and the empty set is a copy of that set; the difference of a set with itself
is the empty set.

`difference` raises no user-trappable error of its own. Running out of memory is not a trappable domain error, and the `add` it is built on is classified infallible.

`difference` is a generic implemented in MFBASIC source; a call is rewritten to
the internal `__collections_difference` generic and instantiated for the element
type like any other generic function.

Both arguments must be the same `Set OF T`. `T` is inferred from the element type
and **must be comparable**, which every `Set OF T` already requires. A call whose
arguments are not both sets of the same element type does not resolve and is
rejected at compile time."#;

const EX: &str = r#"Elements of the first set not in the second:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET only AS Set OF Integer = collections::difference(Set OF Integer { 1, 2, 3 }, Set OF Integer { 2 })
  io::print(toString(len(only)))
  RETURN 0
END FUNC
```

Difference is asymmetric:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET d AS Set OF Integer = collections::difference(Set OF Integer { 2 }, Set OF Integer { 1, 2, 3 })
  io::print(toString(len(d)))
  RETURN 0
END FUNC
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"' Every element of `a` not present in `b`.
FUNC __collections_difference OF T(a AS Set OF T, b AS Set OF T) AS Set OF T
  MUT result AS Set OF T = Set OF T { }
  FOR EACH x IN a
    IF NOT collections::contains(b, x) THEN
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
        name: "difference",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Set OF T, Set OF T"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "a",
                    desc: "The set to subtract from, walked to decide element order. Not modified. `T` must be a comparable type.",
                    aliases: &[],
                    ty: ParameterType::set_of(ParameterType::var("T")),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "b",
                    desc: "The set whose elements are removed from `a`, of the same type as `a`. Not modified.",
                    aliases: &[],
                    ty: ParameterType::set_of(ParameterType::var("T")),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::set_of(ParameterType::var("T")),
            errors: vec![],
            body: Body::mfb(BODY, "__collections_difference"),
        }],
    });
}
