//! `__datetime_offsetLabelSep` — shared private helper for the `datetime` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-339 C5: __datetime_offsetLabel and __datetime_offsetLabelCompact derived the
' sign/hh/mm identically and differed only in the ":" separator between hours and
' minutes. Shared here; both are thin wrappers passing their separator.
FUNC __datetime_offsetLabelSep(seconds AS Integer, sep AS String) AS String
  MUT sign AS String = "+"
  MUT s AS Integer = seconds
  IF s < 0 THEN
    sign = "-"
    s = -s
  END IF
  LET hh AS Integer = s / 3600
  LET mm AS Integer = (s / 60) MOD 60
  RETURN sign & __datetime_pad2(hh) & sep & __datetime_pad2(mm)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("datetime_offsetLabelSep", BODY));
}
