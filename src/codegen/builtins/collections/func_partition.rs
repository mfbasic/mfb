//! `collections::partition` — descriptor entry + MFBASIC source body (Implementation::Mfb).
//! Body byte-significant (2-space indent + inline comments → `.ncode` columns);
//! do not reformat.

use super::{custom, req};
use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = "Split a list into the elements that satisfy a predicate and those that do not";

#[rustfmt::skip]
const BODY: &str =
"FUNC __collections_partition OF T(value AS List OF T, predicate AS FUNC(T) AS Boolean) AS Partition OF T
  ' `predicate` is evaluated once per element through `collections::transform`,
  ' whose callback loop checks the result tag and PROPAGATES a runtime failure —
  ' a directly-called `IF predicate(item)` silently swallows it (the same reason
  ' `sortBy`/`groupBy` build their keys via `transform`). The native
  ' `lower_collection_partition_call` covers the fixed-width fast path; this body
  ' serves the String/Scalar/Byte and inline-TRAP cases.
  LET flags AS List OF Boolean = collections::transform(value, predicate)
  MUT matched AS List OF T = []
  MUT unmatched AS List OF T = []
  MUT i AS Integer = 0
  WHILE i < len(value)
    LET item AS T = collections::get(value, i)
    IF collections::get(flags, i) THEN
      matched = collections::append(matched, item)
    ELSE
      unmatched = collections::append(unmatched, item)
    END IF
    i = i + 1
  END WHILE
  LET result AS Partition OF T = Partition[matched, unmatched]
  RETURN result
END FUNC";

pub(crate) const PARTITION: BuiltinFunction = BuiltinFunction::mfb(
    "collections.partition",
    "partition",
    INTRO,
    "",
    &[],
    &[custom(&[
        req("value", &["list"], "List OF T"),
        req("predicate", &[], "FUNC(T) AS Boolean"),
    ])],
    BODY,
);
