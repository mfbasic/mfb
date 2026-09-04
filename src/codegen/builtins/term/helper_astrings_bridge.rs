//! The `term`↔`astrings` `drawText(AttributedString)` bridge — `__term_drawTextAttr`
//! plus its private `__TermStyle` record and color/style helpers, as ONE gated
//! source chunk.
//!
//! Why this is a helper chunk and not a `Body::mfb` overload on `func_draw_text.rs`:
//! the body references `AttributedString`/`astrings::` (undefined unless `astrings`
//! is imported), so it must inject only when BOTH `term` and `astrings` are imported
//! (`HelperGate::WhenBothImported`, the legacy `term::bridge_uses_package`) — a
//! `Body::Mfb` body renders into `get_mfb` unconditionally. The gated chunk is its
//! own synthetic file (`<builtin-term_astrings_bridge>`), and injected FUNCs are
//! file-local, so its FUNC/SUB/TYPE members must stay together in one chunk. The
//! `strings` scalar seam the bridge calls rides in through `strings`' own
//! `WhenImported("astrings")` gate. Body byte-significant; do not reformat.

use crate::codegen::registry::{HelperGate, RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' ===========================================================================
' term ↔ astrings bridge: term::drawText(x, y, text AS AttributedString)
'
' The native drawing callables know nothing of the `astrings` attribute overlay,
' so the AttributedString overload of `term::drawText` is a source-companion body
' here rather than native codegen. It is injected only when a program imports BOTH
' `term` and `astrings` (see `term::bridge_uses_package`), so a plain `IMPORT term`
' program never drags in the `astrings`/`strings` companions this file needs.
'
' `__term_drawTextAttr` stamps the visible text at (x, y) exactly as the String
' overload, but honours the per-scalar styling the AttributedString carries. Only
' the attributes the terminal surface can represent are applied — bold, underline,
' foreground color and background color; every other attribute (italic, strike,
' overline, font, font size) is silently ignored. The text is drawn in maximal
' runs of a single (bold, underline, foreground, background) state, and each run is
' handed to the native `String` drawText so the grapheme-cluster and wide-glyph
' handling is identical to the String overload; runs are placed left to right,
' advancing the start column by each run's display width. The current global
' bold/underline/foreground/background are saved before the runs and restored
' after, so drawText leaves the attribute state it found (matching the String
' overload, which never changes it). While TUI mode is off the whole body is
' skipped, so — like every other term:: call — it draws nothing and raises nothing.
' ===========================================================================

IMPORT term
IMPORT astrings
IMPORT strings
IMPORT bits

' The renderable subset of an AttributedString's per-scalar styling: the attributes
' the terminal surface can represent — bold, underline, and packed 0xAARRGGBB
' foreground/background colors (-1 = unset, unambiguous because a packed color is
' always in 0..0xFFFFFFFF, which is positive). Compared by value to find maximal
' same-style runs. Named with the internal `__` sigil so the injected type cannot
' collide with a user's own `TermStyle`.
TYPE __TermStyle
  bold AS Boolean
  underline AS Boolean
  fg AS Integer
  bg AS Integer
END TYPE

' Resolve the renderable style at scalar `index` in a single pass over the covering
' attributes: bold/underline flags (any covering span carrying the flag turns it
' on) and the winning foreground/background packed color (or -1 when unset). The
' attributes the terminal cannot render (italic/strike/overline flags, font,
' font size) are ignored.
FUNC __term_styleAt(value AS AttributedString, index AS Integer) AS __TermStyle
  MUT bold AS Boolean = FALSE
  MUT underline AS Boolean = FALSE
  MUT fg AS Integer = -1
  MUT bg AS Integer = -1
  FOR EACH at IN astrings::getAttributes(value, index)
    MATCH at
      CASE astrings::AttrFlag(f)
        MATCH f.kind
          CASE astrings::AttrTypeFlag.Bold
            bold = TRUE
          CASE astrings::AttrTypeFlag.Underline
            underline = TRUE
          CASE ELSE
        END MATCH
      CASE astrings::AttrNumber(nm)
        MATCH nm.kind
          CASE astrings::AttrTypeNumber.Foreground
            fg = nm.value
          CASE astrings::AttrTypeNumber.Background
            bg = nm.value
          CASE ELSE
        END MATCH
      CASE ELSE
    END MATCH
  NEXT
  RETURN __TermStyle[bold, underline, fg, bg]
END FUNC

' Unpack the r / g / b / a channel from a packed `0xAARRGGBB` color (alpha high,
' b low). The r/g/b shifts are unchanged by the plan-122-E widening — alpha was
' added above bit 23, so the colour channels did not move.
FUNC __term_colorR(packed AS Integer) AS Byte
  RETURN toByte(bits::band(bits::sr(packed, 16), 255))
END FUNC

FUNC __term_colorG(packed AS Integer) AS Byte
  RETURN toByte(bits::band(bits::sr(packed, 8), 255))
END FUNC

FUNC __term_colorB(packed AS Integer) AS Byte
  RETURN toByte(bits::band(packed, 255))
END FUNC

' Apply the run's foreground: a packed color when set, else fall back to the pen
' the drawText call inherited (`saved`), so an unset run draws in the ambient
' foreground rather than whatever the previous run left. Background is symmetric.
SUB __term_applyFg(packed AS Integer, saved AS term::TermColor)
  IF packed = -1 THEN
    term::setForeground(saved.r, saved.g, saved.b)
  ELSE
    term::setForeground(__term_colorR(packed), __term_colorG(packed), __term_colorB(packed))
  END IF
END SUB

SUB __term_applyBg(packed AS Integer, saved AS term::TermColor)
  IF packed = -1 THEN
    term::setBackground(saved.r, saved.g, saved.b)
  ELSE
    term::setBackground(__term_colorR(packed), __term_colorG(packed), __term_colorB(packed))
  END IF
END SUB

SUB __term_drawTextAttr(x AS Integer, y AS Integer, value AS AttributedString)
  IF term::isOn() THEN
    LET text AS String = toString(value)
    LET n AS Integer = len(strings::toScalars(value))
    LET saveBold AS Boolean = term::getBold()
    LET saveUnderline AS Boolean = term::getUnderline()
    LET saveFg AS term::TermColor = term::getForeground()
    LET saveBg AS term::TermColor = term::getBackground()
    MUT col AS Integer = x
    MUT i AS Integer = 0
    WHILE i < n
      LET st AS __TermStyle = __term_styleAt(value, i)
      MUT j AS Integer = i + 1
      WHILE j < n AND __term_styleAt(value, j) = st
        j = j + 1
      END WHILE
      LET seg AS String = strings::mid(text, i, j - i)
      term::setBold(st.bold)
      term::setUnderline(st.underline)
      __term_applyFg(st.fg, saveFg)
      __term_applyBg(st.bg, saveBg)
      term::drawText(col, y, seg)
      col = col + strings::displayWidth(seg)
      i = j
    END WHILE
    term::setBold(saveBold)
    term::setUnderline(saveUnderline)
    term::setForeground(saveFg.r, saveFg.g, saveFg.b)
    term::setBackground(saveBg.r, saveBg.g, saveBg.b)
  END IF
END SUB"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper {
        name: "term_astrings_bridge",
        gate: HelperGate::WhenBothImported("term", "astrings"),
        body: Some(BODY),
        import_name: None,
    });
}
