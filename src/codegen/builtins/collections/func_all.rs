//! `collections::all` — descriptor entry + MFBASIC source body (Implementation::Mfb).
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use super::{custom, req};
use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = "Test whether every element of a list satisfies a predicate";

#[rustfmt::skip]
const BODY: &str =
"FUNC __collections_all OF T(value AS List OF T, predicate AS FUNC(T) AS Boolean) AS Boolean
  MUT i AS Integer = 0
  WHILE i < len(value)
    IF NOT predicate(collections::get(value, i)) THEN
      RETURN FALSE
    END IF
    i = i + 1
  END WHILE
  RETURN TRUE
END FUNC";

pub(crate) const ALL: BuiltinFunction = BuiltinFunction::mfb(
    "collections.all",
    "all",
    INTRO,
    "",
    &[],
    &[custom(&[
        req("value", &["list"], "List OF T"),
        req("predicate", &[], "FUNC(T) AS Boolean"),
    ])],
    BODY,
);
