//! `collections::isSubset` — descriptor entry + MFBASIC source body (Implementation::Mfb).
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use super::{custom, req};
use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = "Test whether every element of the first set is in the second";

#[rustfmt::skip]
const BODY: &str =
"FUNC __collections_isSubset OF T(a AS Set OF T, b AS Set OF T) AS Boolean
  FOR EACH x IN a
    IF NOT collections::contains(b, x) THEN
      RETURN FALSE
    END IF
  NEXT
  RETURN TRUE
END FUNC";

const DESC: &str = r#"`collections::isSubset` returns `TRUE` when every element of `a` is also in `b`,
and `FALSE` otherwise. It walks the elements of `a` and returns `FALSE` as soon as
`collections::contains` reports one that is absent from `b`; if the walk finishes
with no such element, it returns `TRUE`.

`isSubset` is **pure**: it inspects both arguments and mutates neither. The empty
set is a subset of every set, so `isSubset(Set OF T { }, b)` is always `TRUE`. A
set is a subset of itself, and equal sets are subsets of each other.

`isSubset` raises no user-trappable error of its own.

`isSubset` is a generic implemented in MFBASIC source; a call is rewritten to the
internal `__collections_isSubset` generic and instantiated for the element type
like any other generic function."#;

const EX: &str = r#"A smaller set contained in a larger one:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET yes AS Boolean = collections::isSubset(Set OF Integer { 1, 2 }, Set OF Integer { 1, 2, 3 })
  io::print(toString(yes))
  RETURN 0
END FUNC
```

An element outside the second set makes it false:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET no AS Boolean = collections::isSubset(Set OF Integer { 1, 9 }, Set OF Integer { 1, 2, 3 })
  io::print(toString(no))
  RETURN 0
END FUNC
```"#;

pub(crate) const IS_SUBSET: BuiltinFunction = BuiltinFunction::mfb(
    "collections.isSubset",
    "isSubset",
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
