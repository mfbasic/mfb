//! `__net_authorityEnd` — shared private helper for the `net` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r##"' Index where the authority ends: the first of '/', '?', '#', or end-of-string.
FUNC __net_authorityEnd(rest AS String) AS Integer
  MUT best AS Integer = len(rest)
  LET slash AS Integer = __net_indexOf(rest, "/", 0)
  IF slash >= 0 AND slash < best THEN
    best = slash
  END IF
  LET question AS Integer = __net_indexOf(rest, "?", 0)
  IF question >= 0 AND question < best THEN
    best = question
  END IF
  LET hash AS Integer = __net_indexOf(rest, "#", 0)
  IF hash >= 0 AND hash < best THEN
    best = hash
  END IF
  RETURN best
END FUNC"##;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("net_authorityEnd", BODY));
}
