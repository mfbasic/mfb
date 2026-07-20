# crypto

Cryptographic hashes, HMAC, KDFs, authenticated encryption, a secure RNG, public-key signatures, and constant-time comparison

## Synopsis

```
IMPORT crypto
crypto::sha256(bytes)
crypto::aes256GcmSeal(key, nonce, plaintext)
crypto::chacha20Poly1305Open(key, nonce, ciphertext, tag)
crypto::randomBytes(32)
crypto::generateEd25519()
```

## Description

The `crypto` package provides cryptographic hashes, HMAC, key-derivation
functions, authenticated encryption (AEAD), a cryptographically-secure random
generator, public-key signatures, and constant-time comparison. It is a built-in
package, so `IMPORT crypto` needs no manifest dependency. [[src/builtins/crypto.rs:augmented_project]]

Inputs and outputs are `List OF Byte`; the hash/HMAC/PBKDF2 functions also accept
a `String` overload that UTF-8-encodes internally. A digest, ciphertext, or key
is raw binary — stringify it for display or storage with the `encoding` package
(`encoding::hexEncode`, `encoding::base64Encode`). The package defines two record
types, `crypto::Sealed` and `crypto::KeyPair`; see `mfb man crypto types`.

### Backend model (hybrid, no deprecated platform calls)

`crypto` is **software-first**: every hash, HMAC, KDF, AEAD, and Ed25519
primitive is a portable core written in MFBASIC source over the `bits` package
(`crypto_package.mfb`). Because each core computes the same standard algorithm,
its output is **byte-identical on every target** (macOS/Linux, aarch64/x86-64)
and uses **no deprecated platform functions**. On macOS the only C-ABI
symmetric/AEAD/EdDSA entry points are deprecated or Swift-only, so a software
core is both the portable and the non-deprecated choice. [[src/builtins/crypto_package.mfb:__crypto_aes256GcmSeal]]

Two categories bind the platform instead of computing in source:

- **`randomBytes`** draws from the OS CSPRNG via `getentropy` (present and
  non-deprecated on macOS and Linux, glibc and musl). It is a native runtime
  helper, is **distinct from `math::rand`** (PCG64, non-cryptographic), and is
  deliberately **not seedable**.
- **NIST-EC public-key** (P-256/384/521 key generation and ECDSA) binds the
  platform's modern key API — `SecKey` (Security.framework) on macOS, `EVP_PKEY`
  (libcrypto) on Linux — because generic NIST bignum arithmetic is impractical
  over `bits`. Both bindings use no deprecated calls on any supported version,
  and the two backends are **wire-compatible**: a key or signature produced on
  one platform is accepted by the other (and by OpenSSL/pyca).

### Algorithm set

- **Hashes** — SHA-224, SHA-256, SHA-384, SHA-512 (FIPS 180-4).
- **HMAC** — HMAC-SHA-256, HMAC-SHA-512 (RFC 2104).
- **KDF** — HKDF-SHA-256/512 (RFC 5869); PBKDF2-HMAC-SHA-256/512 (RFC 8018).
- **AEAD** — AES-256-GCM (NIST SP 800-38D) and ChaCha20-Poly1305 (RFC 8439).
  `seal` returns a `crypto::Sealed` (ciphertext plus a 16-byte tag); `open`
  verifies the tag in constant time and **fails closed** with
  `ErrAuthenticationFailed`, returning plaintext only on success. `aad` defaults
  to empty.
- **Secure random** — `randomBytes` (raw bytes), `randomInt` (uniform, unbiased,
  rejection-sampled, inclusive `[min, max]`), `uuid4` (random version-4 UUID,
  canonical lowercase `8-4-4-4-12`, RFC 4122).
- **Public-key** — Ed25519 (RFC 8032, deterministic signing) plus ECDSA over
  NIST P-256/384/521 (FIPS 186; SHA-256/384/512; DER X9.62 signatures,
  non-deterministic). Key generation returns a `crypto::KeyPair`.
- **Verification** — `constantTimeEqual` compares two byte lists in time
  independent of their contents.

### Security notes

- **Nonce discipline.** AEAD nonces must be unique per key — this is the single
  most important rule when using `aes256GcmSeal` or `chacha20Poly1305Seal`.
  Reusing a `(key, nonce)` pair is catastrophic: it leaks the XOR of the
  plaintexts and can expose the authentication key, breaking both
  confidentiality and integrity. Generate a fresh nonce for every message with
  `crypto::randomBytes(12)`, store or transmit it alongside the ciphertext (the
  nonce is not secret), and **never reuse a `(key, nonce)` pair**.
- **Fail closed.** AEAD `open` returns no plaintext on tag mismatch —
  verification is not optional. A failed tag check raises
  `ErrAuthenticationFailed` and the message must be rejected.
- **Secret safety.** `crypto::KeyPair.privateKey` bytes are sensitive; `typeName`,
  `toString`, and diagnostics are not security boundaries. Never log a `KeyPair`.
- **Cross-platform compatibility.** Every software core is byte-identical across
  targets, and the NIST-EC bindings are wire-compatible with OpenSSL and pyca, so
  keys, digests, and ciphertexts interoperate across platforms.
- **Determinism.** Key generation is random and ECDSA signatures are
  non-deterministic (fresh nonce per call), so those outputs are not reproducible
  across runs; only Ed25519 signing is. Verification of a given
  `(key, message, signature)` is total and identical everywhere.

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | an argument is out of range or the wrong length — an AEAD `key` that is not 32 bytes or `nonce` that is not 12 bytes; an HKDF/PBKDF2 length or iteration count out of range; `randomInt` called with `min > max` or too large a range; a signing private key of the wrong length [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |
| `77050016` | `ErrAuthenticationFailed` | AEAD `open` (`aes256GcmOpen`, `chacha20Poly1305Open`) when the authentication tag does not verify — the ciphertext, tag, nonce, or aad was altered or does not belong to the key. Fails closed: no plaintext is returned [[src/docs/spec/diagnostics/02_error-codes.md]] |
