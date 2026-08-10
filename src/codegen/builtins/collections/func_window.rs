//! `collections::window` — descriptor entry + MFBASIC source body (Implementation::Mfb).
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use super::{custom, opt, req};
use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = "Produce the sliding windows of a list, each of exactly `size` elements";

#[rustfmt::skip]
const BODY: &str =
"FUNC __collections_window OF T(value AS List OF T, size AS Integer, stride AS Integer = 1) AS List OF List OF T
  IF size < 1 OR stride < 1 THEN
    FAIL error(77050002, \"Argument value is not valid for the requested operation.\")
  END IF
  MUT result AS List OF List OF T = []
  MUT i AS Integer = 0
  WHILE i + size <= len(value)
    LET piece AS List OF T = __collections_slice(value, i, i + size)
    result = collections::append(result, piece)
    i = i + stride
  END WHILE
  RETURN result
END FUNC";

pub(crate) const WINDOW: BuiltinFunction = BuiltinFunction::mfb(
    "collections.window",
    "window",
    INTRO,
    "",
    &["ErrInvalidArgument"],
    &[custom(&[
        req("value", &["list"], "List OF T"),
        req("size", &[], "Integer"),
        opt("stride", &[], "Integer"),
    ])],
    BODY,
);
