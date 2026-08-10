//! `collections::mapValues` — descriptor entry + MFBASIC source body (Implementation::Mfb).
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use super::{custom, req};
use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = "Transform every value of a map, leaving the keys unchanged";

#[rustfmt::skip]
const BODY: &str =
"FUNC __collections_mapValues OF K, V, U(value AS Map OF K TO V, f AS FUNC(V) AS U) AS Map OF K TO U
  MUT result AS Map OF K TO U = Map OF K TO U {}
  FOR EACH e IN value
    result = collections::set(result, e.key, f(e.value))
  NEXT
  RETURN result
END FUNC";

pub(crate) const MAP_VALUES: BuiltinFunction = BuiltinFunction::mfb(
    "collections.mapValues",
    "mapValues",
    INTRO,
    "",
    &[],
    &[custom(&[
        req("value", &["map"], "Map OF K TO V"),
        req("f", &["transform"], "FUNC(V) AS U"),
    ])],
    BODY,
);
