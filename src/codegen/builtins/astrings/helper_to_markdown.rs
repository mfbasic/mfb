//! `__astrings_toMarkdown` — shared private helper for the `astrings` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __astrings_toMarkdown(a AS AttributedString) AS String
  LET text AS String = toString(a)
  LET n AS Integer = astrings::scalarLen(a)
  MUT out AS String = ""
  REM Running font/size state (the last emitted); starts at default (unset).
  MUT runHasFont AS Boolean = FALSE
  MUT runFont AS String = ""
  MUT runHasSize AS Boolean = FALSE
  MUT runSize AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < n
    LET st AS MdState = __astrings_mdStateAt(a, i)
    MUT j AS Integer = i + 1
    WHILE j < n AND __astrings_mdStateAt(a, j) = st
      j = j + 1
    END WHILE

    REM font/size forward state switch (minimal delta).
    LET fontChanged AS Boolean = (st.hasFont <> runHasFont) OR (st.hasFont AND st.font <> runFont)
    LET sizeChanged AS Boolean = (st.hasSize <> runHasSize) OR (st.hasSize AND st.size <> runSize)
    IF fontChanged OR sizeChanged THEN
      MUT fpart AS String = "-"
      IF st.hasFont THEN
        fpart = __astrings_mdEscapeFont(st.font)
      END IF
      MUT spart AS String = "-"
      IF st.hasSize THEN
        spart = toString(st.size)
      END IF
      IF fontChanged AND sizeChanged THEN
        out = out & "::" & fpart & ";" & spart & "::"
      ELSE
        IF fontChanged THEN
          out = out & "::" & fpart & "::"
        ELSE
          out = out & "::;" & spart & "::"
        END IF
      END IF
      runHasFont = st.hasFont
      runFont = st.font
      runHasSize = st.hasSize
      runSize = st.size
    END IF

    REM open flags (canonical order), escaped run text, close flags (reverse).
    IF st.bold THEN
      out = out & "**"
    END IF
    IF st.italic THEN
      out = out & "*"
    END IF
    IF st.underline THEN
      out = out & "__"
    END IF
    IF st.strike THEN
      out = out & "~~"
    END IF
    IF st.overline THEN
      out = out & "^^"
    END IF
    out = out & __astrings_mdEscape(strings::mid(text, i, j - i))
    IF st.overline THEN
      out = out & "^^"
    END IF
    IF st.strike THEN
      out = out & "~~"
    END IF
    IF st.underline THEN
      out = out & "__"
    END IF
    IF st.italic THEN
      out = out & "*"
    END IF
    IF st.bold THEN
      out = out & "**"
    END IF

    i = j
  END WHILE
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("astrings_toMarkdown", BODY));
}
