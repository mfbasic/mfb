//! `collections::take` — descriptor entry + MFBASIC source body (Implementation::Mfb).
//!
//! Body moved verbatim from `package.mfb`; see [`super::assembled_source`] for the
//! marker-substitution dual path. `BODY` is byte-significant (its 2-space
//! indentation feeds `.ncode` source-column metadata) — do NOT let a formatter
//! reindent it.

use super::{custom, req};
use crate::target::shared::registry::BuiltinFunction;

const INTRO: &str = "Return a new list holding the first `count` elements of a list";

#[rustfmt::skip]
const BODY: &str =
"FUNC __collections_take OF T(value AS List OF T, count AS Integer) AS List OF T
  RETURN __collections_slice(value, 0, count)
END FUNC";

const DESC: &str = r#"`collections::take` returns a new list containing the leading `count` elements
of `value`, in their original order. It is a generic function written in MFBASIC
source: the call is rewritten to the internal `__collections_take` generic and
instantiated for the element type `T` during monomorphization.

`take(value, count)` is defined as the half-open range `[0, count)` of `value`,
delegated to the internal slice helper. That helper is lowered natively as a
bulk range copy, and the native lowering is what defines the boundary behavior:
the range start is clamped into `[0, len]` and the range stop into
`[start, len]`.

That clamping makes `take` **total** — every `Integer` value of `count` is
accepted and no index is ever rejected:

- `count` of 0 or any negative value clamps the stop back to the start, so the
  result is the empty list.
- `count` greater than or equal to the length of `value` clamps the stop to the
  length, so the whole list is returned.
- Otherwise the result holds exactly `count` elements.

The result is a freshly allocated list; element payloads are copied into it, so
the returned list does not share storage with `value`. `value` is not modified.
`collections::drop` is the complementary operation, returning what `take` leaves
behind."#;

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

pub(crate) const TAKE: BuiltinFunction = BuiltinFunction::mfb(
    "collections.take",
    "take",
    INTRO,
    DESC,
    &[],
    &[custom(&[
        req("value", &["list"], "List OF T"),
        req("count", &[], "Integer"),
    ])],
    BODY,
)
.with_example(EX);
