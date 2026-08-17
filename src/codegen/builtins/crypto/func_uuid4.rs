//! `crypto::uuid4` — descriptor entry + authored docs.
//!
//! A source-glue member: a random RFC 4122 version-4 UUID as a canonical lowercase
//! string, drawing 16 CSPRNG bytes through `crypto::randomBytes`. Takes no arguments.
//! Its `Body::Rewrite("__crypto_uuid4")` repoints the citation at the `package.mfb`
//! helper. Docs migrated from `src/docs/man/builtins/crypto/uuid4.md`.

use super::{Body, Implementation, ParameterType, RegistryFunction};

const INTRO: &str = r#"Return a random RFC 4122 version-4 UUID as a canonical lowercase string."#;
const DESC: &str = r#"`crypto::uuid4` returns a random version-4 UUID as a canonical lowercase
`String` in the 8-4-4-4-12 hyphenated form — for example
`"f47ac10b-58cc-4372-a567-0e02b2c3d479"`. The result is always 36 characters:
32 hexadecimal digits plus the four hyphens.

A version-4 UUID is 122 bits of randomness with the 4-bit version field fixed to
`4` and the 2-bit variant field fixed to the RFC 4122 variant, exactly as the
standard prescribes. Internally `uuid4` draws 16 random bytes, forces the version
nibble of byte 6 and the variant bits of byte 8, hex-encodes the 16 bytes, and
splits the digits into the five hyphenated groups.

The random bytes come from the same OS CSPRNG as `crypto::randomBytes`
(`getentropy` on both macOS and Linux), so the identifiers are cryptographically
strong and effectively collision-free in practice. As with all `crypto` random
helpers the generator is **not** seedable, so each call produces a fresh,
non-reproducible value. Use `uuid4` whenever a random, unguessable identifier is
needed; for its fast, seedable, non-cryptographic counterpart see `math::rand`,
which must never be used for security-sensitive identifiers.

This function takes no arguments and, barring a platform entropy failure or an
allocation failure, always succeeds."#;
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

pub(super) fn register(pkg: &mut super::RegistryPackage) {
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
            errors: vec![],
            body: Body::Rewrite("__crypto_uuid4"),
        }],
    });
}
