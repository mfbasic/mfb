//! `__json_validNumber` — shared private helper for the `json` package.
//!
//! bug-510 (DEC-03): validates the number token in place over the document's
//! bytes. It used to take the token as a `String` and graphemize it, and every
//! compare fetched a one-character `String` out of that list; on an array of
//! 400 000 one-digit numbers the number path cost ~2.2 KB per element against
//! ~660 B for a literal, and this helper was most of the difference. The JSON
//! number grammar is pure ASCII, so a byte compare is exact and any byte `>= 128`
//! simply fails it — the same verdict the grapheme scan reached, since a cluster
//! carrying a non-ASCII mark never equalled a digit.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Is bytes[startIndex..endIndex) a JSON number? `-`? then `0` | [1-9][0-9]*, then
' (`.` [0-9]+)?, then ([eE] [+-]? [0-9]+)?, and nothing else. Codes: 45 `-`, 46 `.`,
' 43 `+`, 48..57 digits, 69 `E`, 101 `e`.
FUNC __json_validNumber(bytes AS List OF Byte, startIndex AS Integer, endIndex AS Integer) AS Boolean
  MUT index AS Integer = startIndex
  IF index >= endIndex THEN
    RETURN FALSE
  END IF
  MUT code AS Integer = toInt(collections::get(bytes, index))
  IF code = 45 THEN
    index = index + 1
    IF index >= endIndex THEN
      RETURN FALSE
    END IF
    code = toInt(collections::get(bytes, index))
  END IF
  IF code = 48 THEN
    index = index + 1
  ELSEIF code >= 49 AND code <= 57 THEN
    index = __json_consumeDigits(bytes, index + 1, endIndex)
  ELSE
    RETURN FALSE
  END IF
  IF index < endIndex THEN
    IF toInt(collections::get(bytes, index)) = 46 THEN
      index = index + 1
      IF index >= endIndex THEN
        RETURN FALSE
      END IF
      code = toInt(collections::get(bytes, index))
      IF code < 48 OR code > 57 THEN
        RETURN FALSE
      END IF
      index = __json_consumeDigits(bytes, index + 1, endIndex)
    END IF
  END IF
  IF index < endIndex THEN
    code = toInt(collections::get(bytes, index))
    IF code = 101 OR code = 69 THEN
      index = index + 1
      IF index < endIndex THEN
        code = toInt(collections::get(bytes, index))
        IF code = 43 OR code = 45 THEN
          index = index + 1
        END IF
      END IF
      IF index >= endIndex THEN
        RETURN FALSE
      END IF
      code = toInt(collections::get(bytes, index))
      IF code < 48 OR code > 57 THEN
        RETURN FALSE
      END IF
      index = __json_consumeDigits(bytes, index + 1, endIndex)
    END IF
  END IF
  RETURN index = endIndex
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_validNumber", BODY));
}
