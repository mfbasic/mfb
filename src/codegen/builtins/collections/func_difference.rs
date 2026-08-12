//! `collections::difference` — descriptor entry + MFBASIC source body (Implementation::Mfb).
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use super::{custom, req};
use crate::target::shared::registry::BuiltinFunction;

const INTRO: &str = "Return the set of elements in the first set but not the second";

#[rustfmt::skip]
const BODY: &str =
"FUNC __collections_difference OF T(a AS Set OF T, b AS Set OF T) AS Set OF T
  MUT result AS Set OF T = Set OF T { }
  FOR EACH x IN a
    IF NOT collections::contains(b, x) THEN
      result = collections::add(result, x)
    END IF
  NEXT
  RETURN result
END FUNC";

const DESC: &str = r#"`collections::difference` returns a new `Set OF T` holding the elements that are
in `a` but not in `b`. It walks the elements of `a` and keeps each one that
`collections::contains` reports as **absent** from `b`, so the result is `a` with
every element of `b` removed. The operation is asymmetric:
`difference(a, b)` and `difference(b, a)` are generally different sets.

`difference` is **pure**: it returns a new value and mutates neither argument.
Surviving elements keep the insertion order they had in `a`. The difference of a
set and the empty set is a copy of that set; the difference of a set with itself
is the empty set.

`difference` raises no user-trappable error of its own. Allocation failure is not
a trappable domain error, and the `add` it is built on is classified infallible.

`difference` is a generic implemented in MFBASIC source; a call is rewritten to
the internal `__collections_difference` generic and instantiated for the element
type like any other generic function."#;

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

pub(crate) const DIFFERENCE: BuiltinFunction = BuiltinFunction::mfb(
    "collections.difference",
    "difference",
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
