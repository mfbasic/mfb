//! `collections::merge` — descriptor entry + MFBASIC source body (Implementation::Mfb).
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use super::{custom, req};
use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = "Combine two maps into one, choosing which side wins on a key collision";

#[rustfmt::skip]
const BODY: &str =
"FUNC __collections_merge OF K, V(a AS Map OF K TO V, b AS Map OF K TO V, preferB AS Boolean) AS Map OF K TO V
  MUT result AS Map OF K TO V = a
  FOR EACH e IN b
    IF preferB OR NOT collections::hasKey(result, e.key) THEN
      result = collections::set(result, e.key, e.value)
    END IF
  NEXT
  RETURN result
END FUNC";

pub(crate) const MERGE: BuiltinFunction = BuiltinFunction::mfb(
    "collections.merge",
    "merge",
    INTRO,
    "",
    &[],
    &[custom(&[
        req("a", &["first"], "Map OF K TO V"),
        req("b", &["second"], "Map OF K TO V"),
        req("preferB", &[], "Boolean"),
    ])],
    BODY,
);
