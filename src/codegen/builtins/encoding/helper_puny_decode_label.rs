//! `__encoding_punyDecodeLabel` — shared private helper for the `encoding` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Decode one Punycode label (without the "xn--" prefix) to its String form.
FUNC __encoding_punyDecodeLabel(label AS String) AS String
  LET data AS List OF Byte = strings::toBytes(label)
  LET total AS Integer = len(data)
  MUT output AS List OF Integer = []
  MUT basicEnd AS Integer = 0
  MUT i AS Integer = 0
  ' Find the last delimiter: everything before it is literal ASCII.
  MUT lastDelim AS Integer = -1
  i = 0
  WHILE i < total
    IF toInt(collections::get(data, i)) = 45 THEN
      lastDelim = i
    END IF
    i = i + 1
  END WHILE
  IF lastDelim >= 0 THEN
    i = 0
    WHILE i < lastDelim
      LET bc AS Integer = toInt(collections::get(data, i))
      IF bc >= 128 THEN
        FAIL error(77050003, "invalid punycode")
      END IF
      output = collections::append(output, bc)
      i = i + 1
    END WHILE
    basicEnd = lastDelim + 1
  ELSE
    basicEnd = 0
  END IF
  MUT n AS Integer = 128
  MUT bias AS Integer = 72
  MUT pos AS Integer = 0
  MUT idx AS Integer = basicEnd
  MUT k AS Integer = 0
  MUT w AS Integer = 0
  MUT threshold AS Integer = 0
  MUT digit AS Integer = 0
  MUT oldPos AS Integer = 0
  MUT outLen AS Integer = 0
  MUT insertAt AS Integer = 0
  WHILE idx < total
    oldPos = pos
    w = 1
    k = 36
    MUT reading AS Boolean = TRUE
    WHILE reading
      IF idx >= total THEN
        FAIL error(77050003, "invalid punycode")
      END IF
      digit = __encoding_punyValue(toInt(collections::get(data, idx)))
      idx = idx + 1
      IF digit < 0 THEN
        FAIL error(77050003, "invalid punycode")
      END IF
      pos = pos + digit * w
      threshold = __encoding_punyThreshold(k, bias)
      IF digit < threshold THEN
        reading = FALSE
      ELSE
        w = w * (36 - threshold)
        k = k + 36
      END IF
    END WHILE
    outLen = len(output) + 1
    bias = __encoding_punyAdapt(pos - oldPos, outLen, oldPos = 0)
    n = n + pos / outLen
    pos = pos - (pos / outLen) * outLen
    ' Insert code point n at index pos.
    MUT rebuilt AS List OF Integer = []
    MUT t AS Integer = 0
    WHILE t < len(output)
      IF t = pos THEN
        rebuilt = collections::append(rebuilt, n)
      END IF
      rebuilt = collections::append(rebuilt, collections::get(output, t))
      t = t + 1
    END WHILE
    IF pos >= len(output) THEN
      rebuilt = collections::append(rebuilt, n)
    END IF
    output = rebuilt
    pos = pos + 1
  END WHILE
  MUT result AS String = ""
  FOR EACH cp IN output
    result = result & __encoding_fromCodepoint(cp)
  NEXT
  RETURN result
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("encoding_punyDecodeLabel", BODY));
}
