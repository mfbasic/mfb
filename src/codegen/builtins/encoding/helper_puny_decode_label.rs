//! `__encoding_punyDecodeLabel` — shared private helper for the `encoding` package.
//!
//! bug-510 (audit-3 DEC-05/06). The decoder used to insert each code point by
//! rebuilding its whole output list with `append`, so a label of `n` code points
//! cost `n^2` appends and left every intermediate list in the arena: a 32 KB label
//! took 47 s and 4.3 GB. Two changes bound it. The label is refused past 1024
//! octets — because the RFC 3492 insertion is inherently quadratic in the label's
//! length, and 1024 makes that at most ~8 MB of in-place shifting while sitting far
//! past anything real: a DNS A-label is at most 63 octets including its `xn--`
//! (RFC 1034 §3.1, RFC 5890 §2.3.1) and the RFC's own longest sample string is 74.
//! (A 63-octet cap was tried first and refused that sample; the encoder also emits
//! longer labels, and `decode(encode(x))` must keep round-tripping them.) Within
//! the limit the insertion is RFC 3492's own shift, `collections::insert` on the
//! `MUT` output, which is in place. And the RFC's §6.4 overflow checks are applied,
//! so a label whose variable-length integer would overflow is malformed Punycode
//! (`ErrInvalidFormat`), not an arithmetic error surfacing from the multiply.
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
  ' The insertion below is quadratic in the label's length, so the length is
  ' bounded before anything is decoded (bug-510, DEC-05). 1024 octets of payload
  ' is sixteen times the longest label DNS can carry and fourteen times RFC 3492's
  ' longest sample, and costs at most a few megabytes of in-place shifting.
  IF total > 1024 THEN
    FAIL error(77050003, "invalid punycode: label longer than 1024 octets")
  END IF
  LET maxint AS Integer = 9223372036854775807
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
      ' RFC 3492 section 6.4: `i + digit * w` must not overflow.
      IF digit > (maxint - pos) / w THEN
        FAIL error(77050003, "invalid punycode")
      END IF
      pos = pos + digit * w
      threshold = __encoding_punyThreshold(k, bias)
      IF digit < threshold THEN
        reading = FALSE
      ELSE
        ' RFC 3492 section 6.4: `w * (base - t)` must not overflow either.
        IF w > maxint / (36 - threshold) THEN
          FAIL error(77050003, "invalid punycode")
        END IF
        w = w * (36 - threshold)
        k = k + 36
      END IF
    END WHILE
    outLen = len(output) + 1
    bias = __encoding_punyAdapt(pos - oldPos, outLen, oldPos = 0)
    ' RFC 3492 section 6.4: `n + i / (out + 1)` must not overflow.
    IF pos / outLen > maxint - n THEN
      FAIL error(77050003, "invalid punycode")
    END IF
    n = n + pos / outLen
    pos = pos - (pos / outLen) * outLen
    ' Insert code point n at index pos: the RFC's shift, in place.
    output = collections::insert(output, pos, n)
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
