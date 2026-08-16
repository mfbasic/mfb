//! `encoding::punycodeEncode` — descriptor entry, docs, and source body.
//!
//! Per-member file (mirrors collections/func_*.rs). The descriptor carries
//! an MFBASIC source body; its authored intro/description/examples migrated from
//! `src/docs/man/builtins/encoding/punycodeEncode.md`. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Encode a Unicode hostname to its ASCII Punycode form."#;
const DESC: &str = r#"`encoding::punycodeEncode` converts a Unicode hostname `domain` to the ASCII
representation used by internationalized domain names (IDNA), applying the
Punycode Bootstring algorithm of RFC 3492. The hostname is split on `.` into
labels, and each label is processed independently; the results are rejoined with
`.` so the dot structure of the input is preserved.

Each label is examined for non-ASCII code points. A label whose code points are
all below `128` is emitted verbatim, unchanged. A label containing any code
point at or above `128` is Punycode-encoded and prefixed with the ACE marker
`xn--`, producing the standard `xn--<encoding>` form.

Within an encoded label, the basic (ASCII) code points are copied out first,
followed by a `-` delimiter when any basic code points are present, and then the
generalized variable-length integers that describe the non-ASCII code points in
ascending order. The algorithm uses the RFC 3492 parameters (initial `n` = 128,
initial bias 72, base 36) and the standard bias-adaptation function. The input
`String` is decoded to Unicode scalar values through the package's UTF-8
decoder before encoding.

The function is **total**: every `String`, including the empty string and
all-ASCII hostnames, encodes successfully, and it never raises a runtime error.
The inverse operation is `encoding::punycodeDecode`, which converts an ASCII
Punycode hostname back to its Unicode form."#;
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __encoding_punycodeEncode(domain AS String) AS String
  LET labels AS List OF String = strings::split(domain, ".")
  MUT out AS String = ""
  MUT first AS Boolean = TRUE
  FOR EACH label IN labels
    IF first THEN
      first = FALSE
    ELSE
      out = out & "."
    END IF
    LET points AS List OF Integer = __encoding_codepoints(label)
    IF __encoding_labelHasNonAscii(points) THEN
      out = out & "xn--" & __encoding_punyEncodeLabel(points)
    ELSE
      out = out & label
    END IF
  NEXT
  RETURN out
END FUNC"#;
const EX: &str = r#"Encode a Unicode hostname to Punycode:

```
IMPORT encoding
IMPORT io

SUB main()
  io::print(encoding::punycodeEncode("bücher.example"))
END SUB
```

Round-trip through `punycodeDecode`:

```
IMPORT encoding
IMPORT io

SUB main()
  LET ace AS String = encoding::punycodeEncode("münchen.de")
  io::print(ace)
  io::print(encoding::punycodeDecode(ace))
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "punycodeEncode",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "domain",
                desc: "The Unicode domain name to encode.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::mfb(BODY, "__encoding_punycodeEncode"),
        }],
    });
}
