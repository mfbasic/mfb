//! `__regex_compile` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_compile(pattern AS String) AS __regex_Program
  LET pat AS List OF String = __regex_toScalars(pattern)
  LET flags0 AS __regex_Flags = __regex_Flags[FALSE, FALSE, FALSE, FALSE, FALSE]
  LET names0 AS Map OF String TO Integer = Map OF String TO Integer {}
  LET parsed AS __regex_Parse = __regex_parseAlt(pat, len(pat), 0, flags0, 0, names0, 0)
  IF parsed.nxt <> len(pat) THEN
    FAIL error(77050003, "invalid regex")
  END IF
  RETURN __regex_Program[parsed.node, parsed.groups, parsed.names]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_compile", BODY));
}
