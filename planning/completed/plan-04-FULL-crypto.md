# plan-04 — Built-in `crypto` Package (library-backed) — overview

Last updated: 2026-06-28
Overall Effort: huge

**Split plan** (by effort into four small/medium sub-plans; see Part C). This document holds the
normative surface and shared design; the implementation phases live in the lettered sub-plans.

This document is the **normative definition and implementation plan** for a new
built-in `crypto` package: cryptographic hashes, HMAC, key-derivation functions,
authenticated encryption (AEAD), public-key key generation and signatures, a
cryptographically-secure RNG, and constant-time comparison.

**`crypto` is backed by the host's standard cryptographic libraries** rather than
a self-written implementation: **OpenSSL libcrypto** on Linux, and
**CommonCrypto + Security.framework** (already linked via libSystem) on macOS.
Those libraries internally dispatch to the CPU's crypto instructions (ARMv8 Crypto
Extensions on AArch64; SHA-NI/AES-NI on x86_64), so **hardware acceleration is
inherited automatically** — this plan does not emit SIMD crypto instructions or
implement CPU feature detection. A small **portable software core** (built on the
`bits` package from plan-02) is used only where a library has no usable C ABI on macOS —
**Ed25519** and **ChaCha20-Poly1305** — and as the implementation for primitives
that are pure glue (HKDF, `randomInt`, `uuid4`).

Outputs are standardized, so the **native and Binary Representation (BR) execution
paths produce identical results** by virtue of computing the same standard
algorithm — a SHA-256 digest is a SHA-256 digest regardless of which library or
core produced it.

This plan delivers the **`crypto`** package (§A) — the cryptographic surface,
backed by the system libraries with the narrow software exceptions noted above. It
builds on two foundational packages that **plan-02 has since landed** and which are
now in place (documented at `./mfb spec stdlib encoding`, `./mfb man bits`, and
`./mfb man encoding`):

- **`bits`** — integer bitwise/shift/rotate operations, now native
  single-instruction ops (`./mfb man bits`). A discovered prerequisite: MFBASIC has
  *no* bitwise integer operators (the `AND`/`OR`/`XOR` operators are
  logical/boolean), so `encoding` and the two macOS software cores
  (ChaCha20-Poly1305, Ed25519) cannot be written without it.
- **`encoding`** — hex and Base64/Base64url byte↔text (`src/spec/stdlib/08_encoding.md`).
  Hash/MAC/key/random outputs are raw `List OF Byte` and are unusable as text
  without these; `crypto` stringifies through `encoding`.

> **Why library-backed, not self-emitted.** The earlier draft of this plan emitted
> ARMv8 crypto instructions directly and shipped a `bits`-based software core for
> every primitive. With OpenSSL and CommonCrypto already linked, that work is
> redundant: the libraries are constant-time, audited, and already
> hardware-accelerated. Binding to them removes the SIMD-encoder and
> feature-detection phases entirely and is the project's stated preference
> ("rather use standard libs than roll my own"). We roll our own **only** for the
> macOS C-ABI gaps (Ed25519, ChaCha20-Poly1305).

It complements (canonical specs now live under `src/spec/**`, read via `./mfb spec <topic>`):

- `./mfb spec language builtin-functions` (universal `toString`/`toInt`;
  `src/spec/language/18_builtin-functions.md`) and `./mfb spec stdlib math-rng`
  (the PCG64 RNG — explicitly **not** cryptographic; `crypto` ships its own CSPRNG;
  `src/spec/stdlib/07_math-rng.md`). `tls` already binds OpenSSL3 / Network.framework
  (the linking machinery this plan reuses; `src/spec/stdlib/05_http.md`).
- `./mfb spec diagnostics error-codes` (the `7-705-*` generic runtime range; this
  plan reserves one new code, §B; `src/spec/diagnostics/02_error-codes.md`)
- `./mfb spec language error-model` (`TRAP`/`RECOVER`/`FAIL`;
  `src/spec/language/08_error-model.md`) and `./mfb spec language native-libraries`
  (the native `LINK` binding mechanism; `src/spec/language/17_native-libraries.md`;
  host-capability surfacing for randomness)
- `./mfb spec stdlib encoding` + `./mfb man bits` (the `bits` and `encoding`
  packages this plan consumes — for the software cores and for stringifying
  digests/keys; **plan-02, now landed**)
- `./mfb spec stdlib http` / the `csv`/`json` source packages (the source-package
  shim and wiring template `crypto`'s source layer mirrors)
- the native linking already in place (OpenSSL libcrypto on Linux, libSystem on
  macOS) and the OS-entropy seeding via `getentropy`/`_getentropy`

---

# Part A — `crypto` package

Called with the `crypto::` qualifier; `IMPORT crypto` needs no manifest
dependency. Inputs/outputs are `List OF Byte`; text overloads UTF-8-encode
internally. Stringification is via `encoding`.

## A.1 Types

```basic
TYPE Sealed
  ciphertext AS List OF Byte
  tag        AS List OF Byte   ' 16-byte authentication tag
END TYPE

TYPE KeyPair
  privateKey AS List OF Byte   ' sensitive — see the secret-safety note in §A.7
  publicKey  AS List OF Byte
END TYPE
```

Both are plain copyable records — public, constructible, thread-sendable. **No
enums** are defined (algorithms are concrete named functions, matching the `math` /
`datetime` style). Keys are raw bytes, **not** resource handles.

## A.2 Hashes

Output is the raw digest. Each has a `List OF Byte` and a `String` (UTF-8) overload.
Library-backed both platforms (OpenSSL `EVP_Digest`; CommonCrypto `CC_SHA*`).

| Function | Signature | Behavior |
|----------|-----------|----------|
| `crypto::sha256` | `FUNC sha256(data AS List OF Byte) AS List OF Byte` · `FUNC sha256(data AS String) AS List OF Byte` | SHA-256 (FIPS 180-4); 32-byte digest. |
| `crypto::sha224` | `FUNC sha224(data AS List OF Byte) AS List OF Byte` · `FUNC sha224(data AS String) AS List OF Byte` | SHA-224; 28-byte digest. |
| `crypto::sha512` | `FUNC sha512(data AS List OF Byte) AS List OF Byte` · `FUNC sha512(data AS String) AS List OF Byte` | SHA-512; 64-byte digest. |
| `crypto::sha384` | `FUNC sha384(data AS List OF Byte) AS List OF Byte` · `FUNC sha384(data AS String) AS List OF Byte` | SHA-384; 48-byte digest. |

## A.3 HMAC (RFC 2104)

Library-backed both platforms (OpenSSL `HMAC`; CommonCrypto `CCHmac`).

| Function | Signature | Behavior |
|----------|-----------|----------|
| `crypto::hmacSha256` | `FUNC hmacSha256(key AS List OF Byte, data AS List OF Byte) AS List OF Byte` · `FUNC hmacSha256(key AS List OF Byte, data AS String) AS List OF Byte` | HMAC-SHA-256; 32-byte MAC. |
| `crypto::hmacSha512` | `FUNC hmacSha512(key AS List OF Byte, data AS List OF Byte) AS List OF Byte` · `FUNC hmacSha512(key AS List OF Byte, data AS String) AS List OF Byte` | HMAC-SHA-512; 64-byte MAC. |

## A.4 Key derivation

PBKDF2 is library-backed (OpenSSL `PKCS5_PBKDF2_HMAC`; CommonCrypto
`CCKeyDerivationPBKDF`) — the iteration loop stays in native code for speed. HKDF
is thin source over the library-backed HMAC (CommonCrypto has no HKDF; building it
on `crypto::hmacSha256/512` is uniform across platforms and trivially correct).

| Function | Signature | Behavior |
|----------|-----------|----------|
| `crypto::hkdfSha256` | `FUNC hkdfSha256(ikm AS List OF Byte, salt AS List OF Byte, info AS List OF Byte, length AS Integer) AS List OF Byte` | HKDF (RFC 5869) extract-and-expand. `length` `1 .. 255*32`. Fails `77050002` out of range. |
| `crypto::hkdfSha512` | `FUNC hkdfSha512(ikm AS List OF Byte, salt AS List OF Byte, info AS List OF Byte, length AS Integer) AS List OF Byte` | As above with SHA-512; `length` up to `255*64`. |
| `crypto::pbkdf2Sha256` | `FUNC pbkdf2Sha256(password AS List OF Byte, salt AS List OF Byte, iterations AS Integer, length AS Integer) AS List OF Byte` · `FUNC pbkdf2Sha256(password AS String, salt AS List OF Byte, iterations AS Integer, length AS Integer) AS List OF Byte` | PBKDF2-HMAC-SHA-256 (RFC 8018). Fails `77050002` when `iterations < 1` or `length < 1`. |
| `crypto::pbkdf2Sha512` | `FUNC pbkdf2Sha512(password AS List OF Byte, salt AS List OF Byte, iterations AS Integer, length AS Integer) AS List OF Byte` · `FUNC pbkdf2Sha512(password AS String, salt AS List OF Byte, iterations AS Integer, length AS Integer) AS List OF Byte` | PBKDF2-HMAC-SHA-512. |

## A.5 Authenticated encryption (AEAD)

`seal` returns ciphertext + a 16-byte tag. `open` verifies the tag in constant
time and **fails closed** with `ErrAuthenticationFailed` (`77050016`, §B) on
mismatch, returning plaintext only on success. `aad` defaults to empty.

| Function | Signature | Behavior |
|----------|-----------|----------|
| `crypto::aes256GcmSeal` | `FUNC aes256GcmSeal(key AS List OF Byte, nonce AS List OF Byte, plaintext AS List OF Byte, aad AS List OF Byte = []) AS Sealed` | AES-256-GCM. `key` 32 bytes, `nonce` 12 bytes. Fails `77050002` on bad lengths. Library-backed both platforms (OpenSSL EVP; CommonCrypto `CCCryptorGCMOneshot`). |
| `crypto::aes256GcmOpen` | `FUNC aes256GcmOpen(key AS List OF Byte, nonce AS List OF Byte, ciphertext AS List OF Byte, tag AS List OF Byte, aad AS List OF Byte = []) AS List OF Byte` | Verifies and decrypts. Fails `77050016` on tag mismatch, `77050002` on bad lengths. |
| `crypto::chacha20Poly1305Seal` | `FUNC chacha20Poly1305Seal(key AS List OF Byte, nonce AS List OF Byte, plaintext AS List OF Byte, aad AS List OF Byte = []) AS Sealed` | ChaCha20-Poly1305 (RFC 8439). `key` 32 bytes, `nonce` 12 bytes. **OpenSSL on Linux; portable software core on macOS** (no CommonCrypto C ABI — see note). |
| `crypto::chacha20Poly1305Open` | `FUNC chacha20Poly1305Open(key AS List OF Byte, nonce AS List OF Byte, ciphertext AS List OF Byte, tag AS List OF Byte, aad AS List OF Byte = []) AS List OF Byte` | Verifies and decrypts. Fails `77050016` / `77050002`. |

> **Nonce discipline.** Nonces must be unique per key. Generate with
> `crypto::randomBytes(12)`, store/transmit alongside the ciphertext, and never
> reuse a `(key, nonce)` pair. The nonce is an explicit argument so the caller owns
> this invariant.
>
> **macOS ChaCha20-Poly1305.** CommonCrypto exposes no ChaCha20-Poly1305 C ABI
> (only Swift CryptoKit), so macOS uses the portable software core (Phase 3); §A is
> otherwise all-library on macOS. Linux uses OpenSSL. AES-256-GCM remains
> library-backed and hardware-accelerated on both platforms.

## A.6 Secure random

A cryptographically-secure generator from the OS/library CSPRNG (OpenSSL
`RAND_bytes`; macOS `getentropy`/`SecRandomCopyBytes`) — **distinct from
`math::rand`** (PCG64, non-cryptographic) and deliberately **not seedable**.

| Function | Signature | Behavior |
|----------|-----------|----------|
| `crypto::randomBytes` | `FUNC randomBytes(count AS Integer) AS List OF Byte` | `count` cryptographically-secure random bytes. Fails `77050002` when `count < 0`. |
| `crypto::randomInt` | `FUNC randomInt(min AS Integer, max AS Integer) AS Integer` | Uniform, **unbiased** (rejection-sampled) integer in inclusive `[min, max]`. Fails `77050002` when `min > max`. |
| `crypto::uuid4` | `FUNC uuid4() AS String` | A random (version-4) UUID, canonical lowercase `8-4-4-4-12` (RFC 4122). |

## A.7 Public-key — key generation & signatures

Keys are raw byte encodings (plain copyable `List OF Byte`, not resources) that
interoperate directly between OpenSSL and Security.framework. Key generation
returns both halves as a `KeyPair` (§A.1).

Raw encodings (fixed so every target agrees on the wire format):

| Algorithm | `privateKey` | `publicKey` |
|-----------|--------------|-------------|
| Ed25519 | 32-byte seed (RFC 8032 §5.1.5) | 32-byte compressed point |
| NIST P-256 | 32-byte big-endian scalar | 65-byte uncompressed point `04 ‖ X ‖ Y` |
| NIST P-384 | 48-byte scalar | 97-byte `04 ‖ X ‖ Y` |
| NIST P-521 | 66-byte scalar | 133-byte `04 ‖ X ‖ Y` |

| Function | Signature | Behavior |
|----------|-----------|----------|
| `crypto::generateEd25519` | `FUNC generateEd25519() AS KeyPair` | Ed25519 keypair; public key derived deterministically from a random 32-byte seed. |
| `crypto::generateP256` | `FUNC generateP256() AS KeyPair` | NIST P-256 (secp256r1) keypair. |
| `crypto::generateP384` | `FUNC generateP384() AS KeyPair` | NIST P-384 keypair. |
| `crypto::generateP521` | `FUNC generateP521() AS KeyPair` | NIST P-521 keypair. |
| `crypto::ed25519Sign` | `FUNC ed25519Sign(privateKey AS List OF Byte, message AS List OF Byte) AS List OF Byte` | 64-byte Ed25519 signature (RFC 8032, PureEdDSA). **Deterministic.** Fails `77050002` on wrong-length key. |
| `crypto::ed25519Verify` | `FUNC ed25519Verify(publicKey AS List OF Byte, message AS List OF Byte, signature AS List OF Byte) AS Boolean` | `TRUE` iff valid; `FALSE` (never fails) for invalid/malformed input. |
| `crypto::p256Sign` | `FUNC p256Sign(privateKey AS List OF Byte, message AS List OF Byte) AS List OF Byte` | ECDSA P-256 / SHA-256; DER X9.62 signature. **Non-deterministic.** Fails `77050002` on wrong-length key. |
| `crypto::p256Verify` | `FUNC p256Verify(publicKey AS List OF Byte, message AS List OF Byte, signature AS List OF Byte) AS Boolean` | `TRUE` iff valid; `FALSE` for invalid/malformed input. |
| `crypto::p384Sign` | `FUNC p384Sign(privateKey AS List OF Byte, message AS List OF Byte) AS List OF Byte` | ECDSA P-384 / SHA-384; DER signature. Non-deterministic. |
| `crypto::p384Verify` | `FUNC p384Verify(publicKey AS List OF Byte, message AS List OF Byte, signature AS List OF Byte) AS Boolean` | As above for P-384. |
| `crypto::p521Sign` | `FUNC p521Sign(privateKey AS List OF Byte, message AS List OF Byte) AS List OF Byte` | ECDSA P-521 / SHA-512; DER signature. Non-deterministic. |
| `crypto::p521Verify` | `FUNC p521Verify(publicKey AS List OF Byte, message AS List OF Byte, signature AS List OF Byte) AS Boolean` | As above for P-521. |

> **Provider split.** Linux backs all four algorithms with OpenSSL libcrypto.
> macOS backs the NIST curves with Security.framework (`SecKey`) and backs
> **Ed25519 with the portable software core** (RFC 8032) — Apple exposes Ed25519
> only through Swift CryptoKit, with no C ABI. Signatures interoperate: ECDSA is
> DER X9.62 on both; the Ed25519 software path matches OpenSSL byte for byte.
>
> **Determinism caveat.** Key generation is random, and **ECDSA signatures are
> non-deterministic** (fresh nonce each call), so those outputs are not
> reproducible across runs or targets — only Ed25519 signing is. Verification of a
> given (key, message, signature) is total and identical everywhere.
>
> **Secret safety.** `privateKey` bytes are sensitive. Per
> `./mfb spec language builtin-functions`, `typeName`/`toString`/diagnostics are not
> security boundaries — never log a `KeyPair`; redact private keys in any
> application output.

## A.8 Verification

| Function | Signature | Behavior |
|----------|-----------|----------|
| `crypto::constantTimeEqual` | `FUNC constantTimeEqual(a AS List OF Byte, b AS List OF Byte) AS Boolean` | Compares two byte lists in time independent of contents (length is not secret). Library-backed (`CRYPTO_memcmp` / `timingsafe_bcmp`). Use for all MAC/tag/digest comparisons; never compare secrets with `=`. |

---

# Part B — Error codes

One new runtime code is reserved in `src/spec/diagnostics/02_error-codes.md`
(`./mfb spec diagnostics error-codes`, the build input for `errorCode::`) and
exported by `errorCode`:

| Canonical | Integer | Name | Meaning |
|-----------|---------|------|---------|
| `7-705-0016` | `77050016` | `ErrAuthenticationFailed` | Authenticated decryption failed: the message authentication tag did not verify. |

Other failures reuse existing codes: `ErrInvalidArgument` (`77050002`) for bad
lengths/iterations/ranges/keys, `ErrInvalidFormat` (`77050003`) for `encoding`
decode failures, and `ErrUnknown` (`77050000`) for an unexpected library failure
(e.g. keygen). AEAD `open` **must** fail with `ErrAuthenticationFailed` and return
no plaintext on tag mismatch — failing closed is a security requirement.

---

# Part C — Implementation Plan

Library bindings use the native `LINK` mechanism
(`./mfb spec language native-libraries`, `src/spec/language/17_native-libraries.md`;
the native-link binding codegen). The `tls` package already links OpenSSL3 on Linux
and Network.framework/libSystem on macOS, so the linking and capability machinery
exists; this plan adds libcrypto / CommonCrypto / Security.framework symbols to it.

## Sub-plans

Split by effort into four small/medium sub-plans; each holds its phases, tasks, and acceptance.
The design above (Parts A/B) and below (Parts D/E) is the shared source of truth all four reference.

| Doc | Effort | Phases | Depends on |
|---|---|---|---|
| [plan-04-A](plan-04-A-bindings-symmetric.md) — bindings + symmetric core | large | prereqs (✅) · library bindings · source glue (HKDF/randomInt/uuid4) | plan-02 |
| [plan-04-B](plan-04-B-software-cores.md) — macOS software cores | large | ChaCha20-Poly1305 + Ed25519 on `bits`/`UMULH` | A |
| [plan-04-C](plan-04-C-public-key.md) — public-key bindings + dispatch | medium | Ed25519 + ECDSA keygen/sign/verify, per-target dispatch | A, B |
| [plan-04-D](plan-04-D-docs-tests.md) — man pages, docs, golden tests | medium | man pages · user docs · known-answer + negative + cross-platform tests | A, B, C |

---

# Part D — Worked examples

```basic
IMPORT crypto
IMPORT encoding

' Hash a string and print lowercase hex.
LET digest = crypto::sha256("hello world")
io::print(encoding::hexEncode(digest))      ' b94d27b9...

' HMAC and a timing-safe check. The annotation selects utf8Encode's
' List OF Byte overload (see ./mfb spec stdlib encoding); in argument
' position the parameter type selects it without one.
LET key AS List OF Byte = encoding::utf8Encode("secret")
LET mac = crypto::hmacSha256(key, "message")
LET ok  = crypto::constantTimeEqual(mac, expectedMac)

' Authenticated encryption with a fresh random nonce.
LET k     = crypto::randomBytes(32)
LET nonce = crypto::randomBytes(12)
LET box   = crypto::aes256GcmSeal(k, nonce, encoding::utf8Encode("attack at dawn"))
LET plain = crypto::aes256GcmOpen(k, nonce, box.ciphertext, box.tag) TRAP(e)
  IF e.code = errorCode::ErrAuthenticationFailed THEN RECOVER []   ' tampered
  FAIL e
END TRAP

' Ed25519 sign / verify.
LET kp  = crypto::generateEd25519()
LET sig = crypto::ed25519Sign(kp.privateKey, encoding::utf8Encode("release v1"))
LET good = crypto::ed25519Verify(kp.publicKey, encoding::utf8Encode("release v1"), sig)

io::print(crypto::uuid4())
```

---

# Part E — Divergences, errors, and non-goals

## E.1 Divergences from the source-package template

- `crypto` is the first **library-backed** standard package built on the native
  `LINK` mechanism plus a thin source layer — distinct from the pure-source
  `json`/`csv`/`regex`/`http` packages and from native-only `net`/`tls`. It depends
  on the `bits` and `encoding` packages (plan-02) for its software cores and for
  stringifying outputs.
- AEAD `open` returns plaintext on success but **fails closed** on tag mismatch —
  no "bytes plus a boolean" shape; verification is not optional.
- Hardware acceleration is **inherited from the system libraries** (which dispatch
  to ARMv8 Crypto Extensions / SHA-NI / AES-NI internally); this plan emits no SIMD
  crypto instructions and implements no CPU feature detection.

## E.2 Non-goals for this version

- **No legacy digests** (`md5`, `sha1`). May be added later, explicitly labeled
  "for legacy/checksum interop, not security."
- **Public-key is limited to** Ed25519 and NIST P-256/P-384/P-521 (keygen + sign/
  verify). **No** key agreement (X25519/X448, ECDH), **no** Ed448, **no** RSA, and
  **no** certificate/PKCS-format parsing — all deferred.
- **No password-hashing KDFs beyond PBKDF2** (Argon2, scrypt, bcrypt) — future.
- **No SHA-3/Keccak, BLAKE2/3.**
- **No streaming/incremental hash or AEAD** (one-shot byte-list API only); no
  detached-nonce sugar.
- **No insecure or configurable modes** (raw AES-CBC/ECB, custom GCM tag lengths).

## E.3 Future: x86_64

Because the cryptography is delegated to the system libraries, an x86_64 port
inherits hardware acceleration (SHA-NI / AES-NI / PCLMULQDQ) **for free** once the
same library bindings resolve — there is no per-arch encoder or feature-detection
work to do. The only architecture-specific code is the two software cores
(ChaCha20-Poly1305, Ed25519), which are plain integer arithmetic on `bits` + the
64-bit multiply and are already portable across AArch64 and x86_64.
