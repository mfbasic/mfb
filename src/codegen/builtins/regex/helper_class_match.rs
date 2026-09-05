//! `__regex_classMatch` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_classMatch(cls AS __regex_Class, pos AS Integer, ctx AS __regex_Ctx) AS Boolean
  LET cp AS Integer = collections::get(ctx.cps, pos)
  MUT hit AS Boolean = FALSE
  IF cp >= 0 AND cp <= 127 THEN
    ' plan-77 R4: O(1) precomputed ASCII membership (fold already baked in by
    ' __regex_asciiClassBitset; `neg` is applied below, not in the bitset).
    hit = collections::get(cls.ascii, cp)
  ELSE
    ' bug-510: the character as text, built from its scalar -- the context no longer
    ' carries a String per character.
    LET ch AS String = __regex_chr(cp)
    hit = __regex_classMatchOne(cls.items, ch, cp)
    IF cls.fold AND hit = FALSE THEN
      LET lc AS String = strings::lower(ch)
      IF lc <> ch AND len(lc) = 1 THEN
        IF __regex_classMatchOne(cls.items, lc, __regex_scalarToCp(lc)) THEN
          hit = TRUE
        END IF
      END IF
    END IF
    IF cls.fold AND hit = FALSE THEN
      LET uc AS String = strings::upper(ch)
      IF uc <> ch AND len(uc) = 1 THEN
        IF __regex_classMatchOne(cls.items, uc, __regex_scalarToCp(uc)) THEN
          hit = TRUE
        END IF
      END IF
    END IF
  END IF
  IF cls.neg THEN
    RETURN NOT hit
  END IF
  RETURN hit
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_classMatch", BODY));
}
