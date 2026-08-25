//! `__datetime_resolveLocal` — shared private helper for the `datetime` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Map a wall-clock second count (civil fields treated as if UTC) to the epoch
' instant it names in zone `z`, applying the §5.7 policy. Probes the offset a day
' on each side to bracket any single DST transition near the local time; a normal
' time uses the bracketing offset, a fall-back overlap takes the earlier offset,
' and a spring-forward gap shifts forward onto the post-transition offset.
FUNC __datetime_resolveLocal(localSeconds AS Integer, z AS Zone) AS Integer
  LET offEarly AS Integer = __datetime_offsetAt(z, Instant[localSeconds - 86400, 0])
  LET offLate AS Integer = __datetime_offsetAt(z, Instant[localSeconds + 86400, 0])
  LET candEarly AS Integer = localSeconds - offEarly
  IF offEarly = offLate THEN
    RETURN candEarly
  END IF
  LET candLate AS Integer = localSeconds - offLate
  LET earlyOk AS Boolean = __datetime_offsetAt(z, Instant[candEarly, 0]) = offEarly
  LET lateOk AS Boolean = __datetime_offsetAt(z, Instant[candLate, 0]) = offLate
  IF lateOk AND NOT earlyOk THEN
    RETURN candLate
  END IF
  RETURN candEarly
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("datetime_resolveLocal", BODY));
}
