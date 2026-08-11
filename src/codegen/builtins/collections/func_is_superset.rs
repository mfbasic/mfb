//! `collections::isSuperset` — descriptor entry + MFBASIC source body (Implementation::Mfb).
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use super::{custom, req};
use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = "Test whether the first set contains every element of the second";

#[rustfmt::skip]
const BODY: &str =
"FUNC __collections_isSuperset OF T(a AS Set OF T, b AS Set OF T) AS Boolean
  FOR EACH x IN b
    IF NOT collections::contains(a, x) THEN
      RETURN FALSE
    END IF
  NEXT
  RETURN TRUE
END FUNC";

const DESC: &str = r#"`collections::isSuperset` returns `TRUE` when every element of `b` is also in
`a`, and `FALSE` otherwise. It is `isSubset` with the arguments swapped: it walks
the elements of `b` and returns `FALSE` as soon as `collections::contains` reports
one that is absent from `a`, returning `TRUE` if the walk finds no such element.

`isSuperset` is **pure**: it inspects both arguments and mutates neither. Every
set is a superset of the empty set, so `isSuperset(a, Set OF T { })` is always
`TRUE`. A set is a superset of itself, and equal sets are supersets of each other.

`isSuperset` raises no user-trappable error of its own.

`isSuperset` is a generic implemented in MFBASIC source; a call is rewritten to
the internal `__collections_isSuperset` generic and instantiated for the element
type like any other generic function."#;

pub(crate) const IS_SUPERSET: BuiltinFunction = BuiltinFunction::mfb(
    "collections.isSuperset",
    "isSuperset",
    INTRO,
    DESC,
    &[],
    &[custom(&[
        req("a", &["first"], "Set OF T"),
        req("b", &["second"], "Set OF T"),
    ])],
    BODY,
);
