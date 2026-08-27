//! `crypto::uuid7` — descriptor entry and authored docs.

use super::{Body, Implementation, ParameterType, RegistryFunction};

#[rustfmt::skip]
const BODY: &str =
r#"' A time-ordered UUIDv7 in canonical lowercase form (RFC 9562).
FUNC __crypto_uuid7() AS String
  LET ms AS Integer = datetime::nowNanos() / 1000000
  LET rb AS List OF Byte = crypto::randomBytes(10)
  MUT bytes AS List OF Byte = []
  MUT divisor AS Integer = 1099511627776
  MUT i AS Integer = 0
  WHILE i < 6
    bytes = collections::append(bytes, toByte((ms / divisor) MOD 256))
    divisor = divisor / 256
    i = i + 1
  END WHILE
  bytes = collections::append(bytes, toByte(bits::bor(bits::band(toInt(collections::get(rb, 0)), 15), 112)))
  bytes = collections::append(bytes, collections::get(rb, 1))
  bytes = collections::append(bytes, toByte(bits::bor(bits::band(toInt(collections::get(rb, 2)), 63), 128)))
  i = 3
  WHILE i < 10
    bytes = collections::append(bytes, collections::get(rb, i))
    i = i + 1
  END WHILE
  LET hex AS String = encoding::hexEncode(bytes)
  RETURN strings::mid(hex, 0, 8) & "-" & strings::mid(hex, 8, 4) & "-" & strings::mid(hex, 12, 4) & "-" & strings::mid(hex, 16, 4) & "-" & strings::mid(hex, 20, 12)
END FUNC"#;

const INTRO: &str =
    r#"Return a time-ordered RFC 9562 version-7 UUID as a canonical lowercase string."#;
const DESC: &str = r#"`crypto::uuid7` returns a UUID version 7 as a canonical lowercase
36-character `String` in 8-4-4-4-12 form. Per RFC 9562, its first 48 bits are the
current Unix timestamp in milliseconds, its version nibble is `7`, its variant
bits are `10`, and its remaining 74 bits are drawn from the OS CSPRNG. This makes
values naturally time-ordered while retaining strong same-millisecond uniqueness.

The function takes no arguments. Clock, entropy, or allocation failures propagate
from `datetime::nowNanos`, `crypto::randomBytes`, and string construction."#;
const EX: &str = r#"Generate a time-ordered identifier:

```
IMPORT crypto
IMPORT io

SUB main()
  io::print(crypto::uuid7())
END SUB
```"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "uuid7",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("()"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::String,
            errors: vec!["ErrUnknown", "ErrOutOfMemory"],
            body: Body::mfb(BODY, "__crypto_uuid7"),
        }],
    });
}
