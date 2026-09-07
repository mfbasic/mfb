//! `crypto::argon2id(password, salt, memoryKiB, iterations, parallelism, length)` and
//! `crypto::argon2id(password, salt, profile, length)` — the package's memory-hard
//! password KDF (bug-515).
//!
//! Two overloads over ONE body. The explicit-cost form rewrites straight onto the
//! `__crypto_argon2id` MFB core ([`super::helper_argon2id`]); the profile form rewrites
//! onto [`super::helper_argon2id_profile`], which resolves its `crypto::Argon2Profile`
//! to concrete `memoryKiB`/`iterations`/`parallelism` and calls that same core. So the
//! two spellings share one validation site and one fill site, and the profile constants
//! are the only part that is retunable later.
//!
//! The explicit form exists because a caller VERIFYING a hash produced elsewhere has to
//! be able to name the exact published parameter set it was produced with; the profile
//! form is what the page recommends for producing one.

use super::{
    bytes, Body, DefaultValue, Implementation, Parameter, ParameterType, RegistryFunction,
};

const INTRO: &str = r#"Derive a key from a password with Argon2id, the memory-hard password hash."#;
const DESC: &str = r#"`crypto::argon2id` derives `length` bytes of key material from a `password` and a
`salt` using Argon2id version 19 (RFC 9106), the winner of the 2015 Password Hashing
Competition. The result is a raw `List OF Byte` of exactly `length` bytes, and it is
deterministic in every one of its inputs. **This is the function to reach for when you
are storing passwords**; `crypto::pbkdf2` is for RFC 8018 / WPA2 compatibility and for
stretching a passphrase into a key.

Argon2id is *memory-hard*: the derivation fills `memoryKiB` kibibytes of working values
and reads back through them in a password-dependent order, so an attacker cannot make a
guess cheaper by throwing more parallel silicon at it — each guess needs the memory too.
That is the property `crypto::pbkdf2` does not have.

There are two spellings, and they compute exactly the same thing:

- `crypto::argon2id(password, salt, memoryKiB, iterations, parallelism, length)` names the three cost parameters directly. Use it to **verify** a hash produced somewhere else, where the parameters were fixed by whoever produced it, or when you have tuned your own.
- `crypto::argon2id(password, salt, profile, length)` takes the costs from a `crypto::Argon2Profile`. Use it to **produce** a new hash. It calls the explicit form with the constants below, so the two can never drift apart.

| `crypto::Argon2Profile` | `memoryKiB` | `iterations` | `parallelism` | Source |
|---|---|---|---|---|
| `Minimum` | 19456 (19 MiB) | 2 | 1 | the OWASP Password Storage Cheat Sheet's minimum configuration |
| `Recommended` | 65536 (64 MiB) | 3 | 4 | RFC 9106 §4's SECOND RECOMMENDED option |

Store the `salt`, the three cost parameters and the derived bytes together — you need
all of them to check a password later — and compare a stored derivation against a
recomputed one with `crypto::constantTimeEqual`, never with `=`. Draw the `salt` from
`crypto::randomBytes`, 16 bytes, fresh for every password. Derived key material is raw
binary, not text: stringify it with `encoding::hexEncode` or `encoding::base64Encode`.

**`parallelism` is a cost parameter, not a thread count.** It changes the digest, and
this member computes every lane on the calling thread — raising it does not make the
call faster, it makes it different.

`memoryKiB` is rounded down to a whole number of blocks, four per lane, so the memory
actually worked through can be a little under what you asked for — Argon2 defines that
rounding, so this matches every other implementation. The value you passed is bound
into the derivation as well, so two `memoryKiB` values that round to the same block
count still derive **different** keys: record the exact number you used.

Ranges, each raising `ErrInvalidArgument` when missed: `parallelism` is 1 through
16777215; `iterations` is at least 1; `length` is at least 4; `salt` is at least 8
bytes (16 is the recommended length); and `memoryKiB` runs from eight times
`parallelism` through 2097152 (2 GiB), which is the largest memory RFC 9106
recommends. A
`memoryKiB` above that ceiling is refused outright — the call raises before it reserves
anything, so an absurd request is an error and not an exhausted machine. `password` may
be any length, including empty.

**Cost.** This is a portable software core, not a native one, so it is far slower than
a C Argon2 — it works through roughly 22 MiB of Argon2 memory per second on a 2026
laptop core. `Minimum` takes about 1.4 seconds and `Recommended` about 7. Its working
set while it runs grows with the work it does, not just with `memoryKiB`: reckon on
roughly three kibibytes for every kibibyte-pass, so a large `memoryKiB` or a large
`iterations` costs more memory than the parameter alone suggests. Pick the profile your login latency budget can pay for, and measure it
on your own hardware.

**Implementation.** Argon2id is specified by RFC 9106; this is a clean-room MFBASIC
software core over the `bits` package, including the BLAKE2b-512 (RFC 7693) it is built
on. No platform cryptographic library is called, so the output is **byte-identical on
macOS, Linux, and Windows** and across aarch64/x86-64. It is pinned against RFC 9106
§5.3's test vector and RFC 7693 Appendix A's BLAKE2b vector.

To derive keys from already-high-entropy input use `crypto::hkdf`; to authenticate a
message use `crypto::hmac`."#;
const EX: &str = r#"Hash a password for storage, then check a guess against it:

```
IMPORT crypto
IMPORT encoding
IMPORT io
IMPORT strings

SUB main()
  LET salt AS List OF Byte = crypto::randomBytes(16)
  ' A real password store uses crypto::Argon2Profile.Recommended. These explicit
  ' costs keep the example instant and are far too cheap to store a password with.
  LET stored AS List OF Byte = crypto::argon2id(strings::toBytes("correct horse"), salt, 32, 1, 1, 32)
  LET guess AS List OF Byte = crypto::argon2id(strings::toBytes("correct horse"), salt, 32, 1, 1, 32)
  io::print("bytes=" & toString(len(stored)))
  io::print("match=" & toString(crypto::constantTimeEqual(stored, guess)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    let password_param = || Parameter {
        name: "password",
        desc: "The password bytes; any length, including empty.",
        aliases: &[],
        ty: bytes(),
        default: DefaultValue::None,
    };
    let salt_param = || Parameter {
        name: "salt",
        desc: "The salt bytes; at least 8, and 16 random bytes unique to this password is best.",
        aliases: &[],
        ty: bytes(),
        default: DefaultValue::None,
    };
    let length_param = || Parameter {
        name: "length",
        desc: "Number of output bytes to derive; must be at least 4.",
        aliases: &[],
        ty: ParameterType::Integer,
        default: DefaultValue::None,
    };
    pkg.add_function(RegistryFunction {
        name: "argon2id",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some(
            "List OF Byte, List OF Byte, Integer, Integer, Integer, Integer or List OF Byte, List OF Byte, crypto::Argon2Profile, Integer",
        ),
        internal_only: false,
        implementations: vec![
            Implementation {
                params: vec![
                    password_param(),
                    salt_param(),
                    Parameter {
                        name: "memoryKiB",
                        desc: "Memory cost in kibibytes: at least eight times `parallelism`, at most 2097152, and rounded down to a whole number of blocks (four per lane).",
                        aliases: &[],
                        ty: ParameterType::Integer,
                        default: DefaultValue::None,
                    },
                    Parameter {
                        name: "iterations",
                        desc: "Number of passes over the memory (Argon2's time cost); must be at least 1.",
                        aliases: &[],
                        ty: ParameterType::Integer,
                        default: DefaultValue::None,
                    },
                    Parameter {
                        name: "parallelism",
                        desc: "Number of Argon2 lanes, 1 through 16777215. A cost parameter that changes the result, not a thread count.",
                        aliases: &[],
                        ty: ParameterType::Integer,
                        default: DefaultValue::None,
                    },
                    length_param(),
                ],
                return_type: bytes(),
                errors: vec!["ErrInvalidArgument"],
                body: Body::Rewrite("__crypto_argon2id"),
            },
            Implementation {
                params: vec![
                    password_param(),
                    salt_param(),
                    Parameter {
                        name: "profile",
                        desc: "The vetted cost setting to derive with; see `mfb man crypto types`.",
                        aliases: &[],
                        ty: ParameterType::named("Argon2Profile"),
                        default: DefaultValue::None,
                    },
                    length_param(),
                ],
                return_type: bytes(),
                errors: vec!["ErrInvalidArgument"],
                body: Body::Rewrite("__crypto_argon2idProfile"),
            },
        ],
    });
}
