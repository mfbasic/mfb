//! `collections::isDisjoint` — descriptor entry + MFBASIC source body (Implementation::Mfb).
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use super::{custom, req};
use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = "Test whether two sets share no element";

#[rustfmt::skip]
const BODY: &str =
"FUNC __collections_isDisjoint OF T(a AS Set OF T, b AS Set OF T) AS Boolean
  FOR EACH x IN a
    IF collections::contains(b, x) THEN
      RETURN FALSE
    END IF
  NEXT
  RETURN TRUE
END FUNC";

const DESC: &str = r#"`collections::isDisjoint` returns `TRUE` when `a` and `b` have no element in
common, and `FALSE` otherwise. It walks the elements of `a` and returns `FALSE`
as soon as `collections::contains` reports one that is also in `b`; if the walk
finds no shared element, it returns `TRUE`. Equivalently, two sets are disjoint
exactly when their intersection is empty.

`isDisjoint` is **pure**: it inspects both arguments and mutates neither. The
empty set is disjoint from every set, so a call with an empty argument is always
`TRUE`. The relation is symmetric: `isDisjoint(a, b)` equals `isDisjoint(b, a)`.

`isDisjoint` raises no user-trappable error of its own.

`isDisjoint` is a generic implemented in MFBASIC source; a call is rewritten to
the internal `__collections_isDisjoint` generic and instantiated for the element
type like any other generic function."#;

const EX: &str = r#"Two sets with no common element:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET yes AS Boolean = collections::isDisjoint(Set OF Integer { 1, 2 }, Set OF Integer { 3, 4 })
  io::print(toString(yes))
  RETURN 0
END FUNC
```

A shared element makes it false:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET no AS Boolean = collections::isDisjoint(Set OF Integer { 1, 2 }, Set OF Integer { 2, 3 })
  io::print(toString(no))
  RETURN 0
END FUNC
```"#;

pub(crate) const IS_DISJOINT: BuiltinFunction = BuiltinFunction::mfb(
    "collections.isDisjoint",
    "isDisjoint",
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
