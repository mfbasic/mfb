//! `collections::distinct` — descriptor entry + MFBASIC source body.
//!
//! First source-generic member migrated to `Implementation::Mfb`: its
//! `FUNC __collections_distinct` body moved verbatim out of `package.mfb` into
//! the [`BODY`] const below, and the dual-path source loader
//! ([`super::assembled_source`]) assembles it back into the injected package
//! source. The external `.mfb` remains the fallback for members not yet
//! migrated, so both paths coexist. `doc_desc` is left unauthored (as it was
//! while the member lived only in `package.mfb`); the one-line `doc_intro` comes
//! from the member's man page summary.

use super::{custom, req};
use crate::codegen::registry::BuiltinFunction;

const INTO_DISTINCT: &str =
    "Remove duplicate elements from a list, keeping the first occurrence of each";

/// The MFBASIC implementation body, injected into importing projects and
/// monomorphized like any generic. Moved verbatim from `package.mfb`.
// NB: the body is byte-significant — its indentation (2-space, as authored in
// `package.mfb`) feeds source-column metadata into `.ncode`, so the
// byte-identity gate diffs if it is reindented. Do NOT let a formatter touch it.
#[rustfmt::skip]
const BODY: &str =
"FUNC __collections_distinct OF T(value AS List OF T) AS List OF T
  MUT result AS List OF T = []
  MUT i AS Integer = 0
  WHILE i < len(value)
    LET item AS T = collections::get(value, i)
    IF NOT collections::contains(result, item) THEN
      result = collections::append(result, item)
    END IF
    i = i + 1
  END WHILE
  RETURN result
END FUNC";

pub(crate) const DISTINCT: BuiltinFunction = BuiltinFunction::mfb(
    "collections.distinct",
    "distinct",
    INTO_DISTINCT,
    "",
    &[],
    &[custom(&[req("value", &["collection"], "List OF T")])],
    BODY,
);
