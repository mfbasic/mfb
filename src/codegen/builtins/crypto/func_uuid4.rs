//! `crypto::uuid4` — descriptor entry + authored docs.
//!
//! A source-glue member: a random RFC 4122 version-4 UUID as a canonical lowercase
//! string, drawing 16 CSPRNG bytes through `crypto::randomBytes`. Takes no arguments.
//! Its `Body::Rewrite("__crypto_uuid4")` repoints the citation at the `package.mfb`
//! helper.

use super::{Body, Implementation, ParameterType, RegistryFunction};

const INTRO: &str = r#"Return a random RFC 4122 version-4 UUID as a canonical lowercase string."#;
const DESC: &str = r#"`crypto::uuid4` returns a random version-4 UUID as a canonical lowercase
`String` in the 8-4-4-4-12 hyphenated form — for example
`"f47ac10b-58cc-4372-a567-0e02b2c3d479"`. The result is always exactly 36
characters: 32 lowercase hexadecimal digits plus the four hyphens.

**Standard.** A version-4 UUID (RFC 4122) is 122 bits of randomness with a 4-bit
version field fixed to `4` and a 2-bit variant field fixed to the RFC 4122
variant (`10` in binary), exactly as the standard prescribes. Every other bit is random, so a `uuid4` carries 122 bits of
randomness.

**Security caveats.** The random bytes come from `crypto::randomBytes` (the OS
CSPRNG), so the identifiers are cryptographically strong and effectively
collision-free in practice. As with every `crypto` random helper the generator is
**not** seedable, so each call produces a fresh, non-reproducible value. Use
`uuid4` whenever a random, unguessable identifier is needed; for its fast,
seedable, non-cryptographic counterpart see `math::rand`, which must never be
used for security-sensitive identifiers.

`uuid4` takes no arguments. It is total in normal operation, but because it draws
entropy through `crypto::randomBytes`, an entropy or out-of-memory
failure there propagates out as `ErrUnknown` or `ErrOutOfMemory`.

**Implementation.** `uuid4` is portable MFBASIC software layered over
`crypto::randomBytes`. Its logic is byte-identical on every target (macOS/Linux,
aarch64/x86-64); only the underlying entropy, and therefore each generated value,
differs."#;
const EX: &str = r#"Generate a unique identifier:

```
IMPORT crypto
IMPORT io

SUB main()
  LET id AS String = crypto::uuid4()
  io::print(id)
END SUB
```

Each call yields a distinct value:

```
IMPORT crypto
IMPORT io

SUB main()
  LET a AS String = crypto::uuid4()
  LET b AS String = crypto::uuid4()
  io::print(toString(a <> b))
END SUB
```"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "uuid4",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("()"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::String,
            errors: vec!["ErrUnknown", "ErrOutOfMemory"],
            body: Body::Rewrite("__crypto_uuid4"),
        }],
    });
}
