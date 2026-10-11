//! `__net_isIpv6` — shared private helper for the `net` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-979: TRUE when `b` (the text between `[` and `]`) is an IPv6 address by
' the WHATWG "IPv6 parser": up to eight colon-separated groups of one to four
' hex digits, at most one `::` standing for the missing groups (exactly eight
' groups without it), and optionally a dotted-decimal IPv4 address in place of
' the last two groups, its four parts 0..255 with no leading zero. A zone
' (`%eth0`), an IPvFuture literal (`v1.fe`) and anything else are not addresses.
FUNC __net_isIpv6(b AS List OF Byte) AS Boolean
  LET n AS Integer = len(b)
  MUT pieces AS Integer = 0
  MUT compressed AS Boolean = FALSE
  MUT p AS Integer = 0
  IF n > 0 THEN
    IF __net_byteAt(b, 0) = 58 THEN
      IF n < 2 THEN
        RETURN FALSE
      END IF
      IF __net_byteAt(b, 1) <> 58 THEN
        RETURN FALSE
      END IF
      p = 2
      pieces = 1
      compressed = TRUE
    END IF
  END IF
  WHILE p < n
    IF pieces = 8 THEN
      RETURN FALSE
    END IF
    IF __net_byteAt(b, p) = 58 THEN
      IF compressed THEN
        RETURN FALSE
      END IF
      p = p + 1
      pieces = pieces + 1
      compressed = TRUE
    ELSE
      MUT length AS Integer = 0
      MUT hex AS Boolean = TRUE
      WHILE length < 4 AND p < n AND hex
        LET c AS Integer = __net_byteAt(b, p)
        IF (c >= 48 AND c <= 57) OR (c >= 97 AND c <= 102) OR (c >= 65 AND c <= 70) THEN
          p = p + 1
          length = length + 1
        ELSE
          hex = FALSE
        END IF
      END WHILE
      MUT after AS Integer = 0
      IF p < n THEN
        after = __net_byteAt(b, p)
      END IF
      IF after = 46 THEN
        ' An embedded IPv4 address: re-read this group as its first part.
        IF length = 0 OR pieces > 6 THEN
          RETURN FALSE
        END IF
        p = p - length
        MUT numbers AS Integer = 0
        WHILE p < n
          IF numbers > 0 THEN
            IF __net_byteAt(b, p) <> 46 OR numbers >= 4 THEN
              RETURN FALSE
            END IF
            p = p + 1
          END IF
          MUT part AS Integer = -1
          MUT digits AS Boolean = TRUE
          WHILE p < n AND digits
            LET d AS Integer = __net_byteAt(b, p)
            IF d >= 48 AND d <= 57 THEN
              IF part = 0 THEN
                RETURN FALSE
              END IF
              IF part < 0 THEN
                part = d - 48
              ELSE
                part = part * 10 + d - 48
              END IF
              IF part > 255 THEN
                RETURN FALSE
              END IF
              p = p + 1
            ELSE
              digits = FALSE
            END IF
          END WHILE
          IF part < 0 THEN
            RETURN FALSE
          END IF
          numbers = numbers + 1
        END WHILE
        IF numbers <> 4 THEN
          RETURN FALSE
        END IF
        pieces = pieces + 2
      ELSE
        IF after = 58 THEN
          p = p + 1
          IF p >= n THEN
            RETURN FALSE
          END IF
        ELSEIF p < n THEN
          RETURN FALSE
        END IF
        pieces = pieces + 1
      END IF
    END IF
  END WHILE
  RETURN compressed OR pieces = 8
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("net_isIpv6", BODY));
}
