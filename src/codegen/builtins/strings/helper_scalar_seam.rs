//! The scalar-seam source chunk (plan-41-D): `__strings_toScalars` /
//! `__strings_fromScalars` and the five classification predicates
//! (`__strings_isLetter`/`isDigit`/`isWhitespace`/`isUpper`/`isLower`), backing the
//! seven `Body::Rewrite` members, plus the Unicode general-category table appended
//! at registration (`__regex_genCat` renamed to `__strings_genCat`).
//!
//! Why this is a gated helper chunk and not seven `Body::mfb` bodies on the member
//! descriptors: the predicates depend on the 4099-arm `__strings_genCat` table, and
//! a `Body::Mfb` body renders into `get_mfb` for EVERY `IMPORT strings` program —
//! the `WhenUsed` gate exists precisely so that table is compiled only when a
//! program references the seam members. A gated chunk is its own synthetic file
//! (`<builtin-strings>`) and injected FUNCs are file-local, so the seam, the
//! predicates, and the table must stay together in one chunk. A second gate — same
//! body, deduped by the shared `"strings"` name — rides the seam in whenever
//! `astrings` is imported, because the injected `astrings` companion calls the seam
//! after this generic pass has run (plan-99 PART B). Body byte-significant; do not
//! reformat.

use crate::codegen::registry::{HelperGate, RegistryHelper, RegistryPackage};

/// The Unicode general-category table, `__regex_genCat` renamed to `__strings_genCat`
/// so `strings`' file-local copy never collides with `regex`' when both are imported
/// (bug-339 B1: one SOURCE of truth, one COMPILED copy per package — language-mandated
/// because an injected builtin source is one file whose FUNCs are file-local).
const GENCAT_TABLE: &str = include_str!("../../string/unicode/unicode_gencat.mfb");

#[rustfmt::skip]
const SEAM: &str =
r#"REM MFBASIC strings scalar companion (plan-41-D). Internal helpers backing the
REM native strings::toScalars / strings::fromScalars seam and the five scalar
REM classification predicates. The Unicode general-category table
REM (__strings_genCat) is appended from unicode_gencat.mfb at build time with its
REM function renamed from __regex_genCat (see strings.rs::source_file), so it
REM never collides with the regex companion's own copy when both are imported.

IMPORT collections
IMPORT encoding

REM ---------------------------------------------------------------------------
REM String <-> List OF Scalar seam
REM ---------------------------------------------------------------------------

REM Decode a String into its Unicode scalars in order. `utf32Encode` walks the
REM UTF-8 once yielding valid code points, so `toScalar` never actually fails;
REM the inline TRAP is unreachable and only keeps the function total (infallible).
FUNC __strings_toScalars(s AS String) AS List OF Scalar
  MUT out AS List OF Scalar = []
  LET cps AS List OF Integer = encoding::utf32Encode(s)
  FOR EACH cp IN cps
    LET sc AS Scalar = toScalar(cp) TRAP(err)
      RETURN out
    END TRAP
    out = collections::append(out, sc)
  NEXT
  RETURN out
END FUNC

REM Rebuild a String from scalars in order. Every Scalar is a valid, non-surrogate
REM code point, so `utf32Decode` never fails; the inline TRAP is unreachable.
FUNC __strings_fromScalars(scalars AS List OF Scalar) AS String
  MUT cps AS List OF Integer = []
  FOR EACH sc IN scalars
    cps = collections::append(cps, toInt(sc))
  NEXT
  LET result AS String = encoding::utf32Decode(cps) TRAP(err)
    RETURN ""
  END TRAP
  RETURN result
END FUNC

REM ---------------------------------------------------------------------------
REM Scalar classification predicates (total over every Scalar)
REM ---------------------------------------------------------------------------

' plan-64 G2: ASCII fast path in each classification predicate. For cp < 0x80 a
' direct range test reproduces genCat's category exactly (ASCII: A-Z=Lu 65-90,
' a-z=Ll 97-122, 0-9=Nd 48-57, space=Zs 32; no other ASCII scalar is a letter,
' digit, or Unicode space), skipping the 4099-arm __strings_genCat scan and its
' String return/compare on the hot path. cp >= 0x80 keeps the genCat path.
FUNC __strings_isLetter(sc AS Scalar) AS Boolean
  LET cp AS Integer = toInt(sc)
  IF cp < 128 THEN
    RETURN (cp >= 65 AND cp <= 90) OR (cp >= 97 AND cp <= 122)
  END IF
  LET cat AS String = __strings_genCat(cp)
  RETURN cat = "Lu" OR cat = "Ll" OR cat = "Lt" OR cat = "Lm" OR cat = "Lo"
END FUNC

FUNC __strings_isDigit(sc AS Scalar) AS Boolean
  LET cp AS Integer = toInt(sc)
  IF cp < 128 THEN
    RETURN cp >= 48 AND cp <= 57
  END IF
  RETURN __strings_genCat(cp) = "Nd"
END FUNC

FUNC __strings_isWhitespace(sc AS Scalar) AS Boolean
  LET cp AS Integer = toInt(sc)
  IF cp < 128 THEN
    RETURN (cp >= 9 AND cp <= 13) OR cp = 32
  END IF
  LET cat AS String = __strings_genCat(cp)
  IF cat = "Zs" OR cat = "Zl" OR cat = "Zp" THEN
    RETURN TRUE
  END IF
  IF cp >= 9 AND cp <= 13 THEN
    RETURN TRUE
  END IF
  IF cp = 133 THEN
    RETURN TRUE
  END IF
  RETURN FALSE
END FUNC

FUNC __strings_isUpper(sc AS Scalar) AS Boolean
  LET cp AS Integer = toInt(sc)
  IF cp < 128 THEN
    RETURN cp >= 65 AND cp <= 90
  END IF
  RETURN __strings_genCat(cp) = "Lu"
END FUNC

FUNC __strings_isLower(sc AS Scalar) AS Boolean
  LET cp AS Integer = toInt(sc)
  IF cp < 128 THEN
    RETURN cp >= 97 AND cp <= 122
  END IF
  RETURN __strings_genCat(cp) = "Ll"
END FUNC"#;

/// The seam members whose reference opens the `WhenUsed` gate.
const SEAM_MEMBERS: &[&str] = &[
    "toScalars",
    "fromScalars",
    "isLetter",
    "isDigit",
    "isWhitespace",
    "isUpper",
    "isLower",
];

pub(crate) fn register(pkg: &mut RegistryPackage) {
    // The registry is built once behind a `OnceLock`, so the leak is a bounded
    // one-time allocation (the concatenated seam + renamed table).
    let body: &'static str = Box::leak(
        format!(
            "{}\n{}",
            SEAM,
            GENCAT_TABLE.replace("__regex_genCat", "__strings_genCat"),
        )
        .into_boxed_str(),
    );
    pkg.add_helper(RegistryHelper {
        name: "strings",
        gate: HelperGate::WhenUsed(SEAM_MEMBERS),
        body: Some(body),
        import_name: None,
    });
    pkg.add_helper(RegistryHelper {
        name: "strings",
        gate: HelperGate::WhenImported("astrings"),
        body: Some(body),
        import_name: None,
    });
}
