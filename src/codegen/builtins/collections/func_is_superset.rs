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

pub(crate) const IS_SUPERSET: BuiltinFunction = BuiltinFunction::mfb(
    "collections.isSuperset",
    "isSuperset",
    INTRO,
    "",
    &[],
    &[custom(&[
        req("a", &["first"], "Set OF T"),
        req("b", &["second"], "Set OF T"),
    ])],
    BODY,
);
