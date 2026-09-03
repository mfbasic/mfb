//! `__encoding_codepageTable` — shared private helper for the `encoding` package.
//!
//! PHASE 1 THROWAWAY (plan-123-A): two hand-generated tables only. Phase 2 replaces
//! this file wholesale with the output of `scripts/gen-codepage-tables.py` over the
//! vendored WHATWG index files under `tools/codepage-index/`.
//!
//! Each table is one 128-scalar MFBASIC `String` literal: scalar `i` is the code
//! point for byte `128 + i`, and `\u{FFFD}` marks a byte the codepage leaves
//! unmapped. U+FFFD is an unambiguous sentinel because the highest code point across
//! all 27 WHATWG single-byte index files is U+FB02.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (after the `Codepage` enum, before the member bodies).
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' The 128-scalar high-half table for a single-byte codepage. Scalar i is the code
' point for byte 128 + i; "\u{FFFD}" marks an unmapped byte.
FUNC __encoding_codepageTable(codepage AS Codepage) AS String
  MATCH codepage
    CASE Codepage.Utf8
      RETURN ""
    CASE Codepage.Windows1252
      RETURN "\u{20AC}\u{0081}\u{201A}\u{0192}\u{201E}\u{2026}\u{2020}\u{2021}\u{02C6}\u{2030}\u{0160}\u{2039}\u{0152}\u{008D}\u{017D}\u{008F}\u{0090}\u{2018}\u{2019}\u{201C}\u{201D}\u{2022}\u{2013}\u{2014}\u{02DC}\u{2122}\u{0161}\u{203A}\u{0153}\u{009D}\u{017E}\u{0178}\u{00A0}\u{00A1}\u{00A2}\u{00A3}\u{00A4}\u{00A5}\u{00A6}\u{00A7}\u{00A8}\u{00A9}\u{00AA}\u{00AB}\u{00AC}\u{00AD}\u{00AE}\u{00AF}\u{00B0}\u{00B1}\u{00B2}\u{00B3}\u{00B4}\u{00B5}\u{00B6}\u{00B7}\u{00B8}\u{00B9}\u{00BA}\u{00BB}\u{00BC}\u{00BD}\u{00BE}\u{00BF}\u{00C0}\u{00C1}\u{00C2}\u{00C3}\u{00C4}\u{00C5}\u{00C6}\u{00C7}\u{00C8}\u{00C9}\u{00CA}\u{00CB}\u{00CC}\u{00CD}\u{00CE}\u{00CF}\u{00D0}\u{00D1}\u{00D2}\u{00D3}\u{00D4}\u{00D5}\u{00D6}\u{00D7}\u{00D8}\u{00D9}\u{00DA}\u{00DB}\u{00DC}\u{00DD}\u{00DE}\u{00DF}\u{00E0}\u{00E1}\u{00E2}\u{00E3}\u{00E4}\u{00E5}\u{00E6}\u{00E7}\u{00E8}\u{00E9}\u{00EA}\u{00EB}\u{00EC}\u{00ED}\u{00EE}\u{00EF}\u{00F0}\u{00F1}\u{00F2}\u{00F3}\u{00F4}\u{00F5}\u{00F6}\u{00F7}\u{00F8}\u{00F9}\u{00FA}\u{00FB}\u{00FC}\u{00FD}\u{00FE}\u{00FF}"
    CASE Codepage.Windows874
      RETURN "\u{20AC}\u{0081}\u{0082}\u{0083}\u{0084}\u{2026}\u{0086}\u{0087}\u{0088}\u{0089}\u{008A}\u{008B}\u{008C}\u{008D}\u{008E}\u{008F}\u{0090}\u{2018}\u{2019}\u{201C}\u{201D}\u{2022}\u{2013}\u{2014}\u{0098}\u{0099}\u{009A}\u{009B}\u{009C}\u{009D}\u{009E}\u{009F}\u{00A0}\u{0E01}\u{0E02}\u{0E03}\u{0E04}\u{0E05}\u{0E06}\u{0E07}\u{0E08}\u{0E09}\u{0E0A}\u{0E0B}\u{0E0C}\u{0E0D}\u{0E0E}\u{0E0F}\u{0E10}\u{0E11}\u{0E12}\u{0E13}\u{0E14}\u{0E15}\u{0E16}\u{0E17}\u{0E18}\u{0E19}\u{0E1A}\u{0E1B}\u{0E1C}\u{0E1D}\u{0E1E}\u{0E1F}\u{0E20}\u{0E21}\u{0E22}\u{0E23}\u{0E24}\u{0E25}\u{0E26}\u{0E27}\u{0E28}\u{0E29}\u{0E2A}\u{0E2B}\u{0E2C}\u{0E2D}\u{0E2E}\u{0E2F}\u{0E30}\u{0E31}\u{0E32}\u{0E33}\u{0E34}\u{0E35}\u{0E36}\u{0E37}\u{0E38}\u{0E39}\u{0E3A}\u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}\u{0E3F}\u{0E40}\u{0E41}\u{0E42}\u{0E43}\u{0E44}\u{0E45}\u{0E46}\u{0E47}\u{0E48}\u{0E49}\u{0E4A}\u{0E4B}\u{0E4C}\u{0E4D}\u{0E4E}\u{0E4F}\u{0E50}\u{0E51}\u{0E52}\u{0E53}\u{0E54}\u{0E55}\u{0E56}\u{0E57}\u{0E58}\u{0E59}\u{0E5A}\u{0E5B}\u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}"
  END MATCH
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("encoding_codepageTable", BODY));
}
