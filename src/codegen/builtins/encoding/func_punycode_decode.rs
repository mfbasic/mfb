//! `encoding::punycodeDecode` — descriptor entry, docs, and source body.
//!
//! Per-member file (mirrors collections/func_*.rs). The descriptor carries
//! an MFBASIC source body. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Decode an ASCII Punycode hostname back to its Unicode form."#;
const DESC: &str = r#"`encoding::punycodeDecode` converts an ASCII hostname in the internationalized
domain name (IDNA) representation back to Unicode, reversing the Punycode
Bootstring algorithm of RFC 3492. It is the inverse of
`encoding::punycodeEncode`.

The hostname is split on `.` into labels, and each label is processed
independently; the results are rejoined with `.` so the dot structure of the
input is preserved. A label that begins with the ACE marker `xn--` is decoded:
the `xn--` prefix is stripped and the remainder is run through the Punycode
label decoder. A label without the `xn--` prefix is emitted verbatim, unchanged.


Within an encoded label, the basic (ASCII) code points up to and including the
last `-` delimiter are copied out first, and the trailing generalized
variable-length integers are decoded to reconstruct the non-ASCII code points and
their insertion positions. The decoder uses the RFC 3492 parameters (initial
`n` = 128, initial bias 72, base 36) and the standard bias-adaptation function.
The reconstructed code points are re-encoded to a UTF-8 `String` on return.


The input is expected to be well-formed Punycode. Malformed input — a basic
(pre-delimiter) byte at or above `128`, a variable-length integer that is
truncated before it terminates or that would overflow, a byte that is not a
valid base-36 digit, a decoded scalar value outside the Unicode range, or an
encoded label longer than 1024 octets — raises `ErrInvalidFormat` rather than
producing a partial result. The length bound exists because RFC 3492's insertion
is quadratic in the label's length; 1024 octets is sixteen times the 63-octet DNS
label limit (RFC 1034 §3.1, RFC 5890 §2.3.1) and well past the RFC's own sample
strings, so no host label or round trip through `punycodeEncode` of ordinary
text can reach it."#;
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __encoding_punycodeDecode(asciiDomain AS String) AS String
  LET labels AS List OF String = strings::split(asciiDomain, ".")
  MUT out AS String = ""
  MUT first AS Boolean = TRUE
  FOR EACH label IN labels
    IF first THEN
      first = FALSE
    ELSE
      out = out & "."
    END IF
    IF strings::startsWith(label, "xn--") THEN
      out = out & __encoding_punyDecodeLabel(strings::mid(label, 4, len(label) - 4))
    ELSE
      out = out & label
    END IF
  NEXT
  RETURN out
END FUNC"#;
const EX: &str = r#"Decode a Punycode label to Unicode:

```
IMPORT encoding
IMPORT io

SUB main()
  io::print(encoding::punycodeDecode("xn--mnchen-3ya.de"))
END SUB
```

Round-trip through `punycodeEncode`:

```
IMPORT encoding
IMPORT io

SUB main()
  LET ace AS String = encoding::punycodeEncode("bücher.example")
  io::print(ace)
  io::print(encoding::punycodeDecode(ace))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "punycodeDecode",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "asciiDomain",
                desc: "The ASCII (Punycode) domain name to decode.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::String,
            errors: vec!["ErrInvalidFormat"],
            body: Body::mfb(BODY, "__encoding_punycodeDecode"),
        }],
    });
}
