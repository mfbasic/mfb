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

pub(crate) const IS_DISJOINT: BuiltinFunction = BuiltinFunction::mfb(
    "collections.isDisjoint",
    "isDisjoint",
    INTRO,
    "",
    &[],
    &[custom(&[
        req("a", &["first"], "Set OF T"),
        req("b", &["second"], "Set OF T"),
    ])],
    BODY,
);
