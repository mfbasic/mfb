//! `astrings::toMarkdown` — Tier-C rendering member (`Body::Rewrite`).
//!
//! Backed by the injected source (the per-helper `helper_*.rs` bodies): a call rewrites to the
//! internal `__astrings_toMarkdown` FUNC through the registry's `rewrite_target`.
//! Renders the resolved styling into a bespoke markdown-flavored format.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Render an `AttributedString`'s flags, font, and size into a bespoke markdown-flavored format; colors are left out."#;

const DESC: &str = r#"`toMarkdown` walks the text scalar by scalar, groups neighbouring scalars that carry the
same styling into one run, and writes each run with markers. A scalar is bold, italic, underlined,
struck or overlined when any covering span says so; its font and size come from the winning span as
`astrings::addAttribute` defines it. Foreground and background colours are ignored. `value` is not
modified. Runs are cut at Unicode scalar boundaries, the same unit `astrings::addAttribute` ranges
use, so a marker can land between a base letter and its combining mark.

**This is not CommonMark.** The format is read by the `astrings` toolchain, not a
standard markdown engine; do not treat `__`/`^^`/`::…::` as CommonMark.

- **Flags** wrap each run as nested pairs in this order — `**bold**`, `*italic*`,
  `__underline__`, `~~strike~~`, `^^overline^^` — and close in the reverse order, so
  overlapping spans always produce valid nesting.
- **Font/size** switch forward via a minimal-delta `::font;size::` marker emitted
  at run boundaries where the state changes: a value sets, `-` resets to default,
  and an omitted slot leaves it unchanged (`::font::` font-only, `::;size::`
  size-only, `::-::` font reset, `::;-::` size reset; a negative size such as
  `::;-5::` is a value, not a reset). Flags always close at the end of their
  run, but font and size are never reset at the end of the text; the last marker
  stays in force. A font set to the empty name is written with an empty slot
  (`::::`, or `::;12::` together with a size), which reads the same as no
  change; avoid empty font names if the output must be read back.
- **Delimiter characters** (`\ * _ ~ ^ :`) in the visible text are backslash-
  escaped; `;` and `-` in the text are not. In a font name `;` is escaped as
  well, and a font whose whole name is `-` is written `\-` so it is not read as
  a reset; a `-` inside a longer name is left as is.

Building the rendered text can raise `ErrOutOfMemory`, meaning the program ran
out of memory."#;

const EX: &str = r#"```
IMPORT astrings
IMPORT io

FUNC main() AS Nothing
  MUT a AS AttributedString = astrings::fromString("hello world")
  a = astrings::addAttribute(a, 0, 4, astrings::bold())
  io::print(astrings::toMarkdown(a))
END FUNC
```"#;

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
    pkg.add_function(RegistryFunction {
        name: "toMarkdown",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "value",
                desc: "The attributed string to render; empty text renders as an empty String.",
                aliases: &[],
                ty: ParameterType::named("AttributedString"),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::String,
            errors: vec!["ErrOutOfMemory"],
            body: Body::mfb(BODY, "__astrings_toMarkdown"),
        }],
    });
}
