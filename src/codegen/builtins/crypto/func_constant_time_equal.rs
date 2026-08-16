//! `crypto::constantTimeEqual` — descriptor entry + authored docs.
//!
//! A source member: a content-independent-time byte-list equality check for comparing
//! secrets (MACs, password hashes, tokens). Total — never raises. Its
//! `Body::Rewrite("__crypto_constantTimeEqual")` repoints the citation at the
//! `package.mfb` helper. Docs migrated from
//! `src/docs/man/builtins/crypto/constantTimeEqual.md`.

use super::{bytes, Body, DefaultValue, Implementation, Parameter, ParameterType, RegistryFunction};

const INTRO: &str =
    r#"Compare two byte lists for equality in time that does not depend on their contents."#;
const DESC: &str = r#"`crypto::constantTimeEqual` reports whether the byte lists `a` and `b` are
equal, taking time that is independent of their byte contents. Unlike an
ordinary comparison it does not return early at the first differing byte: it
accumulates the difference of every byte position and only then reports the
result, so an attacker cannot learn how many leading bytes matched by measuring
how long the comparison took. This is the correct way to compare secrets such
as message authentication codes, password hashes, and tokens.

The comparison works by folding `a[i] XOR b[i]` into a running OR accumulator
across all `i`; the result is `TRUE` exactly when that accumulator is zero,
meaning no byte differed. Every byte position is always examined.

**What is and is not secret.** Only the byte contents are protected. The lengths
of the inputs are not treated as secret. A length difference is folded into the
accumulated difference rather than taken as an early-return branch, so the
comparison does not branch on length (in)equality; the per-byte loop still runs
over the shared prefix, so the running time may reveal the (min) length. When
comparing values that should be a fixed size (for example a 32-byte HMAC tag), the
byte contents of same-length inputs are what stays constant-time.

The function is **total** — every combination of inputs, including two empty
lists (which compare equal), yields a Boolean and it never raises an error. Its
result is a portable software computation over the `bits` package, so it behaves
identically on every target (macOS/Linux, aarch64/x86-64) and uses no platform
crypto library."#;
const EX: &str = r#"Verify a received MAC without leaking timing:

```
IMPORT crypto
IMPORT strings
IMPORT io

SUB main()
  LET key AS List OF Byte = crypto::randomBytes(32)
  LET message AS List OF Byte = strings::toBytes("payload")
  LET received AS List OF Byte = crypto::hmacSha256(key, message)
  LET expected AS List OF Byte = crypto::hmacSha256(key, message)
  IF crypto::constantTimeEqual(expected, received) THEN
    io::print("authentic")
  ELSE
    io::print("tampered")
  END IF
END SUB
```"#;

pub(super) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "constantTimeEqual",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "a",
                    desc: "First byte list.",
                    aliases: &[],
                    ty: bytes(),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "b",
                    desc: "Second byte list.",
                    aliases: &[],
                    ty: bytes(),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::Rewrite("__crypto_constantTimeEqual"),
        }],
    });
}
