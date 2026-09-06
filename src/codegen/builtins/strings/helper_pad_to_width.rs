//! `__strings_padLeftToWidth` / `__strings_padRightToWidth` — the column-counted
//! padding seam behind `strings::padLeftToWidth` / `padRightToWidth` (bug-528).
//!
//! A gated helper chunk rather than two `Body::Mfb` member bodies: a `Body::Mfb`
//! body renders into `get_mfb` for EVERY `IMPORT strings` program, and this seam
//! is only wanted by a program that actually pads to a column width. The
//! `WhenUsed` gate is the same mechanism the scalar seam uses, and injected FUNCs
//! are file-local, so the two members and the copy count they share must stay in
//! ONE chunk. Body byte-significant (2-space indent → `.ncode` columns); do not
//! reformat.

use crate::codegen::registry::{HelperGate, RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"REM MFBASIC strings display-width padding companion (bug-528). Column-counted
REM padding: strings::padLeft counts Unicode scalar values, strings::displayWidth
REM counts terminal columns, and these two members close the gap between them.

IMPORT strings

REM How many whole copies of padChar fit in the gap between value's display width
REM and `columns`. Rejects the inputs that have no answer: a negative target, a
REM padChar that is not exactly one scalar, and a padChar occupying no columns at
REM all (a combining mark or a zero-width joiner), which could never reach the
REM target however many copies were laid down.
FUNC __strings_padToWidthCopies(value AS String, columns AS Integer, padChar AS String) AS Integer
  IF columns < 0 THEN
    FAIL error(77050002, "Argument value is not valid for the requested operation.")
  END IF
  IF len(padChar) <> 1 THEN
    FAIL error(77050002, "Argument value is not valid for the requested operation.")
  END IF
  LET unit AS Integer = strings::displayWidth(padChar)
  IF unit < 1 THEN
    FAIL error(77050002, "Argument value is not valid for the requested operation.")
  END IF
  LET have AS Integer = strings::displayWidth(value)
  IF have >= columns THEN
    RETURN 0
  END IF
  REM Integer division truncates toward zero, which IS the undershoot rule: the
  REM result never exceeds `columns`.
  RETURN (columns - have) / unit
END FUNC

FUNC __strings_padLeftToWidth(value AS String, columns AS Integer, padChar AS String) AS String
  LET copies AS Integer = __strings_padToWidthCopies(value, columns, padChar)
  IF copies = 0 THEN
    RETURN value
  END IF
  RETURN strings::repeat(padChar, copies) & value
END FUNC

FUNC __strings_padRightToWidth(value AS String, columns AS Integer, padChar AS String) AS String
  LET copies AS Integer = __strings_padToWidthCopies(value, columns, padChar)
  IF copies = 0 THEN
    RETURN value
  END IF
  RETURN value & strings::repeat(padChar, copies)
END FUNC"#;

/// The members whose use pulls the chunk in. Both share
/// `__strings_padToWidthCopies`, so neither can be gated alone.
const PAD_TO_WIDTH_MEMBERS: &[&str] = &["padLeftToWidth", "padRightToWidth"];

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper {
        name: "strings_padToWidth",
        gate: HelperGate::WhenUsed(PAD_TO_WIDTH_MEMBERS),
        body: Some(BODY),
        import_name: None,
    });
    // An injected `astrings` companion calls the seam through
    // `__astrings_padLeftToWidth`, and an `astrings`-only program never imports
    // `strings` in user source — the same bridge the scalar seam needs. MEASURED,
    // not assumed: without this second gate an `astrings`-only program that never
    // mentions the member fails to link with
    // `NIR call target '#strings_padLeftToWidth' does not resolve`, because the
    // always-injected companion references it whether or not the program does.
    pkg.add_helper(RegistryHelper {
        name: "strings_padToWidth",
        gate: HelperGate::WhenImported("astrings"),
        body: Some(BODY),
        import_name: None,
    });
}
