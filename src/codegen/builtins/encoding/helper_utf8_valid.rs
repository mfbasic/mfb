//! `__encoding_utf8Valid` — shared private helper for the `encoding` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Validate that `data` is a well-formed UTF-8 byte sequence (no overlong forms,
' no surrogates, no out-of-range scalar values).
FUNC __encoding_utf8Valid(data AS List OF Byte) AS Boolean
  LET n AS Integer = len(data)
  MUT i AS Integer = 0
  MUT extra AS Integer = 0
  MUT lead AS Integer = 0
  MUT codePoint AS Integer = 0
  MUT minValue AS Integer = 0
  MUT j AS Integer = 0
  MUT cont AS Integer = 0
  WHILE i < n
    lead = toInt(collections::get(data, i))
    IF lead <= 127 THEN
      i = i + 1
    ELSE
      IF lead >= 240 THEN
        IF lead > 247 THEN
          RETURN FALSE
        END IF
        extra = 3
        codePoint = lead - 240
        minValue = 65536
      ELSE
        IF lead >= 224 THEN
          extra = 2
          codePoint = lead - 224
          minValue = 2048
        ELSE
          IF lead >= 194 THEN
            extra = 1
            codePoint = lead - 192
            minValue = 128
          ELSE
            RETURN FALSE
          END IF
        END IF
      END IF
      IF i + extra >= n THEN
        RETURN FALSE
      END IF
      j = 0
      WHILE j < extra
        cont = toInt(collections::get(data, i + 1 + j))
        IF cont < 128 OR cont > 191 THEN
          RETURN FALSE
        END IF
        codePoint = codePoint * 64 + (cont - 128)
        j = j + 1
      END WHILE
      IF codePoint < minValue THEN
        RETURN FALSE
      END IF
      IF codePoint > 1114111 THEN
        RETURN FALSE
      END IF
      IF codePoint >= 55296 AND codePoint <= 57343 THEN
        RETURN FALSE
      END IF
      i = i + 1 + extra
    END IF
  END WHILE
  RETURN TRUE
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("encoding_utf8Valid", BODY));
}
