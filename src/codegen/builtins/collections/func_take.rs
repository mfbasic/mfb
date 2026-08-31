//! `collections::take` — descriptor entry + generic MFBASIC source body.
//!
//! A source-generic member in the csv/json `func_*.rs` shape: the registered
//! descriptor carries the docs and the generic `__collections_take` body
//! (`Body::Mfb`), which renders into the injected source and is instantiated by
//! the monomorphizer per call site. Body byte-significant (2-space indent →
//! `.ncode` columns); do not reformat.

const INTRO: &str = r#"Return a new list holding the first `count` elements of a list"#;

const DESC: &str = r#"`collections::take` returns a new list containing the leading `count` elements
of `value`, in their original order.`take(value, count)` is defined as the half-open range `[0, count)` of `value`,
delegated to the internal slice helper. That helper is lowered natively as a
bulk range copy, and that is what defines the boundary behavior:
the range start is clamped into `[0, len]` and the range stop into
`[start, len]`.

That clamping makes `take` **total** — every `Integer` value of `count` is
accepted and no index is ever rejected:

- `count` of 0 or any negative value clamps the stop back to the start, so the
  result is the empty list.
- `count` greater than or equal to the length of `value` clamps the stop to the
  length, so the whole list is returned.
- Otherwise the result holds exactly `count` elements.

The result is a new list; the elements are copied into it, so
the returned list does not share storage with `value`. `value` is not modified.
`collections::drop` is the complementary operation, returning what `take` leaves
behind.

`T` is inferred from `value` and carries no ordering or comparability
requirement: `take` copies a contiguous range and never inspects an element, so
any list element type is accepted. `count` must be `Integer`."#;

const EX: &str = r#"Keep the first two elements:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET head AS List OF Integer = collections::take([1, 2, 3, 4], 2)
  io::print(toString(len(head)))
  RETURN 0
END FUNC
```

An oversized count yields the whole list; a non-positive count yields an empty
one:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET all AS List OF Integer = collections::take([1, 2, 3], 99)
  LET none AS List OF Integer = collections::take([1, 2, 3], 0)
  io::print(toString(len(all)))
  io::print(toString(len(none)))
  RETURN 0
END FUNC
```

Split a list into a head and a tail with `take` and `drop`:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET items AS List OF String = ["a", "b", "c", "d"]
  LET first AS List OF String = collections::take(items, 2)
  LET rest AS List OF String = collections::drop(items, 2)
  io::print(collections::get(rest, 0))
  RETURN 0
END FUNC
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"' bug-339 C9: take and drop are the two halves of one API — kept together here
' under the §6.4 banner (take used to sit above the internal-helpers banner).
' plan-39 A4: both are contiguous ranges delegating to __collections_slice, which
' is natively lowered as a bulk range copy; the native path clamps start into
' [0, len] and stop into [start, len], reproducing the old element-by-element
' bounds exactly: take(count) = slice(0, count) (count<0 or >len both collapse the
' same way), drop(count) = slice(count, len). Do not read the dead MFBASIC
' __collections_slice body as the spec of that clamping.
FUNC __collections_take OF T(value AS List OF T, count AS Integer) AS List OF T
  RETURN __collections_slice(value, 0, count)
END FUNC"#;

pub(crate) fn register(pkg: &mut crate::codegen::registry::RegistryPackage) {
    use crate::codegen::registry::{
        Body, DefaultValue, Implementation, Parameter, RegistryFunction,
    };
    use crate::types::ParameterType;

    pkg.add_function(RegistryFunction {
        name: "take",
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
                    desc: "How many leading elements to keep. Any `Integer` is accepted: values at or below 0 yield the empty list and values at or above the length yield the whole list. Named-argument spelling is `count`.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::list_of(ParameterType::var("T")),
            errors: vec![],
            body: Body::mfb(BODY, "__collections_take"),
        }],
    });
}
