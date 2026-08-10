//! `collections::toSet` — descriptor entry + MFBASIC source body (Implementation::Mfb).
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use super::{custom, req};
use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = "Build a set from the distinct elements of a list";

#[rustfmt::skip]
const BODY: &str =
"FUNC __collections_toSet OF T(value AS List OF T) AS Set OF T
  MUT result AS Set OF T = Set OF T { }
  FOR EACH x IN value
    result = collections::add(result, x)
  NEXT
  RETURN result
END FUNC";

pub(crate) const TO_SET: BuiltinFunction = BuiltinFunction::mfb(
    "collections.toSet",
    "toSet",
    INTRO,
    "",
    &[],
    &[custom(&[req("value", &["list"], "List OF T")])],
    BODY,
);
