//! `crypto::randomBytes` — descriptor entry + authored docs.
//!
//! A NATIVE member: the OS-entropy CSPRNG. Its `Body::native` OS-seam slots point at
//! [`super::native::lower_crypto_random_bytes`] (emission relocated from the former
//! `src/target/shared/code/crypto.rs`), dispatched generically through
//! `registry::os_helper` and its runtime spec DERIVED by `registry::runtime_specs`.
//! Docs migrated from `src/docs/man/builtins/crypto/randomBytes.md`.

use super::{
    bytes, Body, DefaultValue, Implementation, Parameter, ParameterType, RegistryFunction,
};

const INTRO: &str = r#"Return cryptographically secure random bytes drawn from the OS CSPRNG."#;
const DESC: &str = r#"`crypto::randomBytes` returns `count` fresh bytes drawn from the operating
system's cryptographically secure pseudo-random number generator (CSPRNG). The
bytes are produced by `getentropy` (or `BCryptGenRandom` on Windows), a
non-deprecated OS entropy source, so the output is suitable for keys, nonces,
salts, tokens, and any other use where unpredictability is a security requirement.

Unlike the portable software cores in this package (the hashes, HMAC, HKDF,
PBKDF2, and the AEADs), `randomBytes` is a **native runtime helper** rather than
source: it reads OS entropy directly, so its output is inherently non-reproducible
and platform-provided rather than byte-identical across targets.

This generator is cryptographically secure and, by design, **not** seedable:
there is no way to fix or replay its output. That is the deliberate contrast with
`math::rand`, a fast, seedable PCG64 generator that is **not** cryptographically
secure and must never be used for keys, tokens, or nonces.

Each call draws fresh entropy, so results are not reproducible across runs.
`count` must be in the range `0` to `16777216` (16 MiB) inclusive: a `count` of 0
returns an empty list, while a negative `count` or one above the 16 MiB cap raises
`ErrInvalidArgument`. Internally the fill runs in chunks of at most 256 bytes (the
per-call `getentropy` limit), transparent to the caller."#;
const EX: &str = r#"Generate a 32-byte key and a 12-byte AEAD nonce:

```
IMPORT crypto

SUB main()
  LET key AS List OF Byte = crypto::randomBytes(32)
  LET nonce AS List OF Byte = crypto::randomBytes(12)
END SUB
```

A count of zero returns an empty list:

```
IMPORT crypto

SUB main()
  LET none AS List OF Byte = crypto::randomBytes(0)
END SUB
```"#;

pub(super) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "randomBytes",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "count",
                desc: "The number of random bytes to return. Must be in `0` to `16777216` \
                       (16 MiB) inclusive; `0` yields an empty list.",
                aliases: &[],
                ty: ParameterType::Integer,
                default: DefaultValue::None,
            }],
            return_type: bytes(),
            errors: vec!["ErrInvalidArgument", "ErrUnknown", "ErrOutOfMemory"],
            body: Body::native(
                Some(super::native::lower_crypto_random_bytes),
                Some(super::native::lower_crypto_random_bytes),
                None,
            ),
        }],
    });
}
