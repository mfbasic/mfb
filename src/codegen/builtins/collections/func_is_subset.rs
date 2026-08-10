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

pub(crate) const IS_SUBSET: BuiltinFunction = BuiltinFunction::mfb(
    "collections.isSubset",
    "isSubset",
    INTRO,
    "",
    &[],
    &[custom(&[
        req("a", &["first"], "Set OF T"),
        req("b", &["second"], "Set OF T"),
    ])],
    BODY,
);
