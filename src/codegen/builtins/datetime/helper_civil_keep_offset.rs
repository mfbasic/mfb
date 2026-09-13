//! `__datetime_civilKeepOffset` — shared private helper for the `datetime` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-520 S6: wall-clock arithmetic on a host-zone value (`addDays`, `addMonths`).
' `offset` is kept when it is still valid for the new wall clock, as Java's
' ZonedDateTime.plusDays does: a zero shift returns `dt`, and the second 01:30 of
' a fall-back stays the second. Re-resolving unconditionally took the earlier
' offset and moved that instant back an hour. Only when `offset` does not apply
' at the new wall clock does it re-resolve through `civil`'s gap/overlap rule.
FUNC __datetime_civilKeepOffset(d AS Date, t AS Time, z AS Zone, offset AS Integer) AS DateTime
  LET localSeconds AS Integer = __datetime_daysFromCivil(d.year, d.month, d.day) * 86400 + t.hour * 3600 + t.minute * 60 + t.second
  IF __datetime_offsetAt(z, Instant[localSeconds - offset, 0]) = offset THEN
    RETURN DateTime[d, t, z, offset]
  END IF
  RETURN __datetime_civil(d, t, z)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("datetime_civilKeepOffset", BODY));
}
