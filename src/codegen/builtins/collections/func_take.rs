//! `collections::take` — descriptor entry + MFBASIC source body (Implementation::Mfb).
//!
//! Body moved verbatim from `package.mfb`; see [`super::assembled_source`] for the
//! marker-substitution dual path. `BODY` is byte-significant (its 2-space
//! indentation feeds `.ncode` source-column metadata) — do NOT let a formatter
//! reindent it.

use super::{custom, req};
use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = "Return a new list holding the first `count` elements of a list";

#[rustfmt::skip]
const BODY: &str =
"FUNC __collections_take OF T(value AS List OF T, count AS Integer) AS List OF T
  RETURN __collections_slice(value, 0, count)
END FUNC";

pub(crate) const TAKE: BuiltinFunction = BuiltinFunction::mfb(
    "collections.take",
    "take",
    INTRO,
    "",
    &[],
    &[custom(&[
        req("value", &["list"], "List OF T"),
        req("count", &[], "Integer"),
    ])],
    BODY,
);
