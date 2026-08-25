//! `__astrings_mdStateAt` — shared private helper for the `astrings` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"REM Resolve the styling state at scalar `i`.
FUNC __astrings_mdStateAt(a AS AttributedString, i AS Integer) AS MdState
  MUT bold AS Boolean = FALSE
  MUT italic AS Boolean = FALSE
  MUT underline AS Boolean = FALSE
  MUT strike AS Boolean = FALSE
  MUT overline AS Boolean = FALSE
  MUT hasFont AS Boolean = FALSE
  MUT font AS String = ""
  MUT hasSize AS Boolean = FALSE
  MUT size AS Integer = 0
  FOR EACH at IN astrings::getAttributes(a, i)
    MATCH at
      CASE AttrFlag(f)
        MATCH f.kind
          CASE AttrTypeFlag.Bold
            bold = TRUE
          CASE AttrTypeFlag.Italic
            italic = TRUE
          CASE AttrTypeFlag.Underline
            underline = TRUE
          CASE AttrTypeFlag.Strike
            strike = TRUE
          CASE AttrTypeFlag.Overline
            overline = TRUE
        END MATCH
      CASE AttrText(t)
        hasFont = TRUE
        font = t.value
      CASE AttrNumber(nm)
        REM Only FontSize renders in markdown; Foreground/Background carry a
        REM packed color that this bespoke format does not represent, so ignore.
        IF nm.kind = AttrTypeNumber.FontSize THEN
          hasSize = TRUE
          size = nm.value
        END IF
    END MATCH
  NEXT
  RETURN MdState[bold, italic, underline, strike, overline, hasFont, font, hasSize, size]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("astrings_mdStateAt", BODY));
}
