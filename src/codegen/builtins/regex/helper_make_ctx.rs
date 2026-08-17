//! `__regex_makeCtx` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_makeCtx(value AS String) AS __regex_Ctx
  LET cps AS List OF Integer = encoding::utf32Encode(value)
  MUT chars AS List OF String = []
  FOR EACH cp IN cps
    chars = collections::append(chars, encoding::utf32Decode([cp]))
  NEXT
  RETURN __regex_Ctx[chars, cps, len(chars)]
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_makeCtx", BODY));
}
