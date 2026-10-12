//! `__net_ipv4Host` — shared private helper for the `net` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-979: the WHATWG "ends in a number" checker and IPv4 parser over a host's
' bytes. Returns "" when the host does not end in a number (it is a name, not an
' address), the canonical dotted-decimal address when it is a valid IPv4 address
' in any accepted spelling (`0x7f.1`, `2130706433`, `0177.0.0.1`, `127.1` are all
' `127.0.0.1`), and fails when it ends in a number but is not one (`256.0.0.1`,
' `1.2.3.09`, `foo.0x4`, five parts). One trailing dot is ignored.
FUNC __net_ipv4Host(b AS List OF Byte) AS String
  MUT n AS Integer = len(b)
  IF n > 0 THEN
    IF __net_byteAt(b, n - 1) = 46 THEN
      n = n - 1
    END IF
  END IF
  ' The last dot-separated part decides whether this is an address at all.
  MUT lastStart AS Integer = n
  MUT allDigits AS Boolean = TRUE
  MUT scanning AS Boolean = TRUE
  WHILE scanning
    IF lastStart = 0 THEN
      scanning = FALSE
    ELSEIF __net_byteAt(b, lastStart - 1) = 46 THEN
      scanning = FALSE
    ELSE
      lastStart = lastStart - 1
      LET c AS Integer = __net_byteAt(b, lastStart)
      IF c < 48 OR c > 57 THEN
        allDigits = FALSE
      END IF
    END IF
  END WHILE
  IF lastStart = n THEN
    RETURN ""
  END IF
  IF allDigits = FALSE AND __net_ipv4Number(b, lastStart, n) < 0 THEN
    RETURN ""
  END IF
  ' It ends in a number, so it must parse as one to four numbers.
  MUT address AS Integer = 0
  MUT count AS Integer = 0
  MUT partStart AS Integer = 0
  MUT i AS Integer = 0
  WHILE i <= n
    MUT atDot AS Boolean = TRUE
    IF i < n THEN
      IF __net_byteAt(b, i) <> 46 THEN
        atDot = FALSE
      END IF
    END IF
    IF atDot THEN
      LET value AS Integer = __net_ipv4Number(b, partStart, i)
      count = count + 1
      IF value < 0 OR count > 4 THEN
        FAIL error(77050003, "invalid URL: invalid IPv4 address")
      END IF
      ' Every part but the last is one byte; the last fills what is left.
      MUT limit AS Integer = 256
      MUT scale AS Integer = 1
      MUT k AS Integer = count
      WHILE k < 4
        IF i = n THEN
          limit = limit * 256
        ELSE
          scale = scale * 256
        END IF
        k = k + 1
      END WHILE
      IF value >= limit THEN
        FAIL error(77050003, "invalid URL: invalid IPv4 address")
      END IF
      address = address + value * scale
      partStart = i + 1
    END IF
    i = i + 1
  END WHILE
  RETURN toString(address / 16777216) & "." & toString((address / 65536) MOD 256) & "." & toString((address / 256) MOD 256) & "." & toString(address MOD 256)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("net_ipv4Host", BODY));
}
