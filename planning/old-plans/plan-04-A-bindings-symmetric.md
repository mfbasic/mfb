# plan-04-A: crypto — bindings + symmetric core

Last updated: 2026-06-28
Effort: large

Part **A** of plan-04 (Built-in `crypto` Package). Lands the library-binding skeleton and the
symmetric primitives (hashes, HMAC, KDF, AEAD, CSPRNG, constant-time compare) plus the pure source
glue over them. Shared design (the `crypto` surface, types, error codes, examples) lives in the
overview: [plan-04-crypto.md](plan-04-crypto.md).

- **Depends on:** plan-02 `bits`/`encoding` (landed).
- **Blocks:** plan-04-B (software cores reuse `crypto::sha512` + `bits`), plan-04-C (public-key),
  plan-04-D (docs + tests).
- **Spec/design:** overview Part A (§A.1–§A.6 hashes/HMAC/KDF/AEAD/random), Part B (error codes).

## Phases

### Phase A1 — Prerequisites: `bits` and `encoding` (✅ landed)

- [x] `bits` native single-instruction ops shipped (`src/builtins/bits.rs`; `./mfb man bits`).
- [x] `encoding` shipped (`src/builtins/encoding.rs`, `src/builtins/encoding_package.mfb`; `src/spec/stdlib/08_encoding.md`).

Acceptance: plan-02's `bits`/`encoding` are present and `crypto` can stringify/build on them — confirmed landed; no work in this plan.
Commit: landed in plan-02

### Phase A2 — `crypto` library bindings (the core)

Native `LINK` bindings + the `crypto` shim/source package skeleton.

- [ ] `src/builtins/crypto.rs` (shim) + `src/builtins/crypto_package.mfb` (export `Sealed`, `KeyPair`; entry points validate lengths then dispatch).
- [ ] Linux (OpenSSL libcrypto): `EVP_Digest` (SHA-2), `HMAC`, `PKCS5_PBKDF2_HMAC`, AES-256-GCM + ChaCha20-Poly1305 via `EVP_CipherInit/Update/Final` + GCM/AEAD ctrl, `RAND_bytes`, `CRYPTO_memcmp`.
- [ ] macOS (CommonCrypto/libSystem): `CC_SHA256/224/512/384`, `CCHmac`, `CCKeyDerivationPBKDF`, `CCCryptorGCMOneshot` (AES-GCM), `getentropy` (`randomBytes`), `timingsafe_bcmp` (`constantTimeEqual`).
- [ ] Wire SHA-2, HMAC, PBKDF2, AES-256-GCM, `randomBytes`, `constantTimeEqual` to the bindings on both platforms; surface the **randomness** host capability for the CSPRNG (as `math::rand` does).

Acceptance: SHA-2 / HMAC / PBKDF2 / AES-256-GCM / `randomBytes` / `constantTimeEqual` produce correct known-answer results on both Linux (OpenSSL) and macOS (CommonCrypto) through the `crypto` package.
Commit: —

### Phase A3 — source glue over library primitives

Pure source in `crypto_package.mfb` (works on native + BR, no new bindings).

- [ ] HKDF (`hkdfSha256`/`hkdfSha512`) — RFC 5869 extract/expand over `crypto::hmacSha256/512`.
- [ ] `randomInt` — rejection sampling over `crypto::randomBytes`.
- [ ] `uuid4` — 16 `randomBytes`, set version/variant nibbles via `bits`, format with `encoding::hexEncode` + dashes.

Acceptance: HKDF matches RFC 5869 vectors, `randomInt` is unbiased, and `uuid4` has correct version/variant nibbles + format — all on native and BR with no new bindings.
Commit: —
