# crypto — cryptographic primitives

The `crypto` package provides cryptographic hashes, HMAC, key-derivation
functions, authenticated encryption (AEAD), a cryptographically-secure RNG,
public-key signatures, and constant-time comparison. Called with the `crypto::`
qualifier; `IMPORT crypto` needs no manifest dependency. Inputs and outputs are
`List OF Byte`; the `String` overloads UTF-8-encode internally. Outputs are
stringified through `encoding` (`./mfb spec stdlib encoding`).

The per-function API — signatures, parameters, return types, errors — is owned by
`./mfb man crypto`. This topic specifies the *behavior behind* that API: the
algorithm set, the backend split, and the security-relevant guarantees.

## Backend model (hybrid, no deprecated platform calls)

`crypto` is **software-first**: every hash, HMAC, KDF, AEAD, and Ed25519
primitive is a portable core implemented in injected MFBASIC source over the
`bits` package (`crypto_package.mfb`). Because each core computes the same
standard algorithm, its output is **byte-identical on every target**
(macOS/Linux, aarch64/x86-64), and it uses **no deprecated platform functions**.
This is a deliberate divergence from an earlier library-backed draft: on macOS
the only C-ABI symmetric/AEAD/EdDSA entry points are deprecated (`CC_SHA*`,
`CCCryptorGCM`) or Swift-only (CryptoKit), so a software core is both the
portable and the non-deprecated choice.

Two categories bind the platform instead of computing in source:

- **`randomBytes`** draws from the OS CSPRNG via `getentropy` (present and
  non-deprecated on macOS and Linux, glibc and musl). It is a native runtime
  helper (`_mfb_rt_crypto_crypto_randomBytes`). This is **distinct from
  `math::rand`** (PCG64, non-cryptographic; `./mfb spec stdlib math-rng`) and is
  deliberately **not seedable**.
- **NIST-EC public-key** (P-256/384/521 key generation and ECDSA) binds the
  platform's key API — `SecKey` (Security.framework) on macOS, `EVP_PKEY`
  (libcrypto) on Linux — rather than a software core: generic NIST bignum
  arithmetic is ~100× costlier than Ed25519's special-prime field and is
  impractical over `bits`. Both bindings use **no deprecated calls on any
  supported version**: macOS uses the non-deprecated `SecKey`
  create/sign/verify surface; Linux exchanges keys as DER through
  `d2i_AutoPrivateKey`/`d2i_PUBKEY` + `EVP_DigestSign`/`EVP_DigestVerify`, which
  are non-deprecated on both OpenSSL 1.1 and 3.x, and generates keys with
  `EVP_EC_gen` (OpenSSL 3.x) or `EC_KEY_*` (OpenSSL 1.1, where it is not
  deprecated). `libcrypto` is resolved at load time via `dlopen`
  (`libcrypto.so.3`, falling back to `libcrypto.so.1.1`).

  The two backends are **wire-compatible**: a key or signature produced on one
  platform is accepted by the other (and by OpenSSL/pyca). The agreed encodings,
  identical on every target, are

  - `KeyPair.privateKey` = `0x04 ‖ X ‖ Y ‖ K` — the SEC1 uncompressed point
    followed by the big-endian scalar (self-contained: 97 bytes for P-256, 145
    for P-384, 199 for P-521);
  - `KeyPair.publicKey` = `0x04 ‖ X ‖ Y` — the SEC1 uncompressed point (65 / 97 /
    133 bytes);
  - signatures = ASN.1 DER `Ecdsa-Sig-Value` (X9.62).

Hardware acceleration (AES-NI, SHA extensions) is not currently inherited by the
software cores; a future library-backed fast path could add it without changing
any output.

## Algorithm set

- **Hashes** — SHA-224, SHA-256, SHA-384, SHA-512 (FIPS 180-4).
- **HMAC** — HMAC-SHA-256, HMAC-SHA-512 (RFC 2104).
- **KDF** — HKDF-SHA-256/512 (RFC 5869, extract-and-expand over the HMAC cores);
  PBKDF2-HMAC-SHA-256/512 (RFC 8018).
- **AEAD** — AES-256-GCM (NIST SP 800-38D) and ChaCha20-Poly1305 (RFC 8439).
  `seal` returns ciphertext plus a 16-byte tag; `open` verifies the tag in
  constant time and **fails closed** with `ErrAuthenticationFailed`
  (`77050016`), returning plaintext only on success. `aad` defaults to empty.
- **Secure random** — `randomBytes` (raw bytes), `randomInt` (uniform, unbiased,
  rejection-sampled, inclusive `[min, max]`), `uuid4` (random version-4 UUID,
  canonical lowercase `8-4-4-4-12`, RFC 4122).
- **Public-key** — Ed25519 (RFC 8032, PureEdDSA, deterministic signing) plus
  ECDSA over NIST P-256/384/521 (FIPS 186; SHA-256/384/512 respectively; DER
  X9.62 signatures, non-deterministic). Key generation returns a `KeyPair`.
- **Verification** — `constantTimeEqual` compares two byte lists in time
  independent of their contents (length is not secret).

## Numeric representation

The software cores keep 32-bit arithmetic masked to `0..2^32-1` (a sum of two such
values is at most `2^33-2`, within the trapping 63-bit `+`, and is masked back).
64-bit modular addition (SHA-512, Poly1305) is done through a limb-split helper
that never lets an intermediate cross `2^63`. Ed25519 field elements use 16 × 16-bit
limbs (TweetNaCl representation), whose products stay well within range.

## Security notes

- **Nonce discipline.** AEAD nonces must be unique per key. Generate with
  `crypto::randomBytes(12)`, store/transmit alongside the ciphertext, and never
  reuse a `(key, nonce)` pair.
- **Fail closed.** AEAD `open` returns no plaintext on tag mismatch — verification
  is not optional.
- **Secret safety.** `KeyPair.privateKey` bytes are sensitive; `typeName` /
  `toString` / diagnostics are not security boundaries. Never log a `KeyPair`.
- **Determinism.** Key generation is random and ECDSA signatures are
  non-deterministic (fresh nonce per call), so those outputs are not reproducible
  across runs; only Ed25519 signing is. Verification of a given
  `(key, message, signature)` is total and identical everywhere.

## See Also

* `./mfb man crypto` — the per-function API reference.
* `./mfb spec stdlib encoding` — hex/Base64 stringification of digests and keys.
* `./mfb spec stdlib math-rng` — the non-cryptographic `math::rand` PCG64 RNG.
* `./mfb spec diagnostics error-codes` — `ErrAuthenticationFailed` and the shared
  `7-705-*` runtime codes.
