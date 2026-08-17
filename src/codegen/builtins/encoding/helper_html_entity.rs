//! `__encoding_htmlEntity` — shared private helper for the `encoding` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Map a named HTML entity (without the surrounding & and ;) to its code point,
' or -1 when unknown. Covers the core five plus the common HTML named set.
FUNC __encoding_htmlEntity(name AS String) AS Integer
  IF name = "amp" THEN
    RETURN 38
  END IF
  IF name = "lt" THEN
    RETURN 60
  END IF
  IF name = "gt" THEN
    RETURN 62
  END IF
  IF name = "quot" THEN
    RETURN 34
  END IF
  IF name = "apos" THEN
    RETURN 39
  END IF
  IF name = "nbsp" THEN
    RETURN 160
  END IF
  IF name = "copy" THEN
    RETURN 169
  END IF
  IF name = "reg" THEN
    RETURN 174
  END IF
  IF name = "trade" THEN
    RETURN 8482
  END IF
  IF name = "hellip" THEN
    RETURN 8230
  END IF
  IF name = "mdash" THEN
    RETURN 8212
  END IF
  IF name = "ndash" THEN
    RETURN 8211
  END IF
  IF name = "lsquo" THEN
    RETURN 8216
  END IF
  IF name = "rsquo" THEN
    RETURN 8217
  END IF
  IF name = "ldquo" THEN
    RETURN 8220
  END IF
  IF name = "rdquo" THEN
    RETURN 8221
  END IF
  IF name = "euro" THEN
    RETURN 8364
  END IF
  IF name = "pound" THEN
    RETURN 163
  END IF
  IF name = "cent" THEN
    RETURN 162
  END IF
  IF name = "yen" THEN
    RETURN 165
  END IF
  IF name = "sect" THEN
    RETURN 167
  END IF
  IF name = "deg" THEN
    RETURN 176
  END IF
  IF name = "plusmn" THEN
    RETURN 177
  END IF
  IF name = "times" THEN
    RETURN 215
  END IF
  IF name = "divide" THEN
    RETURN 247
  END IF
  IF name = "frac12" THEN
    RETURN 189
  END IF
  IF name = "frac14" THEN
    RETURN 188
  END IF
  IF name = "frac34" THEN
    RETURN 190
  END IF
  IF name = "middot" THEN
    RETURN 183
  END IF
  IF name = "laquo" THEN
    RETURN 171
  END IF
  IF name = "raquo" THEN
    RETURN 187
  END IF
  IF name = "aacute" THEN
    RETURN 225
  END IF
  IF name = "eacute" THEN
    RETURN 233
  END IF
  IF name = "iacute" THEN
    RETURN 237
  END IF
  IF name = "oacute" THEN
    RETURN 243
  END IF
  IF name = "uacute" THEN
    RETURN 250
  END IF
  IF name = "agrave" THEN
    RETURN 224
  END IF
  IF name = "egrave" THEN
    RETURN 232
  END IF
  IF name = "ccedil" THEN
    RETURN 231
  END IF
  IF name = "ntilde" THEN
    RETURN 241
  END IF
  IF name = "uuml" THEN
    RETURN 252
  END IF
  IF name = "ouml" THEN
    RETURN 246
  END IF
  IF name = "auml" THEN
    RETURN 228
  END IF
  IF name = "szlig" THEN
    RETURN 223
  END IF
  RETURN -1
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("encoding_htmlEntity", BODY));
}
