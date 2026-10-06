//! `__http_percentEncodeTarget` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-981: `__http_requestTarget` used to copy the URL's path+query verbatim
' into the request line (`METHOD target HTTP/1.1`). RFC 9112 allows no SP, DEL
' or non-ASCII byte in a request-target, so a `net::Url` built from a path
' with a space (e.g. "/x HTTP/1.0") or raw UTF-8 let the caller inject a
' second whitespace-separated token onto the request line — observed as
' `GET /x HTTP/1.0 HTTP/1.1`, a second, attacker-chosen "HTTP version" token
' that some intermediaries parse as a smuggled pipeline. Percent-encoding
' those three byte classes (matching Node's `new URL(...).pathname + search`)
' keeps the target a single token.
' Every other byte — including the target's own structural delimiters
' `/ ? = & ; : @ , + $ ! * ' ( ) #` — passes through unchanged, so the path and
' query are not otherwise altered. An escape already present (`%` followed by
' two hex digits) is copied verbatim, never double-encoded; a lone `%` that is
' not the start of a valid escape is itself encoded to `%25` so the result
' always re-parses as a validly escaped target. Control bytes (below 0x20) are
' left alone here; they are rejected separately by `__http_hasControlBytes`.
FUNC __http_percentEncodeTarget(s AS String) AS String
  MUT out AS String = ""
  LET n AS Integer = len(s)
  MUT i AS Integer = 0
  WHILE i < n
    LET c AS String = strings::mid(s, i, 1)
    IF c = "%" AND i + 2 < n AND __http_isHexDigitChar(strings::mid(s, i + 1, 1)) AND __http_isHexDigitChar(strings::mid(s, i + 2, 1)) THEN
      out = out & "%" & strings::mid(s, i + 1, 2)
      i = i + 3
    ELSEIF c = "%" THEN
      out = out & "%25"
      i = i + 1
    ELSE
      FOR EACH b IN strings::toBytes(c)
        LET v AS Integer = toInt(b)
        IF v = 32 OR v = 127 OR v >= 128 THEN
          out = out & __http_percentByteEncode(v)
        ELSE
          out = out & c
        END IF
      NEXT
      i = i + 1
    END IF
  END WHILE
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_percentEncodeTarget", BODY));
}
