//! `strings::lineBreaks` and its `strings::LineBreak` record — the Unicode Line Breaking
//! Algorithm, UAX #14 for Unicode 16.0.0, untailored (plan-162-E).
//!
//! The member rewrites to `__strings_lineBreaks`, in a chunk gated `WhenUsed`, so only a
//! program that asks for line breaks compiles the algorithm (a `Body::Mfb` body would
//! ride into every `IMPORT strings` program; `helper_scalar_seam.rs` explains the gate).
//!
//! **Classes.** `strings::lineBreakClass` answers each scalar's class after LB1's
//! default resolutions, with flag bits for the properties the rules read beyond the
//! class (`gen_linebreak_table.py`). The class numbers below are that generator's
//! `CLASSES` order.
//!
//! **LB9 first, as units.** LB9 says a combining sequence `X (CM | ZWJ)*` takes X's class
//! in every later rule, its marks ignored; LB10 says any CM or ZWJ left over is AL (with
//! U+0041's properties). So the text is first cut into **units** — a base and the marks it
//! absorbs — and every rule after LB9 reads units, never scalars: `A` and `B` are the units
//! either side of the position, and look-behinds (`SP*`, `(SY | IS)*`, regional-indicator
//! counts) walk units. The only positions considered are unit starts: inside a unit no
//! rule can break, because LB9 does not absorb after BK, CR, LF, NL, SP or ZW — the only
//! classes rules LB4–LB8 break after. A unit remembers whether it ends in ZWJ, for LB8a.
//!
//! Each position runs the rules in order and the first that matches decides it — the
//! rule list read top to bottom, exactly as the report states it. The test file
//! `LineBreakTest-16.0.0.txt` is the oracle (`tests/runtime/rt_strings_line_breaks.rs`).

use crate::codegen::registry::{
    Body, DefaultValue, HelperGate, Implementation, Parameter, RecordProp, RegistryFunction,
    RegistryHelper, RegistryPackage, RegistryRecord,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Where a line of text may break, and where it must."#;

const DESC: &str = r#"`lineBreaks` returns every position in `text` where a line may start, following the
Unicode Line Breaking Algorithm (Unicode Standard Annex #14, for Unicode 16.0). Each
`strings::LineBreak` has an `index` — a position in Unicode scalars, the unit
`strings::mid`, `strings::left` and `strings::find` count in, so `strings::left(text,
b.index)` is the text before the break — and `mandatory`, which is `TRUE` where the line
**must** break: after a line feed, a carriage return (but not between a CR and the LF
that follows it), a form feed, or a line or paragraph separator.

The positions are in order. Position 0 is never reported, because a line cannot be
empty at the start. The end of the text is always reported, as `mandatory`, so a
wrapping loop needs no special case for its last line. An empty string has no breaks
at all and returns an empty list.

This is the rule set that gets text in many writing systems right where splitting on
spaces does not: a break may fall between two Chinese or Japanese characters with no
space between them; never before a closing bracket, `!` or `.`, or after an opening one;
after a hyphen but not before it; never inside a number like `$(12.35)` or `1,000.00`;
never inside an emoji sequence, a flag, or a letter with its accents; and a no-break space
or word joiner keeps its neighbours together.

What it does not do. There is no tailoring for a language or a style: Korean is broken
between syllables, and the CSS `word-break` and `line-break` options are not offered.
Thai, Lao, Khmer and Burmese, which need a dictionary to find word boundaries, break only
at spaces and punctuation. And it never inserts a hyphen: an existing `-` or soft
hyphen is a place a line may break, nothing more.

Wrapping is your loop: walk the breaks, measure the text from the line's start to each
one, and break at the last one that still fits — with `canvas::measureText` for drawn
text, or with a character count for a terminal, as the example does."#;

const EX: &str = r#"```
IMPORT collections
IMPORT io
IMPORT strings

' Wrap `text` to lines of at most `width` characters, breaking only where the
' algorithm allows. A word longer than the width gets a line of its own.
FUNC wrap(text AS String, width AS Integer) AS List OF String
  MUT lines AS List OF String = []
  MUT start AS Integer = 0
  MUT last AS Integer = 0
  FOR EACH b IN strings::lineBreaks(text)
    IF b.index - start > width AND last > start THEN
      lines = collections::append(lines, strings::trimEnd(strings::mid(text, start, last - start)))
      start = last
    END IF
    IF b.mandatory THEN
      lines = collections::append(lines, strings::trimEnd(strings::mid(text, start, b.index - start)))
      start = b.index
    END IF
    last = b.index
  NEXT
  RETURN lines
END FUNC

FUNC main() AS Integer
  FOR EACH line IN wrap("The price is $(12.35) — about €11 — per box, (tax-free).", 20)
    io::print("|" & line & "|")
  NEXT
  RETURN 0
END FUNC
```"#;

#[rustfmt::skip]
const LINE_BREAKS: &str =
r#"REM UAX #14 for Unicode 16.0.0 (plan-162-E). See func_line_breaks.rs.
REM Its own synthetic file, so it names the packages it calls.
IMPORT collections
IMPORT strings

LET __STRINGS_LB_BK AS Integer = 0
LET __STRINGS_LB_CR AS Integer = 1
LET __STRINGS_LB_LF AS Integer = 2
LET __STRINGS_LB_NL AS Integer = 3
LET __STRINGS_LB_SP AS Integer = 4
LET __STRINGS_LB_ZW AS Integer = 5
LET __STRINGS_LB_ZWJ AS Integer = 6
LET __STRINGS_LB_CM AS Integer = 7
LET __STRINGS_LB_WJ AS Integer = 8
LET __STRINGS_LB_GL AS Integer = 9
LET __STRINGS_LB_BA AS Integer = 10
LET __STRINGS_LB_BB AS Integer = 11
LET __STRINGS_LB_B2 AS Integer = 12
LET __STRINGS_LB_HY AS Integer = 13
LET __STRINGS_LB_CB AS Integer = 14
LET __STRINGS_LB_CL AS Integer = 15
LET __STRINGS_LB_CP AS Integer = 16
LET __STRINGS_LB_EX AS Integer = 17
LET __STRINGS_LB_IN AS Integer = 18
LET __STRINGS_LB_NS AS Integer = 19
LET __STRINGS_LB_OP AS Integer = 20
LET __STRINGS_LB_QU AS Integer = 21
LET __STRINGS_LB_IS AS Integer = 22
LET __STRINGS_LB_NU AS Integer = 23
LET __STRINGS_LB_PO AS Integer = 24
LET __STRINGS_LB_PR AS Integer = 25
LET __STRINGS_LB_SY AS Integer = 26
LET __STRINGS_LB_AL AS Integer = 27
LET __STRINGS_LB_HL AS Integer = 28
LET __STRINGS_LB_ID AS Integer = 29
LET __STRINGS_LB_EB AS Integer = 30
LET __STRINGS_LB_EM AS Integer = 31
LET __STRINGS_LB_H2 AS Integer = 32
LET __STRINGS_LB_H3 AS Integer = 33
LET __STRINGS_LB_JL AS Integer = 34
LET __STRINGS_LB_JV AS Integer = 35
LET __STRINGS_LB_JT AS Integer = 36
LET __STRINGS_LB_RI AS Integer = 37
LET __STRINGS_LB_AK AS Integer = 38
LET __STRINGS_LB_AP AS Integer = 39
LET __STRINGS_LB_AS AS Integer = 40
LET __STRINGS_LB_VF AS Integer = 41
LET __STRINGS_LB_VI AS Integer = 42

REM A unit is five words: class, flags (lineBreakClass / 64: bit 0 East Asian F/W/H,
REM bit 1 Extended_Pictographic & Cn, bit 2 Pi, bit 3 Pf), the base's codepoint, the
REM scalar index it starts at, and 1 when it ends in a ZWJ (LB8a).
LET __STRINGS_LB_STRIDE AS Integer = 5

FUNC __strings_lineBreaks(text AS String) AS List OF strings::LineBreak
  MUT breaks AS List OF strings::LineBreak = []
  LET scalars AS List OF Scalar = strings::toScalars(text)
  LET n AS Integer = len(scalars)
  IF n = 0 THEN
    RETURN breaks
  END IF
  LET units AS List OF Integer = __strings_lbUnits(scalars)
  LET count AS Integer = len(units) / __STRINGS_LB_STRIDE
  MUT k AS Integer = 1
  WHILE k < count
    LET verdict AS Integer = __strings_lbAt(units, k)
    IF verdict > 0 THEN
      breaks = collections::append(breaks, strings::LineBreak[index := __strings_lbField(units, k, 3), mandatory := verdict = 2])
    END IF
    k = k + 1
  END WHILE
  ' LB3: always break at the end of text.
  RETURN collections::append(breaks, strings::LineBreak[index := n, mandatory := TRUE])
END FUNC

REM LB1, LB9 and LB10: the scalars as units.
FUNC __strings_lbUnits(scalars AS List OF Scalar) AS List OF Integer
  MUT units AS List OF Integer = []
  MUT i AS Integer = 0
  FOR EACH sc IN scalars
    LET cp AS Integer = toInt(sc)
    LET value AS Integer = strings::lineBreakClass(cp)
    LET cls AS Integer = value MOD 64
    LET mark AS Boolean = cls = __STRINGS_LB_CM OR cls = __STRINGS_LB_ZWJ
    LET last AS Integer = len(units) - __STRINGS_LB_STRIDE
    MUT absorbed AS Boolean = FALSE
    IF mark AND last >= 0 THEN
      LET base AS Integer = collections::getOr(units, last, 0)
      IF base <> __STRINGS_LB_BK AND base <> __STRINGS_LB_CR AND base <> __STRINGS_LB_LF AND base <> __STRINGS_LB_NL AND base <> __STRINGS_LB_SP AND base <> __STRINGS_LB_ZW THEN
        ' LB9: the mark joins the unit before it, which keeps that unit's class.
        MUT ends AS Integer = 0
        IF cls = __STRINGS_LB_ZWJ THEN
          ends = 1
        END IF
        units = collections::set(units, last + 4, ends)
        absorbed = TRUE
      END IF
    END IF
    IF NOT absorbed THEN
      IF mark THEN
        ' LB10: a mark with nothing to join is a letter A.
        units = collections::append(units, __STRINGS_LB_AL)
        units = collections::append(units, 0)
        units = collections::append(units, 65)
        units = collections::append(units, i)
        MUT ends AS Integer = 0
        IF cls = __STRINGS_LB_ZWJ THEN
          ends = 1
        END IF
        units = collections::append(units, ends)
      ELSE
        units = collections::append(units, cls)
        units = collections::append(units, value / 64)
        units = collections::append(units, cp)
        units = collections::append(units, i)
        units = collections::append(units, 0)
      END IF
    END IF
    i = i + 1
  NEXT
  RETURN units
END FUNC

REM Word `field` of unit `k`; a unit past either end reads -1 (sot or eot).
FUNC __strings_lbField(units AS List OF Integer, k AS Integer, field AS Integer) AS Integer
  IF k < 0 OR k * __STRINGS_LB_STRIDE >= len(units) THEN
    RETURN 0 - 1
  END IF
  RETURN collections::getOr(units, k * __STRINGS_LB_STRIDE + field, 0 - 1)
END FUNC

FUNC __strings_lbClass(units AS List OF Integer, k AS Integer) AS Integer
  RETURN __strings_lbField(units, k, 0)
END FUNC

FUNC __strings_lbFlag(units AS List OF Integer, k AS Integer, bit AS Integer) AS Boolean
  LET flags AS Integer = __strings_lbField(units, k, 1)
  IF flags < 0 THEN
    RETURN FALSE
  END IF
  MUT v AS Integer = flags
  MUT b AS Integer = 0
  WHILE b < bit
    v = v / 2
    b = b + 1
  END WHILE
  RETURN v MOD 2 = 1
END FUNC

FUNC __strings_lbEastAsian(units AS List OF Integer, k AS Integer) AS Boolean
  RETURN __strings_lbFlag(units, k, 0)
END FUNC

REM The last unit at or before `k` that is not SP (-1 at sot).
FUNC __strings_lbSkipSpaces(units AS List OF Integer, k AS Integer) AS Integer
  MUT j AS Integer = k
  WHILE j >= 0 AND __strings_lbClass(units, j) = __STRINGS_LB_SP
    j = j - 1
  END WHILE
  RETURN j
END FUNC

REM The last unit at or before `k` that is neither SY nor IS (-1 at sot).
FUNC __strings_lbSkipSyIs(units AS List OF Integer, k AS Integer) AS Integer
  MUT j AS Integer = k
  WHILE j >= 0 AND (__strings_lbClass(units, j) = __STRINGS_LB_SY OR __strings_lbClass(units, j) = __STRINGS_LB_IS)
    j = j - 1
  END WHILE
  RETURN j
END FUNC

REM LB28a's (AK | U+25CC | AS), and its (AK | U+25CC).
FUNC __strings_lbAksara(units AS List OF Integer, k AS Integer, withAs AS Boolean) AS Boolean
  LET c AS Integer = __strings_lbClass(units, k)
  IF c = __STRINGS_LB_AK OR __strings_lbField(units, k, 2) = 9676 THEN
    RETURN TRUE
  END IF
  RETURN withAs AND c = __STRINGS_LB_AS
END FUNC

REM The verdict at the start of unit `k` (k >= 1): 0 no break, 1 a break opportunity,
REM 2 a mandatory break. LB4 to LB31, first match wins.
FUNC __strings_lbAt(units AS List OF Integer, k AS Integer) AS Integer
  LET a AS Integer = __strings_lbClass(units, k - 1)
  LET b AS Integer = __strings_lbClass(units, k)
  LET count AS Integer = len(units) / __STRINGS_LB_STRIDE
  ' LB4, LB5
  IF a = __STRINGS_LB_BK THEN
    RETURN 2
  END IF
  IF a = __STRINGS_LB_CR AND b = __STRINGS_LB_LF THEN
    RETURN 0
  END IF
  IF a = __STRINGS_LB_CR OR a = __STRINGS_LB_LF OR a = __STRINGS_LB_NL THEN
    RETURN 2
  END IF
  ' LB6
  IF b = __STRINGS_LB_BK OR b = __STRINGS_LB_CR OR b = __STRINGS_LB_LF OR b = __STRINGS_LB_NL THEN
    RETURN 0
  END IF
  ' LB7
  IF b = __STRINGS_LB_SP OR b = __STRINGS_LB_ZW THEN
    RETURN 0
  END IF
  ' LB8: ZW SP* ÷
  LET q AS Integer = __strings_lbSkipSpaces(units, k - 1)
  LET qc AS Integer = __strings_lbClass(units, q)
  IF qc = __STRINGS_LB_ZW THEN
    RETURN 1
  END IF
  ' LB8a
  IF __strings_lbField(units, k - 1, 4) = 1 THEN
    RETURN 0
  END IF
  ' LB11
  IF b = __STRINGS_LB_WJ OR a = __STRINGS_LB_WJ THEN
    RETURN 0
  END IF
  ' LB12
  IF a = __STRINGS_LB_GL THEN
    RETURN 0
  END IF
  ' LB12a
  IF b = __STRINGS_LB_GL AND a <> __STRINGS_LB_SP AND a <> __STRINGS_LB_BA AND a <> __STRINGS_LB_HY THEN
    RETURN 0
  END IF
  ' LB13
  IF b = __STRINGS_LB_CL OR b = __STRINGS_LB_CP OR b = __STRINGS_LB_EX OR b = __STRINGS_LB_SY THEN
    RETURN 0
  END IF
  ' LB14: OP SP* ×
  IF qc = __STRINGS_LB_OP THEN
    RETURN 0
  END IF
  ' LB15a: (sot | BK | CR | LF | NL | OP | QU | GL | SP | ZW) [Pi & QU] SP* ×
  IF qc = __STRINGS_LB_QU AND __strings_lbFlag(units, q, 2) THEN
    LET pc AS Integer = __strings_lbClass(units, q - 1)
    IF q = 0 OR pc = __STRINGS_LB_BK OR pc = __STRINGS_LB_CR OR pc = __STRINGS_LB_LF OR pc = __STRINGS_LB_NL OR pc = __STRINGS_LB_OP OR pc = __STRINGS_LB_QU OR pc = __STRINGS_LB_GL OR pc = __STRINGS_LB_SP OR pc = __STRINGS_LB_ZW THEN
      RETURN 0
    END IF
  END IF
  ' LB15b: × [Pf & QU] (SP | GL | WJ | CL | QU | CP | EX | IS | SY | BK | CR | LF | NL | ZW | eot)
  IF b = __STRINGS_LB_QU AND __strings_lbFlag(units, k, 3) THEN
    LET nc AS Integer = __strings_lbClass(units, k + 1)
    IF k + 1 >= count OR nc = __STRINGS_LB_SP OR nc = __STRINGS_LB_GL OR nc = __STRINGS_LB_WJ OR nc = __STRINGS_LB_CL OR nc = __STRINGS_LB_QU OR nc = __STRINGS_LB_CP OR nc = __STRINGS_LB_EX OR nc = __STRINGS_LB_IS OR nc = __STRINGS_LB_SY OR nc = __STRINGS_LB_BK OR nc = __STRINGS_LB_CR OR nc = __STRINGS_LB_LF OR nc = __STRINGS_LB_NL OR nc = __STRINGS_LB_ZW THEN
      RETURN 0
    END IF
  END IF
  ' LB15c: SP ÷ IS NU
  IF a = __STRINGS_LB_SP AND b = __STRINGS_LB_IS AND __strings_lbClass(units, k + 1) = __STRINGS_LB_NU THEN
    RETURN 1
  END IF
  ' LB15d
  IF b = __STRINGS_LB_IS THEN
    RETURN 0
  END IF
  ' LB16: (CL | CP) SP* × NS
  IF b = __STRINGS_LB_NS AND (qc = __STRINGS_LB_CL OR qc = __STRINGS_LB_CP) THEN
    RETURN 0
  END IF
  ' LB17: B2 SP* × B2
  IF b = __STRINGS_LB_B2 AND qc = __STRINGS_LB_B2 THEN
    RETURN 0
  END IF
  ' LB18
  IF a = __STRINGS_LB_SP THEN
    RETURN 1
  END IF
  ' LB19
  IF b = __STRINGS_LB_QU AND NOT __strings_lbFlag(units, k, 2) THEN
    RETURN 0
  END IF
  IF a = __STRINGS_LB_QU AND NOT __strings_lbFlag(units, k - 1, 3) THEN
    RETURN 0
  END IF
  ' LB19a
  IF b = __STRINGS_LB_QU AND NOT __strings_lbEastAsian(units, k - 1) THEN
    RETURN 0
  END IF
  IF b = __STRINGS_LB_QU AND (k + 1 >= count OR NOT __strings_lbEastAsian(units, k + 1)) THEN
    RETURN 0
  END IF
  IF a = __STRINGS_LB_QU AND NOT __strings_lbEastAsian(units, k) THEN
    RETURN 0
  END IF
  IF a = __STRINGS_LB_QU AND (k - 2 < 0 OR NOT __strings_lbEastAsian(units, k - 2)) THEN
    RETURN 0
  END IF
  ' LB20
  IF b = __STRINGS_LB_CB OR a = __STRINGS_LB_CB THEN
    RETURN 1
  END IF
  ' LB20a: (sot | BK | CR | LF | NL | SP | ZW | CB | GL) (HY | U+2010) × AL
  IF (a = __STRINGS_LB_HY OR __strings_lbField(units, k - 1, 2) = 8208) AND b = __STRINGS_LB_AL THEN
    LET pc AS Integer = __strings_lbClass(units, k - 2)
    IF k - 2 < 0 OR pc = __STRINGS_LB_BK OR pc = __STRINGS_LB_CR OR pc = __STRINGS_LB_LF OR pc = __STRINGS_LB_NL OR pc = __STRINGS_LB_SP OR pc = __STRINGS_LB_ZW OR pc = __STRINGS_LB_CB OR pc = __STRINGS_LB_GL THEN
      RETURN 0
    END IF
  END IF
  ' LB21
  IF b = __STRINGS_LB_BA OR b = __STRINGS_LB_HY OR b = __STRINGS_LB_NS THEN
    RETURN 0
  END IF
  IF a = __STRINGS_LB_BB THEN
    RETURN 0
  END IF
  ' LB21a: HL (HY | [BA - EastAsian]) × [^HL]
  IF (a = __STRINGS_LB_HY OR (a = __STRINGS_LB_BA AND NOT __strings_lbEastAsian(units, k - 1))) AND __strings_lbClass(units, k - 2) = __STRINGS_LB_HL AND b <> __STRINGS_LB_HL THEN
    RETURN 0
  END IF
  ' LB21b
  IF a = __STRINGS_LB_SY AND b = __STRINGS_LB_HL THEN
    RETURN 0
  END IF
  ' LB22
  IF b = __STRINGS_LB_IN THEN
    RETURN 0
  END IF
  ' LB23
  IF (a = __STRINGS_LB_AL OR a = __STRINGS_LB_HL) AND b = __STRINGS_LB_NU THEN
    RETURN 0
  END IF
  IF a = __STRINGS_LB_NU AND (b = __STRINGS_LB_AL OR b = __STRINGS_LB_HL) THEN
    RETURN 0
  END IF
  ' LB23a
  IF a = __STRINGS_LB_PR AND (b = __STRINGS_LB_ID OR b = __STRINGS_LB_EB OR b = __STRINGS_LB_EM) THEN
    RETURN 0
  END IF
  IF (a = __STRINGS_LB_ID OR a = __STRINGS_LB_EB OR a = __STRINGS_LB_EM) AND b = __STRINGS_LB_PO THEN
    RETURN 0
  END IF
  ' LB24
  IF (a = __STRINGS_LB_PR OR a = __STRINGS_LB_PO) AND (b = __STRINGS_LB_AL OR b = __STRINGS_LB_HL) THEN
    RETURN 0
  END IF
  IF (a = __STRINGS_LB_AL OR a = __STRINGS_LB_HL) AND (b = __STRINGS_LB_PR OR b = __STRINGS_LB_PO) THEN
    RETURN 0
  END IF
  ' LB25
  IF b = __STRINGS_LB_PO OR b = __STRINGS_LB_PR THEN
    IF (a = __STRINGS_LB_CL OR a = __STRINGS_LB_CP) AND __strings_lbClass(units, __strings_lbSkipSyIs(units, k - 2)) = __STRINGS_LB_NU THEN
      RETURN 0
    END IF
    IF __strings_lbClass(units, __strings_lbSkipSyIs(units, k - 1)) = __STRINGS_LB_NU THEN
      RETURN 0
    END IF
  END IF
  IF (a = __STRINGS_LB_PO OR a = __STRINGS_LB_PR) AND b = __STRINGS_LB_OP THEN
    IF __strings_lbClass(units, k + 1) = __STRINGS_LB_NU THEN
      RETURN 0
    END IF
    IF __strings_lbClass(units, k + 1) = __STRINGS_LB_IS AND __strings_lbClass(units, k + 2) = __STRINGS_LB_NU THEN
      RETURN 0
    END IF
  END IF
  IF (a = __STRINGS_LB_PO OR a = __STRINGS_LB_PR OR a = __STRINGS_LB_HY OR a = __STRINGS_LB_IS) AND b = __STRINGS_LB_NU THEN
    RETURN 0
  END IF
  IF b = __STRINGS_LB_NU AND __strings_lbClass(units, __strings_lbSkipSyIs(units, k - 1)) = __STRINGS_LB_NU THEN
    RETURN 0
  END IF
  ' LB26
  IF a = __STRINGS_LB_JL AND (b = __STRINGS_LB_JL OR b = __STRINGS_LB_JV OR b = __STRINGS_LB_H2 OR b = __STRINGS_LB_H3) THEN
    RETURN 0
  END IF
  IF (a = __STRINGS_LB_JV OR a = __STRINGS_LB_H2) AND (b = __STRINGS_LB_JV OR b = __STRINGS_LB_JT) THEN
    RETURN 0
  END IF
  IF (a = __STRINGS_LB_JT OR a = __STRINGS_LB_H3) AND b = __STRINGS_LB_JT THEN
    RETURN 0
  END IF
  ' LB27
  LET aKorean AS Boolean = a = __STRINGS_LB_JL OR a = __STRINGS_LB_JV OR a = __STRINGS_LB_JT OR a = __STRINGS_LB_H2 OR a = __STRINGS_LB_H3
  LET bKorean AS Boolean = b = __STRINGS_LB_JL OR b = __STRINGS_LB_JV OR b = __STRINGS_LB_JT OR b = __STRINGS_LB_H2 OR b = __STRINGS_LB_H3
  IF aKorean AND b = __STRINGS_LB_PO THEN
    RETURN 0
  END IF
  IF a = __STRINGS_LB_PR AND bKorean THEN
    RETURN 0
  END IF
  ' LB28
  IF (a = __STRINGS_LB_AL OR a = __STRINGS_LB_HL) AND (b = __STRINGS_LB_AL OR b = __STRINGS_LB_HL) THEN
    RETURN 0
  END IF
  ' LB28a
  IF a = __STRINGS_LB_AP AND __strings_lbAksara(units, k, TRUE) THEN
    RETURN 0
  END IF
  IF __strings_lbAksara(units, k - 1, TRUE) AND (b = __STRINGS_LB_VF OR b = __STRINGS_LB_VI) THEN
    RETURN 0
  END IF
  IF a = __STRINGS_LB_VI AND __strings_lbAksara(units, k - 2, TRUE) AND __strings_lbAksara(units, k, FALSE) THEN
    RETURN 0
  END IF
  IF __strings_lbAksara(units, k - 1, TRUE) AND __strings_lbAksara(units, k, TRUE) AND __strings_lbClass(units, k + 1) = __STRINGS_LB_VF THEN
    RETURN 0
  END IF
  ' LB29
  IF a = __STRINGS_LB_IS AND (b = __STRINGS_LB_AL OR b = __STRINGS_LB_HL) THEN
    RETURN 0
  END IF
  ' LB30
  IF (a = __STRINGS_LB_AL OR a = __STRINGS_LB_HL OR a = __STRINGS_LB_NU) AND b = __STRINGS_LB_OP AND NOT __strings_lbEastAsian(units, k) THEN
    RETURN 0
  END IF
  IF a = __STRINGS_LB_CP AND NOT __strings_lbEastAsian(units, k - 1) AND (b = __STRINGS_LB_AL OR b = __STRINGS_LB_HL OR b = __STRINGS_LB_NU) THEN
    RETURN 0
  END IF
  ' LB30a: break between two RIs only after an even number of them.
  IF a = __STRINGS_LB_RI AND b = __STRINGS_LB_RI THEN
    MUT run AS Integer = 0
    MUT j AS Integer = k - 1
    WHILE j >= 0 AND __strings_lbClass(units, j) = __STRINGS_LB_RI
      run = run + 1
      j = j - 1
    END WHILE
    IF run MOD 2 = 1 THEN
      RETURN 0
    END IF
  END IF
  ' LB30b
  IF b = __STRINGS_LB_EM AND (a = __STRINGS_LB_EB OR __strings_lbFlag(units, k - 1, 1)) THEN
    RETURN 0
  END IF
  ' LB31
  RETURN 1
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_record(RegistryRecord {
        name: "LineBreak",
        export: true,
        description: "One place a line of text may start, as `strings::lineBreaks` \
                      returns it.",
        props: vec![
            RecordProp {
                name: "index",
                ty: ParameterType::Integer,
                description: "Where the new line would start, in Unicode scalars from \
                              the start of the text, as `strings::mid` counts. Never 0.",
            },
            RecordProp {
                name: "mandatory",
                ty: ParameterType::Boolean,
                description: "`TRUE` where the line must break — after a hard line \
                              break, and at the end of the text — and `FALSE` where it \
                              only may.",
            },
        ],
    });
    pkg.add_function(RegistryFunction {
        name: "lineBreaks",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "text",
                desc: "The text to find line breaks in. Empty text has none.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::list_of(ParameterType::named("LineBreak")),
            errors: vec![],
            body: Body::Rewrite("__strings_lineBreaks"),
        }],
    });
    pkg.add_helper(RegistryHelper {
        name: "strings_lineBreaks",
        gate: HelperGate::WhenUsed(&["lineBreaks"]),
        body: Some(LINE_BREAKS),
        import_name: None,
        natively_called: false,
    });
}
