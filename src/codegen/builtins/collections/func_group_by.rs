//! `collections::groupBy` — descriptor entry + MFBASIC source body (Implementation::Mfb).
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use super::{custom, req};
use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = "Group the items of a list into a map of lists keyed by a projection";

#[rustfmt::skip]
const BODY: &str =
"FUNC __collections_groupBy OF T, K, V(value AS List OF T, keyFn AS FUNC(T) AS K, valFn AS FUNC(T) AS V) AS Map OF K TO List OF V
  LET keys AS List OF K = collections::transform(value, keyFn)
  LET vals AS List OF V = collections::transform(value, valFn)
  MUT result AS Map OF K TO List OF V = Map OF K TO List OF V {}
  MUT i AS Integer = 0
  WHILE i < len(keys)
    LET k AS K = collections::get(keys, i)
    LET v AS V = collections::get(vals, i)
    IF collections::hasKey(result, k) THEN
      MUT bucket AS List OF V = collections::get(result, k)
      bucket = collections::append(bucket, v)
      result = collections::set(result, k, bucket)
    ELSE
      MUT bucket AS List OF V = []
      bucket = collections::append(bucket, v)
      result = collections::set(result, k, bucket)
    END IF
    i = i + 1
  END WHILE
  RETURN result
END FUNC";

pub(crate) const GROUP_BY: BuiltinFunction = BuiltinFunction::mfb(
    "collections.groupBy",
    "groupBy",
    INTRO,
    "",
    &[],
    &[custom(&[
        req("value", &["list"], "List OF T"),
        req("keyFn", &["key"], "FUNC(T) AS K"),
        req("valFn", &["value"], "FUNC(T) AS V"),
    ])],
    BODY,
);
