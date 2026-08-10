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

pub(crate) const UNION: BuiltinFunction = BuiltinFunction::mfb(
    "collections.union",
    "union",
    INTRO,
    "",
    &[],
    &[custom(&[
        req("a", &["first"], "Set OF T"),
        req("b", &["second"], "Set OF T"),
    ])],
    BODY,
);
