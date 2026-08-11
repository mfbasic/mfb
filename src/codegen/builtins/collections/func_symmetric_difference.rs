//! `collections::symmetricDifference` — descriptor entry + MFBASIC source body
//! (Implementation::Mfb). Body byte-significant (2-space indent → `.ncode`
//! columns); do not reformat.

use super::{custom, req};
use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = "Return the set of elements in exactly one of two sets";

#[rustfmt::skip]
const BODY: &str =
"FUNC __collections_symmetricDifference OF T(a AS Set OF T, b AS Set OF T) AS Set OF T
  MUT result AS Set OF T = Set OF T { }
  FOR EACH x IN a
    IF NOT collections::contains(b, x) THEN
      result = collections::add(result, x)
    END IF
  NEXT
  FOR EACH y IN b
    IF NOT collections::contains(a, y) THEN
      result = collections::add(result, y)
    END IF
  NEXT
  RETURN result
END FUNC";

const DESC: &str = r#"`collections::symmetricDifference` returns a new `Set OF T` holding the elements
that are in exactly one of `a` and `b` — every element of their union that is not
in their intersection. It is computed as a two-pass fold: it keeps each element of
`a` that `collections::contains` reports as absent from `b`, then adds each
element of `b` that is absent from `a`. Unlike `difference`, the operation is
symmetric: `symmetricDifference(a, b)` and `symmetricDifference(b, a)` are equal.

`symmetricDifference` is **pure**: it returns a new value and mutates neither
argument. Element insertion order follows the surviving elements of `a` first,
then the surviving elements of `b`. The symmetric difference of two equal sets is
the empty set, and of a set with the empty set is a copy of that set.

`symmetricDifference` raises no user-trappable error of its own. Allocation
failure is not a trappable domain error, and the `add` it is built on is
classified infallible.

`symmetricDifference` is a generic implemented in MFBASIC source; a call is
rewritten to the internal `__collections_symmetricDifference` generic and
instantiated for the element type like any other generic function."#;

pub(crate) const SYMMETRIC_DIFFERENCE: BuiltinFunction = BuiltinFunction::mfb(
    "collections.symmetricDifference",
    "symmetricDifference",
    INTRO,
    DESC,
    &[],
    &[custom(&[
        req("a", &["first"], "Set OF T"),
        req("b", &["second"], "Set OF T"),
    ])],
    BODY,
);
