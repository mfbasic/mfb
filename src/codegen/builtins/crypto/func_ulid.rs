//! `crypto::ulid` — descriptor entry and authored docs.

use super::{Body, Implementation, ParameterType, RegistryFunction};

#[rustfmt::skip]
const BODY: &str =
r#"' A canonical 26-character ULID with a 48-bit millisecond timestamp.
FUNC __crypto_ulid() AS String
  LET alphabet AS String = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"
  MUT ms AS Integer = datetime::nowNanos() / 1000000
  MUT out AS String = ""
  MUT i AS Integer = 0
  WHILE i < 10
    LET digit AS Integer = ms MOD 32
    out = strings::mid(alphabet, digit, 1) & out
    ms = ms / 32
    i = i + 1
  END WHILE
  LET rb AS List OF Byte = crypto::randomBytes(10)
  MUT buffer AS Integer = 0
  MUT bitCount AS Integer = 0
  i = 0
  WHILE i < 10
    buffer = buffer * 256 + toInt(collections::get(rb, i))
    bitCount = bitCount + 8
    WHILE bitCount >= 5
      bitCount = bitCount - 5
      LET power AS Integer = bits::sl(1, bitCount)
      LET digit AS Integer = (buffer / power) MOD 32
      out = out & strings::mid(alphabet, digit, 1)
      buffer = buffer MOD power
    END WHILE
    i = i + 1
  END WHILE
  RETURN out
END FUNC"#;

const INTRO: &str = r#"Return a canonical time-sortable ULID string."#;
const DESC: &str = r#"`crypto::ulid` returns a canonical 26-character ULID. The first
10 Crockford Base32 characters encode the current 48-bit Unix timestamp in
milliseconds; the remaining 16 encode 80 bits from the OS CSPRNG. The uppercase
alphabet is `0123456789ABCDEFGHJKMNPQRSTVWXYZ`, excluding ambiguous letters.

The function takes no arguments. Values sort by generation millisecond under
ordinary ASCII lexical ordering; calls within one millisecond use independent
randomness and are not guaranteed to sort in call order. Clock, entropy, or
allocation failures propagate from the underlying built-ins."#;
const EX: &str = r#"Generate a compact, time-sortable identifier:

```
IMPORT crypto
IMPORT io

SUB main()
  io::print(crypto::ulid())
END SUB
```"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "ulid",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("()"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::String,
            errors: vec!["ErrUnknown", "ErrOutOfMemory"],
            body: Body::mfb(BODY, "__crypto_ulid"),
        }],
    });
}
