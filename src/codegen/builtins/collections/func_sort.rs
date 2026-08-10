//! `collections::sort` — descriptor entry + MFBASIC source body (Implementation::Mfb).
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use super::{custom, req};
use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = "Return a new list holding the elements of a list in ascending order";

#[rustfmt::skip]
const BODY: &str =
"FUNC __collections_sort OF T(value AS List OF T) AS List OF T
  LET n AS Integer = len(value)
  IF n < 2 THEN
    RETURN value
  END IF
  MUT src AS List OF T = value
  MUT width AS Integer = 1
  WHILE width < n
    MUT dst AS List OF T = src
    MUT lo AS Integer = 0
    WHILE lo < n
      MUT mid AS Integer = lo + width
      IF mid > n THEN
        mid = n
      END IF
      MUT hi AS Integer = lo + width + width
      IF hi > n THEN
        hi = n
      END IF
      MUT i AS Integer = lo
      MUT j AS Integer = mid
      MUT k AS Integer = lo
      WHILE i < mid AND j < hi
        IF collections::get(src, j) < collections::get(src, i) THEN
          dst = collections::set(dst, k, collections::get(src, j))
          j = j + 1
        ELSE
          dst = collections::set(dst, k, collections::get(src, i))
          i = i + 1
        END IF
        k = k + 1
      END WHILE
      WHILE i < mid
        dst = collections::set(dst, k, collections::get(src, i))
        i = i + 1
        k = k + 1
      END WHILE
      WHILE j < hi
        dst = collections::set(dst, k, collections::get(src, j))
        j = j + 1
        k = k + 1
      END WHILE
      lo = lo + width + width
    END WHILE
    src = dst
    width = width + width
  END WHILE
  RETURN src
END FUNC";

pub(crate) const SORT: BuiltinFunction = BuiltinFunction::mfb(
    "collections.sort",
    "sort",
    INTRO,
    "",
    &[],
    &[custom(&[req("value", &["list"], "List OF T")])],
    BODY,
);
