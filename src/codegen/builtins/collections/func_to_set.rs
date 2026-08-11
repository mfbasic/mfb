//! `collections::toSet` — descriptor entry + MFBASIC source body (Implementation::Mfb).
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use super::{custom, req};
use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = "Build a set from the distinct elements of a list";

#[rustfmt::skip]
const BODY: &str =
"FUNC __collections_toSet OF T(value AS List OF T) AS Set OF T
  MUT result AS Set OF T = Set OF T { }
  FOR EACH x IN value
    result = collections::add(result, x)
  NEXT
  RETURN result
END FUNC";

const DESC: &str = r#"`collections::toSet` returns a new `Set OF T` containing every distinct element
of the list `value`. It folds over `value` in order and adds each element to a
fresh set; because `collections::add` is idempotent, a repeated element is stored
only once, so the result holds each distinct element exactly once.

`toSet` is **pure**: it returns a new value and does not mutate `value`. Element
insertion order follows first occurrence in the list, so `toSet([2, 1, 2, 3])`
holds `2`, `1`, `3` in that order. Converting a list that is already free of
duplicates yields a set with the same elements; converting the empty list yields
the empty set.

`toSet` raises no user-trappable error of its own. It allocates while building the
result, but allocation failure is not a trappable domain error, and the `add` it
is built on is classified infallible for exactly that reason.

`toSet` is a generic implemented in MFBASIC source; a call is rewritten to the
internal `__collections_toSet` generic and instantiated for the element type like
any other generic function."#;

pub(crate) const TO_SET: BuiltinFunction = BuiltinFunction::mfb(
    "collections.toSet",
    "toSet",
    INTRO,
    DESC,
    &[],
    &[custom(&[req("value", &["list"], "List OF T")])],
    BODY,
);
