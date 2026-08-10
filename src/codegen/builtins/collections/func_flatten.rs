//! `collections::flatten` — descriptor entry + MFBASIC source body (Implementation::Mfb).
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use super::{custom, req};
use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = "Concatenate a list of lists into a single list, one level deep";

#[rustfmt::skip]
const BODY: &str =
"FUNC __collections_flatten OF T(value AS List OF List OF T) AS List OF T
  MUT result AS List OF T = []
  MUT i AS Integer = 0
  WHILE i < len(value)
    LET inner AS List OF T = collections::get(value, i)
    result = collections::append(result, inner)
    i = i + 1
  END WHILE
  RETURN result
END FUNC";

pub(crate) const FLATTEN: BuiltinFunction = BuiltinFunction::mfb(
    "collections.flatten",
    "flatten",
    INTRO,
    "",
    &[],
    &[custom(&[req("value", &["list"], "List OF List OF T")])],
    BODY,
);
