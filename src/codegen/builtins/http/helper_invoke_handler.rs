//! `__http_invokeHandler` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Invoke a route's handler on the request, converting any handler failure into
' a 500 (§F.5.1). The handler is a first-class `FUNC(Request) AS Response`;
' MFBASIC invokes a stored function value only through a builtin that calls it,
' so the singleton `collections::transform` applies it exactly once.
FUNC __http_invokeHandler(r AS Route, req AS Request) AS Response
  LET h AS FUNC(Request) AS Response = r.handler
  LET single AS List OF Request = [req]
  LET fallback AS List OF Response = [__http_status(500, "Internal Server Error")]
  MUT out AS List OF Response = fallback
  out = collections::transform(single, h) TRAP(e)
    RECOVER fallback
  END TRAP
  RETURN collections::get(out, 0)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_invokeHandler", BODY));
}
