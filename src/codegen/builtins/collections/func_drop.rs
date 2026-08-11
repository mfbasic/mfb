//! `collections::drop` — descriptor entry + MFBASIC source body (Implementation::Mfb).
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use super::{custom, req};
use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = "Return a new list with the first `count` elements removed";

#[rustfmt::skip]
const BODY: &str =
"FUNC __collections_drop OF T(value AS List OF T, count AS Integer) AS List OF T
  RETURN __collections_slice(value, count, len(value))
END FUNC";

const DESC: &str = r#"`collections::drop` returns a new list containing everything in `value` except
its leading `count` elements, in their original order. It is a generic function
written in MFBASIC source: the call is rewritten to the internal
`__collections_drop` generic and instantiated for the element type `T` during
monomorphization.

`drop(value, count)` is defined as the half-open range `[count, len(value))` of
`value`, delegated to the internal slice helper. That helper is lowered natively
as a bulk range copy, and the native lowering is what defines the boundary
behavior: the range start is clamped into `[0, len]` and the range stop into
`[start, len]`.

That clamping makes `drop` **total** — every `Integer` value of `count` is
accepted and no index is ever rejected:

- `count` of 0 or any negative value clamps the start back to 0, so the whole
  list is returned.
- `count` greater than or equal to the length of `value` clamps the start to the
  length, so the result is the empty list.
- Otherwise the result holds `len(value) - count` elements.

The result is a freshly allocated list; element payloads are copied into it, so
the returned list does not share storage with `value`. `value` is not modified.
`collections::take` is the complementary operation, returning the elements
`drop` discards."#;

pub(crate) const DROP: BuiltinFunction = BuiltinFunction::mfb(
    "collections.drop",
    "drop",
    INTRO,
    DESC,
    &[],
    &[custom(&[
        req("value", &["list"], "List OF T"),
        req("count", &[], "Integer"),
    ])],
    BODY,
);
