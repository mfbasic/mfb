//! `collections::drop` — descriptor entry + generic MFBASIC source body.
//!
//! A source-generic member in the csv/json `func_*.rs` shape: the registered
//! descriptor carries the docs and the generic `__collections_drop` body
//! (`Body::Mfb`), which renders into the injected source and is instantiated by
//! the monomorphizer per call site. Body byte-significant (2-space indent →
//! `.ncode` columns); do not reformat.

const INTRO: &str = r#"Return a new list with the first `count` elements removed"#;

const DESC: &str = r#"`collections::drop` returns a new list containing everything in `value` except
its leading `count` elements, in their original order.`drop(value, count)` is defined as the half-open range `[count, len(value))` of
`value`, delegated to the internal slice helper. That helper is lowered natively
as a bulk range copy, and that is what defines the boundary
behavior: the range start is clamped into `[0, len]` and the range stop into
`[start, len]`.

That clamping makes `drop` **total** — every `Integer` value of `count` is
accepted and no index is ever rejected:

- `count` of 0 or any negative value clamps the start back to 0, so the whole
  list is returned.
- `count` greater than or equal to the length of `value` clamps the start to the
  length, so the result is the empty list.
- Otherwise the result holds `len(value) - count` elements.

The result is a new list; the elements are copied into it, so
the returned list does not share storage with `value`. `value` is not modified.
`collections::take` is the complementary operation, returning the elements
`drop` discards.

`T` is inferred from `value` and carries no ordering or comparability
requirement: `drop` copies a contiguous range and never inspects an element, so
any list element type is accepted. `count` must be `Integer`."#;

const EX: &str = r#"Discard the first two elements:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET tail AS List OF Integer = collections::drop([1, 2, 3, 4], 2)
  io::print(toString(len(tail)))
  RETURN 0
END FUNC
```

An oversized count yields the empty list; a non-positive count yields the whole
list:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET none AS List OF Integer = collections::drop([1, 2, 3], 99)
  LET all AS List OF Integer = collections::drop([1, 2, 3], 0)
  io::print(toString(len(none)))
  io::print(toString(len(all)))
  RETURN 0
END FUNC
```

Skip a header row before processing the rest:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET rows AS List OF String = ["name", "ada", "grace"]
  LET body AS List OF String = collections::drop(rows, 1)
  io::print(collections::get(body, 0))
  RETURN 0
END FUNC
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __collections_drop OF T(value AS List OF T, count AS Integer) AS List OF T
  RETURN __collections_slice(value, count, len(value))
END FUNC"#;

pub(crate) fn register(pkg: &mut crate::codegen::registry::RegistryPackage) {
    use crate::codegen::registry::{
        Body, DefaultValue, Implementation, Parameter, RegistryFunction,
    };
    use crate::types::ParameterType;

    pkg.add_function(RegistryFunction {
        name: "drop",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("List OF T, Integer"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "The source list. Any length is accepted, including the empty list. Named-argument spelling is `value`.",
                    aliases: &[],
                    ty: ParameterType::list_of(ParameterType::var("T")),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "count",
                    desc: "How many leading elements to discard. Any `Integer` is accepted: values at or below 0 return the whole list and values at or above the length return the empty list. Named-argument spelling is `count`.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::list_of(ParameterType::var("T")),
            errors: vec![],
            body: Body::mfb(BODY, "__collections_drop"),
        }],
    });
}
