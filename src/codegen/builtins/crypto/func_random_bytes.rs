//! `crypto::randomBytes` — descriptor entry + authored docs.
//!
//! A NATIVE member: the OS-entropy CSPRNG. Its `Body::native` OS-seam slots point at
//! [`super::native::lower_crypto_random_bytes`] (emission relocated from the former
//! `src/codegen/builtins/crypto/native`), dispatched generically through
//! `registry::os_helper` and its runtime spec DERIVED by `registry::runtime_specs`.

use super::{
    bytes, Body, DefaultValue, Implementation, Parameter, ParameterType, RegistryFunction,
};

const INTRO: &str =
    r#"Return `count` cryptographically secure random bytes drawn from the OS CSPRNG."#;
const DESC: &str = r#"`crypto::randomBytes` returns a fresh `List OF Byte` of length `count`, filled
from the operating system's cryptographically secure pseudo-random number
generator (CSPRNG). The output is unpredictable to an adversary and is the
correct source for keys, nonces, initialization vectors, salts, tokens, and any
other value whose secrecy or unguessability is a security requirement.

**Range and boundaries.** `count` is validated **before** any allocation and
must satisfy `0 <= count <= 16777216` (16 MiB, the `RANDOM_BYTES_MAX_COUNT`
cap). A `count` of `0` returns an empty list; a negative `count` or one above
the 16 MiB cap raises `ErrInvalidArgument` and allocates nothing. The cap also
keeps the internal collection-size arithmetic well below integer overflow.

**Security caveats.** This generator is cryptographically secure and, by design,
**not** seedable — there is no way to fix, seed, or replay its output, and each
call draws fresh entropy, so results are never reproducible across runs. That is
the deliberate contrast with `math::rand`, a fast, seedable PCG64 generator that
is **not** cryptographically secure and must never be used for keys, tokens, or
nonces. After the returned list is built, the internal entropy scratch buffer is
zeroed, so no later allocation in the same program can observe the generated
bytes. When you later compare secret material derived from these bytes (a MAC, a
token, an API key), never use the ordinary `=` operator — it short-circuits and
leaks timing; use `crypto::constantTimeEqual`.

**Implementation.** Unlike the portable software cores in this package (the
hashes, HMAC, HKDF, PBKDF2, and the AEADs), `randomBytes` is the one member here
that is a **native runtime helper reading OS entropy directly**, not MFBASIC
source. On **macOS and Linux** (glibc and musl) it uses `getentropy(2)`, filling
the buffer in chunks of at most 256 bytes (the per-call `getentropy` limit),
transparent to the caller. On **Windows** it uses `BCryptGenRandom` with the
`BCRYPT_USE_SYSTEM_PREFERRED_RNG` flag. Because the bytes come from OS entropy,
the output is inherently non-reproducible and platform-provided rather than
byte-identical across targets."#;
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

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "randomBytes",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
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
