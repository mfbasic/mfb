//! `crypto::shake256(data, length)` — the SHAKE256 extendable-output function.
//!
//! SHAKE256 (FIPS 202 §6.2) is an XOF: the same Keccak-f[1600] sponge behind the
//! SHA-3 `Hash` selectors, at rate 1088 bits with the `0x1f` domain suffix, squeezed
//! to any caller-chosen `length`. A variable-length output does not fit the
//! fixed-digest `Hash` selector, so it is its own member. Pure-MFB rewrite onto
//! `__crypto_shake256` (registered by [`super::helper_shake256`]); the `String`
//! overload rewrites to the `__crypto_shake256Text` UTF-8 shim
//! ([`super::helper_shake256_text`]) — a `String` and a `List OF Byte` are not
//! ABI-interchangeable, so the two overloads rewrite to distinct MFB bodies.
//! The Ed448 signature scheme (RFC 8032 §5.2) hashes with this same core.

use super::{
    bytes, Body, DefaultValue, Implementation, Parameter, ParameterType, RegistryFunction,
};

const INTRO: &str = r#"Squeeze any number of output bytes from a message with the SHAKE256 extendable-output function (FIPS 202)."#;
const DESC: &str = r#"`crypto::shake256(data, length)` absorbs `data` into the SHAKE256 sponge and
returns the first `length` bytes of its output as a raw `List OF Byte`. SHAKE256 is
the FIPS 202 *extendable-output function* (XOF) of the SHA-3 family: unlike a
fixed-digest hash it produces an output stream of any length, and a shorter
request is always a prefix of a longer one (`shake256(m, 16)` equals the first 16
bytes of `shake256(m, 64)`). It offers 256-bit security against collisions and
preimages for outputs of at least 64 bytes; shorter outputs cap security at half
the output length in bits.

`length` must be at least 1; `0` or a negative value raises `ErrInvalidArgument`.
There is no upper bound beyond available memory — a large `length` simply squeezes
more blocks. Any input length is accepted, including the empty message.

Two overloads accept the message: the `List OF Byte` overload absorbs the raw bytes
as given; the `String` overload absorbs the string's UTF-8 encoding (equivalent to
`crypto::shake256(strings::toBytes(s), length)`). The output is raw binary, not
text — stringify it with `encoding::hexEncode` or `encoding::base64Encode`.

Use `shake256` where a protocol specifies it (Ed448 hashes with SHAKE256, and
KEM/KDF designs use it to derive arbitrary-length keys). For a fixed-size digest
prefer `crypto::hash` with a `Hash.SHA3_*` or `Hash.SHA2_*` selector, and for a
keyed MAC or password stretching use `crypto::hmac`/`crypto::pbkdf2`.

**Implementation.** SHAKE256 is the Keccak sponge (FIPS 202 §4–§6) at rate 1088
bits / capacity 512 with domain suffix `0x1f` and `pad10*1`, over a portable
MFBASIC Keccak-f[1600] core (24 rounds; each 64-bit lane is one `Integer` bit
pattern manipulated only through the `bits` package). No platform cryptographic
library is called, so the output is **byte-identical on macOS, Linux, and
Windows**. No loop bound, branch, or index depends on the message or state
contents; only the public message and output lengths do."#;
const EX: &str = r#"Derive 64 bytes from a message and print them as hex:

```
IMPORT crypto
IMPORT encoding
IMPORT io

SUB main()
  LET out AS List OF Byte = crypto::shake256("abc", 64)
  io::print(encoding::hexEncode(out))
END SUB
```

A shorter request is a prefix of a longer one:

```
IMPORT crypto
IMPORT encoding
IMPORT collections
IMPORT io

SUB main()
  LET short AS List OF Byte = crypto::shake256("abc", 16)
  LET long AS List OF Byte = crypto::shake256("abc", 64)
  io::print(toString(encoding::hexEncode(short) = encoding::hexEncode(collections::mid(long, 0, 16))))
END SUB
```"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "shake256",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("List OF Byte or String, Integer"),
        internal_only: false,
        implementations: vec![
            Implementation {
                params: vec![
                    Parameter {
                        name: "data",
                        desc: "The bytes to absorb. Any length is accepted, including the empty list.",
                        aliases: &[],
                        ty: bytes(),
                        default: DefaultValue::None,
                    },
                    Parameter {
                        name: "length",
                        desc: "Number of output bytes to squeeze; must be at least 1.",
                        aliases: &[],
                        ty: ParameterType::Integer,
                        default: DefaultValue::None,
                    },
                ],
                return_type: bytes(),
                errors: vec!["ErrInvalidArgument"],
                body: Body::Rewrite("__crypto_shake256"),
            },
            Implementation {
                params: vec![
                    Parameter {
                        name: "data",
                        desc: "A string whose UTF-8 bytes are absorbed.",
                        aliases: &[],
                        ty: ParameterType::String,
                        default: DefaultValue::None,
                    },
                    Parameter {
                        name: "length",
                        desc: "Number of output bytes to squeeze; must be at least 1.",
                        aliases: &[],
                        ty: ParameterType::Integer,
                        default: DefaultValue::None,
                    },
                ],
                return_type: bytes(),
                errors: vec!["ErrInvalidArgument"],
                body: Body::Rewrite("__crypto_shake256Text"),
            },
        ],
    });
}
