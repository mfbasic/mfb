//! `__regex_asciiClassBitset` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' plan-77 R4: precompute a 0..127 ASCII membership table for a character class so
' __regex_classMatch does an O(1) lookup per input position instead of an
' O(items) scan (plus its string comparisons). Built ONCE when the class node is
' constructed, using the exact same logic __regex_classMatch would run for a
' non-ASCII scalar — __regex_classMatchOne with case folding applied when the
' class is case-insensitive. `neg` is NOT baked in here; __regex_classMatch
' negates the looked-up bit so a single table serves both `[…]` and `[^…]`.
FUNC __regex_asciiClassBitset(items AS List OF __regex_ClassItem, fold AS Boolean) AS List OF Boolean
  MUT bits AS List OF Boolean = []
  MUT cp AS Integer = 0
  WHILE cp <= 127
    LET ch AS String = encoding::utf32Decode([cp])
    MUT hit AS Boolean = __regex_classMatchOne(items, ch, cp)
    IF fold AND hit = FALSE THEN
      LET lc AS String = strings::lower(ch)
      IF lc <> ch AND len(lc) = 1 THEN
        IF __regex_classMatchOne(items, lc, __regex_scalarToCp(lc)) THEN
          hit = TRUE
        END IF
      END IF
    END IF
    IF fold AND hit = FALSE THEN
      LET uc AS String = strings::upper(ch)
      IF uc <> ch AND len(uc) = 1 THEN
        IF __regex_classMatchOne(items, uc, __regex_scalarToCp(uc)) THEN
          hit = TRUE
        END IF
      END IF
    END IF
    bits = collections::append(bits, hit)
    cp = cp + 1
  END WHILE
  RETURN bits
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_asciiClassBitset", BODY));
}
