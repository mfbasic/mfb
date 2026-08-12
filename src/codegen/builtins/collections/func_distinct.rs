//! `collections::distinct` — descriptor entry + MFBASIC source body.
//!
//! First source-generic member migrated to `Implementation::Mfb`: its
//! `FUNC __collections_distinct` body moved verbatim out of `package.mfb` into
//! the [`BODY`] const below, and the dual-path source loader
//! ([`super::assembled_source`]) assembles it back into the injected package
//! source. The external `.mfb` remains the fallback for members not yet
//! migrated, so both paths coexist. `doc_desc` is left unauthored (as it was
//! while the member lived only in `package.mfb`); the one-line `doc_intro` comes
//! from the member's man page summary.

use super::{custom, req};
use crate::target::shared::registry::BuiltinFunction;

const INTO_DISTINCT: &str =
    "Remove duplicate elements from a list, keeping the first occurrence of each";

/// The MFBASIC implementation body, injected into importing projects and
/// monomorphized like any generic. Moved verbatim from `package.mfb`.
// NB: the body is byte-significant — its indentation (2-space, as authored in
// `package.mfb`) feeds source-column metadata into `.ncode`, so the
// byte-identity gate diffs if it is reindented. Do NOT let a formatter touch it.
#[rustfmt::skip]
const BODY: &str =
"FUNC __collections_distinct OF T(value AS List OF T) AS List OF T
  MUT result AS List OF T = []
  MUT i AS Integer = 0
  WHILE i < len(value)
    LET item AS T = collections::get(value, i)
    IF NOT collections::contains(result, item) THEN
      result = collections::append(result, item)
    END IF
    i = i + 1
  END WHILE
  RETURN result
END FUNC";

const DESC: &str = r#"`collections::distinct` returns a new list holding the elements of `value` with
duplicates removed. It walks `value` from index `0` upward and appends each
element to the result only when the result does not already contain an element
equal to it, so the **first** occurrence of each distinct value is the one kept
and later duplicates are dropped.

First-occurrence order is preserved: the surviving elements appear in the
result in the same relative order they had in `value`. The input is not
modified — `distinct` builds and returns a separate list. An empty input yields
an empty result.

Membership is tested with `collections::contains`, so "equal" here means exactly
the element equality that `contains` uses, and nothing else — there is no
user-supplied comparison and no key-extraction overload.
That equality is applied per element type: `Integer`, `Fixed`, `Money`,
`Boolean`, `Byte`, and `Scalar` compare by value; `String` compares by length
and then byte-for-byte over its UTF-8 bytes; a record compares field by field.

Two consequences of that equality deserve care:

- **`Float` is compared bitwise**, not with IEEE-754 numeric equality. `0.0` and
  `-0.0` are therefore treated as *distinct* values and both survive, while two
  `NaN` values with identical bit patterns are treated as *equal* and the second
  is dropped. This matches the packed-payload comparison used for `contains` and
  for map-literal keys.
- **String comparison is byte equality**, not Unicode-aware. Two strings that
  are canonically equivalent but differently normalized are distinct here; run
  `strings::normalizeNfc` (or `strings::caseFold` for case-insensitive
  deduplication) over the list first if that is not what you want.

`distinct` is O(n²) in the worst case: `contains` performs a linear scan of the
already-accumulated result for every input element, so a list with n distinct
elements does about n²/2 comparisons.
For large inputs of a comparable key type, building a `Map` keyed by the element
and reading `collections::keys` is asymptotically cheaper, at the cost of losing
first-occurrence order.

`distinct` raises no user-trappable error of its own. It allocates while
building the result, but allocation failure is not a trappable domain error, and
the `append` it uses is classified infallible for exactly that reason.

`distinct` is a generic implemented in MFBASIC source; a call is rewritten to
the internal `__collections_distinct` generic and instantiated for the element
type like any other generic function."#;

const EX: &str = r#"Deduplicate a list of integers:

```
IMPORT io
IMPORT collections

FUNC main AS Integer
  LET unique AS List OF Integer = collections::distinct([1, 2, 1, 3, 2])
  io::print(toString(len(unique)))
  RETURN 0
END FUNC
```

First occurrences are kept in their original order:

```
IMPORT io
IMPORT collections

FUNC main AS Integer
  LET names AS List OF String = collections::distinct(["b", "a", "b", "c"])
  io::print(collections::get(names, 0))
  io::print(collections::get(names, 1))
  io::print(collections::get(names, 2))
  RETURN 0
END FUNC
```

Normalize before deduplicating when Unicode equivalence matters:

```
IMPORT io
IMPORT collections
IMPORT strings

FUNC normalize(s AS String) AS String
  RETURN strings::normalizeNfc(s)
END FUNC

FUNC main AS Integer
  LET raw AS List OF String = ["a", "a", "b"]
  LET unique AS List OF String = collections::distinct(collections::transform(raw, normalize))
  io::print(toString(len(unique)))
  RETURN 0
END FUNC
```

The single parameter is named `value`:

```
IMPORT io
IMPORT collections

FUNC main AS Integer
  io::print(toString(len(collections::distinct(value := [1, 1, 2]))))
  RETURN 0
END FUNC
```"#;

pub(crate) const DISTINCT: BuiltinFunction = BuiltinFunction::mfb(
    "collections.distinct",
    "distinct",
    INTO_DISTINCT,
    DESC,
    &[],
    &[custom(&[req("value", &["collection"], "List OF T")])],
    BODY,
)
.with_example(EX);
