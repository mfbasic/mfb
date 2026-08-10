//! `collections::intersection` — descriptor entry + MFBASIC source body (Implementation::Mfb).
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use super::{custom, req};
use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = "Return the set of elements present in both of two sets";

#[rustfmt::skip]
const BODY: &str =
"FUNC __collections_intersection OF T(a AS Set OF T, b AS Set OF T) AS Set OF T
  MUT result AS Set OF T = Set OF T { }
  FOR EACH x IN a
    IF collections::contains(b, x) THEN
      result = collections::add(result, x)
    END IF
  NEXT
  RETURN result
END FUNC";

pub(crate) const INTERSECTION: BuiltinFunction = BuiltinFunction::mfb(
    "collections.intersection",
    "intersection",
    INTRO,
    "",
    &[],
    &[custom(&[
        req("a", &["first"], "Set OF T"),
        req("b", &["second"], "Set OF T"),
    ])],
    BODY,
);
