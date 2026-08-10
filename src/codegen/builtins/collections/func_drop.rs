//! `collections::drop` — descriptor entry + MFBASIC source body (Implementation::Mfb).
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use super::{custom, req};
use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = "Return a new list with the first `count` elements removed";

#[rustfmt::skip]
const BODY: &str =
"FUNC __collections_drop OF T(value AS List OF T, count AS Integer) AS List OF T
  RETURN __collections_slice(value, count, len(value))
END FUNC";

pub(crate) const DROP: BuiltinFunction = BuiltinFunction::mfb(
    "collections.drop",
    "drop",
    INTRO,
    "",
    &[],
    &[custom(&[
        req("value", &["list"], "List OF T"),
        req("count", &[], "Integer"),
    ])],
    BODY,
);
