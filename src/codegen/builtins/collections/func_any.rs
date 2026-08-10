//! `collections::any` — descriptor entry + MFBASIC source body (Implementation::Mfb).
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use super::{custom, req};
use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = "Test whether at least one element of a list satisfies a predicate";

#[rustfmt::skip]
const BODY: &str =
"FUNC __collections_any OF T(value AS List OF T, predicate AS FUNC(T) AS Boolean) AS Boolean
  MUT i AS Integer = 0
  WHILE i < len(value)
    IF predicate(collections::get(value, i)) THEN
      RETURN TRUE
    END IF
    i = i + 1
  END WHILE
  RETURN FALSE
END FUNC";

pub(crate) const ANY: BuiltinFunction = BuiltinFunction::mfb(
    "collections.any",
    "any",
    INTRO,
    "",
    &[],
    &[custom(&[
        req("value", &["list"], "List OF T"),
        req("predicate", &[], "FUNC(T) AS Boolean"),
    ])],
    BODY,
);
