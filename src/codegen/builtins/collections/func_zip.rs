//! `collections::zip` — descriptor entry + MFBASIC source body (Implementation::Mfb).
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use super::{custom, req};
use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = "Pair items from two lists position-wise";

#[rustfmt::skip]
const BODY: &str =
"FUNC __collections_zip OF A, B(a AS List OF A, b AS List OF B) AS List OF Pair OF A, B
  MUT result AS List OF Pair OF A, B = []
  MUT n AS Integer = len(a)
  IF len(b) < n THEN
    n = len(b)
  END IF
  MUT i AS Integer = 0
  WHILE i < n
    LET p AS Pair OF A, B = Pair[collections::get(a, i), collections::get(b, i)]
    result = collections::append(result, p)
    i = i + 1
  END WHILE
  RETURN result
END FUNC";

pub(crate) const ZIP: BuiltinFunction = BuiltinFunction::mfb(
    "collections.zip",
    "zip",
    INTRO,
    "",
    &[],
    &[custom(&[
        req("a", &["first"], "List OF A"),
        req("b", &["second"], "List OF B"),
    ])],
    BODY,
);
