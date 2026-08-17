//! `__regex_toScalars` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_toScalars(s AS String) AS List OF String
  ' Split into Unicode *scalars* (regex indexes by scalar, never grapheme) in one
  ' O(n) pass: utf32Encode walks the UTF-8 once, then each scalar is materialized
  ' via the in-place MUT append. Replaces the O(n^2) per-index strings::mid(s,i,1)
  ' (each re-walks from the start), which ran on every regex call (plan-02 §6b).
  MUT out AS List OF String = []
  LET cps AS List OF Integer = encoding::utf32Encode(s)
  FOR EACH cp IN cps
    LET one AS List OF Integer = [cp]
    LET scalar AS String = encoding::utf32Decode(one)
    out = collections::append(out, scalar)
  NEXT
  RETURN out
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_toScalars", BODY));
}
