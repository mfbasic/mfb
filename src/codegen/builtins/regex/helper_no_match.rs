//! `__regex_noMatch` — shared private helper for the `regex` package.
//!
//! The `MatchInfo` the span-returning members hand back when nothing matched. The
//! package never raises on absence (`find` returns `-1`, `findAll` returns `[]`),
//! so `findMatch` reports absence the same way: a `MatchInfo` whose `start` is `-1`.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_noMatch AS MatchInfo
  LET g AS List OF Group = []
  LET nm AS Map OF String TO Integer = Map OF String TO Integer {}
  RETURN MatchInfo[-1, -1, "", g, nm]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_noMatch", BODY));
}
