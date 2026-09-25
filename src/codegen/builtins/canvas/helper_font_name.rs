//! Reading a face's names out of its sfnt `name` table (plan-148-A).
//!
//! The system-font members name faces the way the operating system does — by full
//! name (nameID 4, `"Helvetica Bold"`) and PostScript name (nameID 6,
//! `"Helvetica-Bold"`) — and a collection holds many faces in one file. Choosing the
//! right face therefore means reading each face's own names, which is all this does.
//!
//! **Which record wins.** A `name` table carries the same name in several encodings.
//! The Windows Unicode records (platform 3, encoding 1 or 10) are the ones every modern
//! font has, and US English (language `0x0409`) is the spelling CoreText, fontconfig
//! and DirectWrite report; the Mac Roman record (platform 1, encoding 0) is the fallback
//! older Apple fonts may carry alone. A malformed string answers `""` rather than
//! failing: one bad record in one face must not stop a program choosing another.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const FACE_NAME: &str =
r#"FUNC __canvas_faceName(b AS List OF Byte, dir AS Integer, nameId AS Integer) AS String
  LET table AS Integer = __canvas_faceTable(b, dir, "name")
  IF table < 0 THEN
    RETURN ""
  END IF
  LET count AS Integer = __canvas_beU16(b, table + 2)
  LET strings AS Integer = table + __canvas_beU16(b, table + 4)
  MUT bestRank AS Integer = 0
  MUT bestStart AS Integer = 0
  MUT bestLength AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < count
    LET rec AS Integer = table + 6 + i * 12
    IF rec + 12 <= len(b) AND __canvas_beU16(b, rec + 6) = nameId THEN
      LET platform AS Integer = __canvas_beU16(b, rec)
      LET encodingId AS Integer = __canvas_beU16(b, rec + 2)
      LET language AS Integer = __canvas_beU16(b, rec + 4)
      LET length AS Integer = __canvas_beU16(b, rec + 8)
      LET start AS Integer = strings + __canvas_beU16(b, rec + 10)
      MUT rank AS Integer = 0
      IF platform = 3 AND (encodingId = 1 OR encodingId = 10) THEN
        rank = 2
        IF language = 1033 THEN
          rank = 3
        END IF
      END IF
      IF platform = 1 AND encodingId = 0 THEN
        rank = 1
      END IF
      IF rank > bestRank AND start + length <= len(b) THEN
        bestRank = rank
        bestStart = start
        bestLength = length
      END IF
    END IF
    i = i + 1
  END WHILE
  IF bestRank = 0 THEN
    RETURN ""
  END IF
  IF bestRank = 1 THEN
    LET roman AS String = encoding::codepageDecode(encoding::Codepage.Macintosh, collections::mid(b, bestStart, bestLength)) TRAP(e)
      RETURN ""
    END TRAP
    RETURN roman
  END IF
  MUT units AS List OF Integer = []
  MUT k AS Integer = 0
  WHILE k + 1 < bestLength
    units = collections::append(units, __canvas_beU16(b, bestStart + k))
    k = k + 2
  END WHILE
  LET wide AS String = encoding::utf16Decode(units) TRAP(e)
    RETURN ""
  END TRAP
  RETURN wide
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("canvas_faceName", FACE_NAME));
}
