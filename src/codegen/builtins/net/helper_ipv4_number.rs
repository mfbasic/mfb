//! `__net_ipv4Number` — shared private helper for the `net` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-979: the WHATWG "IPv4 number parser" over the bytes `b[start, stop)`: a
' `0x`/`0X` prefix selects hexadecimal, any other leading `0` on two or more
' characters selects octal, everything else is decimal, and a prefix with nothing
' after it is 0. Returns -1 when the part is empty or holds a character that is
' not a digit of its radix. No part of an address may exceed 4294967295, so the
' value saturates at 4294967296 rather than overflowing on a long digit run.
FUNC __net_ipv4Number(b AS List OF Byte, start AS Integer, stop AS Integer) AS Integer
  IF stop <= start THEN
    RETURN -1
  END IF
  MUT radix AS Integer = 10
  MUT i AS Integer = start
  IF stop - start >= 2 AND __net_byteAt(b, start) = 48 THEN
    LET x AS Integer = __net_byteAt(b, start + 1)
    IF x = 120 OR x = 88 THEN
      radix = 16
      i = start + 2
    ELSE
      radix = 8
      i = start + 1
    END IF
  END IF
  MUT value AS Integer = 0
  WHILE i < stop
    LET c AS Integer = __net_byteAt(b, i)
    MUT digit AS Integer = 99
    IF c >= 48 AND c <= 57 THEN
      digit = c - 48
    ELSEIF c >= 97 AND c <= 102 THEN
      digit = c - 87
    ELSEIF c >= 65 AND c <= 70 THEN
      digit = c - 55
    END IF
    IF digit >= radix THEN
      RETURN -1
    END IF
    value = value * radix + digit
    IF value > 4294967295 THEN
      value = 4294967296
    END IF
    i = i + 1
  END WHILE
  RETURN value
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("net_ipv4Number", BODY));
}
