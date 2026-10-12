//! `__net_host` — shared private helper for the `net` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r##"' bug-979: validate the host text of an http(s) URL and return the host to
' store. `bracketed` is TRUE for the text between `[` and `]`, which must be an
' IPv6 address and is kept as written. Any other host is checked in its
' percent-decoded form, as WHATWG does: a malformed escape, a decoding that is
' not UTF-8, or a forbidden character (a C0 control, space, DEL or one of
' `#%/:<>?@[\]^|`) fails. A host that ends in a number is an IPv4 address and is
' returned in canonical dotted-decimal form; a name is returned as written.
FUNC __net_host(text AS String, bracketed AS Boolean) AS String
  IF bracketed THEN
    IF __net_isIpv6(strings::toBytes(text)) = FALSE THEN
      FAIL error(77050003, "invalid URL: invalid IPv6 literal")
    END IF
    RETURN text
  END IF
  MUT decoded AS String = text
  IF strings::contains(text, "%") THEN
    decoded = __net_percentDecodeImpl(text, FALSE) TRAP(err)
      FAIL error(77050003, "invalid URL: invalid percent-escape in host")
    END TRAP
  END IF
  LET b AS List OF Byte = strings::toBytes(decoded)
  LET n AS Integer = len(b)
  MUT i AS Integer = 0
  WHILE i < n
    LET c AS Integer = __net_byteAt(b, i)
    IF c <= 32 OR c = 35 OR c = 37 OR c = 47 OR c = 58 OR c = 60 OR c = 62 OR c = 63 OR c = 64 OR (c >= 91 AND c <= 94) OR c = 124 OR c = 127 THEN
      FAIL error(77050003, "invalid URL: forbidden character in host")
    END IF
    i = i + 1
  END WHILE
  LET address AS String = __net_ipv4Host(b)
  IF address <> "" THEN
    RETURN address
  END IF
  RETURN text
END FUNC"##;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("net_host", BODY));
}
