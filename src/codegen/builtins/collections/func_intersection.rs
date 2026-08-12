//! `collections::intersection` — descriptor entry + MFBASIC source body (Implementation::Mfb).
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use super::{custom, req};
use crate::target::shared::registry::BuiltinFunction;

const INTRO: &str = "Return the set of elements present in both of two sets";

#[rustfmt::skip]
const BODY: &str =
"FUNC __collections_intersection OF T(a AS Set OF T, b AS Set OF T) AS Set OF T
  MUT result AS Set OF T = Set OF T { }
  FOR EACH x IN a
    IF collections::contains(b, x) THEN
      result = collections::add(result, x)
    END IF
  NEXT
  RETURN result
END FUNC";

const DESC: &str = r#"`collections::intersection` returns a new `Set OF T` holding exactly the elements
that are in both `a` and `b`. It walks the elements of `a` and keeps each one
that `collections::contains` reports as present in `b`, so an element only
survives when it appears in both sets.

`intersection` is **pure**: it returns a new value and mutates neither argument.
Surviving elements keep the insertion order they had in `a`. The intersection of
disjoint sets is the empty set, and the intersection of a set with itself is a
set equal to it.

`intersection` raises no user-trappable error of its own. Allocation failure is
not a trappable domain error, and the `add` it is built on is classified
infallible.

`intersection` is a generic implemented in MFBASIC source; a call is rewritten to
the internal `__collections_intersection` generic and instantiated for the
element type like any other generic function."#;

const EX: &str = r#"Elements common to two sets:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET both AS Set OF Integer = collections::intersection(Set OF Integer { 1, 2, 3 }, Set OF Integer { 2, 3, 4 })
  io::print(toString(len(both)))
  RETURN 0
END FUNC
```

Disjoint sets intersect to the empty set:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET both AS Set OF Integer = collections::intersection(Set OF Integer { 1, 2 }, Set OF Integer { 3, 4 })
  io::print(toString(len(both)))
  RETURN 0
END FUNC
```"#;

pub(crate) const INTERSECTION: BuiltinFunction = BuiltinFunction::mfb(
    "collections.intersection",
    "intersection",
    INTRO,
    DESC,
    &[],
    &[custom(&[
        req("a", &["first"], "Set OF T"),
        req("b", &["second"], "Set OF T"),
    ])],
    BODY,
)
.with_example(EX);
