//! `collections::sortBy` — descriptor entry + MFBASIC source body (Implementation::Mfb).
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use super::{custom, req};
use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = "Return a new list ordered ascending by a key computed from each element";

#[rustfmt::skip]
const BODY: &str =
"FUNC __collections_sortBy OF T, U(value AS List OF T, keyFn AS FUNC(T) AS U) AS List OF T
  LET n AS Integer = len(value)
  IF n < 2 THEN
    RETURN value
  END IF
  MUT items AS List OF T = value
  MUT keys AS List OF U = collections::transform(value, keyFn)
  MUT width AS Integer = 1
  WHILE width < n
    MUT itemsDst AS List OF T = items
    MUT keysDst AS List OF U = keys
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
        IF collections::get(keys, j) < collections::get(keys, i) THEN
          itemsDst = collections::set(itemsDst, k, collections::get(items, j))
          keysDst = collections::set(keysDst, k, collections::get(keys, j))
          j = j + 1
        ELSE
          itemsDst = collections::set(itemsDst, k, collections::get(items, i))
          keysDst = collections::set(keysDst, k, collections::get(keys, i))
          i = i + 1
        END IF
        k = k + 1
      END WHILE
      WHILE i < mid
        itemsDst = collections::set(itemsDst, k, collections::get(items, i))
        keysDst = collections::set(keysDst, k, collections::get(keys, i))
        i = i + 1
        k = k + 1
      END WHILE
      WHILE j < hi
        itemsDst = collections::set(itemsDst, k, collections::get(items, j))
        keysDst = collections::set(keysDst, k, collections::get(keys, j))
        j = j + 1
        k = k + 1
      END WHILE
      lo = lo + width + width
    END WHILE
    items = itemsDst
    keys = keysDst
    width = width + width
  END WHILE
  RETURN items
END FUNC";

pub(crate) const SORT_BY: BuiltinFunction = BuiltinFunction::mfb(
    "collections.sortBy",
    "sortBy",
    INTRO,
    "",
    &[],
    &[custom(&[
        req("value", &["list"], "List OF T"),
        req("keyFn", &["key"], "FUNC(T) AS U"),
    ])],
    BODY,
);
