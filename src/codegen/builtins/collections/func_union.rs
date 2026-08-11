//! `collections::union` — descriptor entry + MFBASIC source body (Implementation::Mfb).
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use super::{custom, req};
use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = "Return the set of elements present in either of two sets";

#[rustfmt::skip]
const BODY: &str =
"FUNC __collections_union OF T(a AS Set OF T, b AS Set OF T) AS Set OF T
  MUT result AS Set OF T = a
  FOR EACH x IN b
    result = collections::add(result, x)
  NEXT
  RETURN result
END FUNC";

const DESC: &str = r#"`collections::union` returns a new `Set OF T` holding every element that is in
`a`, in `b`, or in both. It starts from the elements of `a` and adds each element
of `b`; because `collections::add` is idempotent, an element already present is
not duplicated, so the result contains each distinct element exactly once.

`union` is **pure**: it returns a new value and mutates neither argument. Element
insertion order follows the elements of `a` first, then the elements of `b` that
were not already in `a`. The union of a set with the empty set is a copy of that
set, and the union of two equal sets is a set equal to either one.

`union` raises no user-trappable error of its own. It allocates while building
the result, but allocation failure is not a trappable domain error, and the
`add` it is built on is classified infallible for exactly that reason.

`union` is a generic implemented in MFBASIC source; a call is rewritten to the
internal `__collections_union` generic and instantiated for the element type like
any other generic function."#;

pub(crate) const UNION: BuiltinFunction = BuiltinFunction::mfb(
    "collections.union",
    "union",
    INTRO,
    DESC,
    &[],
    &[custom(&[
        req("a", &["first"], "Set OF T"),
        req("b", &["second"], "Set OF T"),
    ])],
    BODY,
);
