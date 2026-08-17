//! `__regex_classMatchOne` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_classMatchOne(items AS List OF __regex_ClassItem, ch AS String, cp AS Integer) AS Boolean
  FOR EACH item IN items
    MATCH item
      CASE __regex_Range(rng)
        IF ch >= rng.lo AND ch <= rng.hi THEN
          RETURN TRUE
        END IF
      CASE __regex_Single(sng)
        IF ch = sng.ch THEN
          RETURN TRUE
        END IF
      CASE __regex_Short(sh)
        IF __regex_shorthandMatch(sh.kind, cp) THEN
          RETURN TRUE
        END IF
      CASE __regex_Prop(pr)
        IF __regex_propMatchItem(pr.name, pr.neg, cp) THEN
          RETURN TRUE
        END IF
    END MATCH
  NEXT
  RETURN FALSE
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_classMatchOne", BODY));
}
