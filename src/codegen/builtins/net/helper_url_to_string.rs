//! `__net_urlToString` — shared private helper for the `net` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r##"' Render a `Url` back to an absolute href — the inverse of `toUrl`. The compiler
' routes a universal `toString(url)` call to this helper (plan-03-http.md §A.3);
' the `__`-prefixed name is internalized so it never collides with the builtin
' `toString` symbol.
FUNC __net_urlToString(value AS Url) AS String
  MUT out AS String = value.scheme & "://"
  IF value.username <> "" OR value.password <> "" THEN
    out = out & value.username
    IF value.password <> "" THEN
      out = out & ":" & value.password
    END IF
    out = out & "@"
  END IF
  IF strings::contains(value.host, ":") THEN
    out = out & "[" & value.host & "]"
  ELSE
    out = out & value.host
  END IF
  IF value.port <> __net_defaultPort(value.scheme) THEN
    out = out & ":" & toString(value.port)
  END IF
  out = out & value.path
  IF value.query <> "" THEN
    out = out & "?" & value.query
  END IF
  IF value.fragment <> "" THEN
    out = out & "#" & value.fragment
  END IF
  RETURN out
END FUNC"##;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("net_urlToString", BODY));
}
