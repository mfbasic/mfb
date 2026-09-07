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

`crypto` is **software-first**: every hash (SHA-1, SHA-2, SHA-3, SHAKE256, and the
BLAKE2b-512 behind Argon2id), HMAC, KDF, AEAD, and Ed25519
primitive is a portable core implemented in injected MFBASIC source over the
`bits` package. Because each core computes the same
standard algorithm, its output is **byte-identical on every target**
(macOS/Linux, aarch64/x86-64/riscv64), and it uses **no deprecated platform
functions**. The software cores are used in preference to platform crypto
libraries because, on macOS,
the only C-ABI symmetric/AEAD/EdDSA entry points are deprecated (`CC_SHA*`,
`CCCryptorGCM`) or Swift-only (CryptoKit), so a software core is both the
portable and the non-deprecated choice.

Two categories bind the platform instead of computing in source:

- **`randomBytes`** draws from the OS CSPRNG via `getentropy` (present and
  non-deprecated on macOS and Linux, glibc and musl). It is a native runtime
  helper. This is **distinct from
  `math::rand`** (PCG64, non-cryptographic; `./mfb spec stdlib math-rng`) and is
  deliberately **not seedable**. [[src/codegen/builtins/crypto/func_random_bytes.rs:randomBytes]]
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
  platform is accepted by the other. The agreed encodings, identical on every
  target, are

  - `KeyPair.privateKey` = `0x04 ‖ X ‖ Y ‖ K` — the SEC1 uncompressed point
    followed by the big-endian scalar (self-contained: 97 bytes for P-256, 145
    for P-384, 199 for P-521);
  - `KeyPair.publicKey` = `0x04 ‖ X ‖ Y` — the SEC1 uncompressed point (65 / 97 /
    133 bytes);
  - signatures = ASN.1 DER `Ecdsa-Sig-Value` (X9.62).

  Two of those three are also **externally** interoperable, and one is not. The
  public key is the standard SEC1 / X9.62 uncompressed point and the signature is
  the standard X9.62 DER structure, so OpenSSL and pyca read both as they stand.
  The private key is **package-local**: `0x04 ‖ X ‖ Y ‖ K` is neither the raw SEC1
  scalar nor a SEC1/PKCS#8 DER wrapper, and no other ecosystem parses it. It is
  chosen so that `crypto::sign` needs only the private key, never the public half
  alongside. Conversion is byte surgery, not a re-encoding: the leading
  `1 + 2·field` bytes are exactly the public key and the trailing `field` bytes
  are the SEC1 scalar `K`, so `publicKey ‖ K` builds the private form and slicing
  recovers both halves. `mfb man crypto generate` carries the `openssl`
  incantation in each direction. No member of `crypto` reads or writes DER- or
  PEM-framed keys; that would need a general ASN.1 codec the package does not
  have (bug-516).

The software cores do not use hardware crypto acceleration (AES-NI, SHA
extensions); computation is portable-arithmetic only, identical across targets.

## Algorithm set

- **Hashes** — the `crypto::Hash` selector: `SHA1` (FIPS 180-4, 20-byte digest,
  legacy); the SHA-2 family `SHA2_224`, `SHA2_256`, `SHA2_384`, `SHA2_512`
  (FIPS 180-4; 28/32/48/64 bytes); and the SHA-3 family `SHA3_224`, `SHA3_256`,
  `SHA3_384`, `SHA3_512` (FIPS 202; the Keccak-f[1600] sponge at rate
  1152/1088/832/576 bits, domain suffix `0x06`, `pad10*1`; 28/32/48/64 bytes).
  The family prefix is part of every spelling;
  there are no bare `SHA256`-style aliases. `Hash.SHA1` reports the non-fatal
  `CRYPTO_SHA1_INSECURE` warning (`2-203-0136`, see
  `./mfb spec diagnostics rule-codes`) — SHA-1 is not collision-resistant, so it
  exists only for interoperability with systems that require it; the program
  still builds and the digest is the standard value. The warning is scoped to the
  **use**: `Hash.SHA1` written directly as the hash selector of `hmac`, `hkdf` or
  `pbkdf2` reports nothing, because none of those rests on collision resistance
  (HMAC's security follows from the compression function behaving as a PRF), so
  HMAC-SHA1 for RFC 6238 TOTP or WPA2 is a sound choice rather than a legacy
  concession. Every other occurrence reports, `hash` included.
  [[src/codegen/builtins/crypto/mod.rs:CRYPTO]]
- **XOF** — `shake256(data, length)`: SHAKE256 (FIPS 202 §6.2; the same sponge
  at rate 1088 with domain suffix `0x1f`) squeezed to any `length ≥ 1`; a shorter
  output is a prefix of a longer one. It is also the Ed448 hash (RFC 8032 §5.2).
  [[src/codegen/builtins/crypto/func_shake256.rs:register]]
- **HMAC** — HMAC over every `Hash` selector (RFC 2104): HMAC-SHA1,
  HMAC-SHA-224/256/384/512, and HMAC-SHA3-224/256/384/512 (block size = the
  sponge rate, FIPS 202 §7).
- **KDF** — HKDF over every `Hash` selector (RFC 5869, extract-and-expand over
  the HMAC core; output ceiling `255 × L` for the selector's digest length `L`);
  PBKDF2-HMAC over every `Hash` selector (RFC 8018); and Argon2id version `0x13`
  (RFC 9106), the memory-hard password hash, over a BLAKE2b-512 core (RFC 7693).
  [[src/codegen/builtins/crypto/helper_argon2id.rs:BODY]]
  `argon2id` is the specified answer for **storing** a password and PBKDF2 is the
  answer for RFC 8018 / WPA2 interoperation; the two are not interchangeable.
  Its cost parameters are validated in one place, before any memory is taken:
  `parallelism` in `1..=16777215`, `iterations ≥ 1`, `length ≥ 4`, `len(salt) ≥ 8`,
  and `memoryKiB` in `8 × parallelism ..= 2097152` — RFC 9106 §4's largest
  recommended memory. Every out-of-range value raises `ErrInvalidArgument`
  (`77050002`); an over-large `memoryKiB` is refused rather than attempted. The
  `crypto::Argon2Profile` overload is a wrapper that resolves its variant to those
  same three parameters and calls the explicit one, so both spellings share one
  validation site and one memory-fill site and are byte-identical for the same
  costs. `Minimum` is `(19456, 2, 1)` (the OWASP Password Storage Cheat Sheet's
  minimum configuration) and `Recommended` is `(65536, 3, 4)` (RFC 9106 §4's
  SECOND RECOMMENDED option).
  [[src/codegen/builtins/crypto/helper_argon2id_profile.rs:BODY]]
- **AEAD** — AES-256-GCM (NIST SP 800-38D) and ChaCha20-Poly1305 (RFC 8439).
  `seal` returns ciphertext plus a 16-byte tag; `open` verifies the tag in
  constant time and **fails closed** with `ErrAuthenticationFailed`
  (`77050016`), returning plaintext only on success. `aad` defaults to empty.
- **Secure random and identifiers** — `randomBytes` (raw bytes), `randomInt`
  (uniform, unbiased, rejection-sampled, inclusive `[min, max]`), `uuid4`
  (random version-4 UUID, canonical lowercase `8-4-4-4-12`), `uuid7` (RFC 9562
  version-7 UUID with a 48-bit Unix-millisecond prefix and 74 random bits), and
  `ulid` (canonical 26-character Crockford Base32 with a 48-bit
  Unix-millisecond prefix and 80 random bits).
- **Public-key** — Ed25519 and Ed448 (RFC 8032 PureEdDSA with the empty context,
  deterministic signing; Ed448 over edwards448 with SHAKE256 and `dom4`: 57-byte
  seed/public key, 114-byte `R‖S`, verification rejects a non-canonical `S ≥ L`,
  a non-canonical or off-curve point, a dirty sign byte, and a small-order
  public key or `R`) plus ECDSA over NIST P-256/384/521 (FIPS 186;
  SHA-256/384/512 respectively; DER X9.62 signatures, non-deterministic). Key
  generation returns a `KeyPair`.
  [[src/codegen/builtins/crypto/helper_ed448_decode.rs:BODY]]
  [[src/codegen/builtins/crypto/helper_ed448_sign.rs:BODY]]
- **Public-key encryption** — `encrypt`/`decrypt` are **RFC 9180 HPKE**,
  single-shot base mode (`mode_base`, no PSK, empty `info`, sequence number 0),
  selected by `AsymmetricCipher`: the `Ed25519_*` suites are DHKEM(X25519,
  HKDF-SHA256) `0x0020` / HKDF-SHA256 `0x0001` (`Nenc` = 32, `Nsecret` = 32),
  the `Ed448_*` suites DHKEM(X448, HKDF-SHA512) `0x0021` / HKDF-SHA512 `0x0003`
  (`Nenc` = 56, `Nsecret` = 64), each with AES-256-GCM `0x0002`
  (`*_AES256GCM`) or ChaCha20Poly1305 `0x0003` (`*_CHACHA20POLY1305`). The
  profile is read by explicit property of the selector (`__crypto_hpkeIsX448`,
  `__crypto_hpkeIsAesGcm`), never by ordinal arithmetic. The recipient's
  Ed25519/Ed448 signing key is mapped to X25519/X448 by the `convert` maps; the
  wire value is exactly `enc ‖ ct` (`ct` = ciphertext ‖ 16-byte tag), so it
  interoperates with any conformant HPKE implementation and the pre-RFC
  `mfb-box-v1` construction is no longer accepted. `decrypt` fails closed:
  `ErrAuthenticationFailed` on any tamper / wrong key / wrong suite (including
  the other curve's suite) / wrong `aad` / legacy box, `ErrInvalidArgument` on
  a box shorter than `Nenc` + 16 bytes (48 / 72), a wrong-length recipient key,
  or a low-order `enc`. Proven against the RFC's Appendix A vectors and both
  ways, for all four profiles, against an independent implementation
  (`tests/interop/rt_crypto_hpke_interop.rs`).
  [[src/codegen/builtins/crypto/helper_hpke_profile.rs:BODY]]
  [[src/codegen/builtins/crypto/helper_hpke_seal_with.rs:BODY]]
  [[src/codegen/builtins/crypto/helper_hpke_key_schedule.rs:BODY]]
- **Key agreement** — `Certificate.X25519` (RFC 7748, 32-byte keys; a 255-step
  ladder over the 16 × 16-bit-limb GF(2^255−19) field) and `Certificate.X448`
  (RFC 7748, 56-byte keys; a 448-step ladder over a 16 × 28-bit-limb
  GF(2^448−2^224−1) field) through `exchange(type, privateKey, publicKey)`,
  which **fails closed** with `ErrInvalidArgument` on a signing certificate, a
  wrong key length, or an all-zero shared secret (a low-order peer point, RFC
  7748 §6.1; the all-zero test scans the whole secret with no early exit).
  **Both** ladders run a fixed number of iterations and swap their state with a
  branch-free masked select — `__crypto_sel25519` and `__crypto_gf448Select`,
  each computing `a XOR (mask AND (a XOR b))` per limb under a `0 − bit` mask —
  so no control flow depends on the private scalar. Each curve's canonical
  packer reduces mod `p` through the same select on the borrow, so the shared
  secret's representative is chosen without a branch either. `sign`/`verify`
  reject both. [[src/codegen/builtins/crypto/helper_x25519.rs:BODY]]
  [[src/codegen/builtins/crypto/helper_sel25519.rs:BODY]]
  [[src/codegen/builtins/crypto/helper_x448.rs:BODY]]
  [[src/codegen/builtins/crypto/helper_exchange.rs:BODY]]
- **Key conversion** — `convert(KeyConvert.Ed25519ToX25519, keys)` (libsodium's
  `crypto_sign_ed25519_{pk,sk}_to_curve25519` maps) and
  `convert(KeyConvert.Ed448ToX448, keys)`: the public key by the RFC 7748 §4.2
  edwards448→curve448 4-isogeny `u = y²/x²` (evaluated as
  `y²·(1 − d·y²)/(1 − y²)`, no square root), the private key as
  `SHAKE256(seed)[0..56]` — libdecaf's `decaf_ed448_convert_*_to_x448`
  convention, under which the edwards448 base point maps to `u = 5` and
  `X448(convertedPrivate, 5) = convertedPublic`. Both maps check the input
  lengths (32 bytes for `Ed25519ToX25519`, 57 for `Ed448ToX448`) **and then
  verify the source curve** before mapping: RFC 8032 defines the public key as a
  derivation of the seed, so `convert` re-derives it (`A =
  [clamp(SHA-512(seed)[0..32])]B` for Ed25519, `A = [prune(SHAKE256(seed,
  114)[0..57])]B` for Ed448) and requires it to equal `keys.publicKey`, compared
  in constant time; anything else raises `ErrInvalidArgument` rather than being
  mis-mapped. Length alone is not sufficient — an X25519 pair is 32 bytes on both
  halves exactly like an Ed25519 pair — and the derivation check, unlike any test
  of the public key's encoding, cannot be fooled by a Montgomery `u` that decodes
  as a valid Edwards `y`. It accepts every pair `generate(Certificate.Ed25519)` /
  `generate(Certificate.Ed448)` and every conformant RFC 8032 implementation
  produce, and costs one fixed-base scalar multiplication.
  [[src/codegen/builtins/crypto/helper_ed448_pub_to_x448.rs:BODY]]
  [[src/codegen/builtins/crypto/helper_ed448_priv_to_x448.rs:BODY]]
- **Verification** — `constantTimeEqual` compares two byte lists in time
  independent of their contents (length is not secret).

## Numeric representation

The software cores keep 32-bit arithmetic masked to `0..2^32-1` (a sum of two such
values is at most `2^33-2`, within the trapping 63-bit `+`, and is masked back);
SHA-1 shares SHA-256's 512-bit padding and masked-32-bit model, differing only in
its 80-word schedule, round function, and constants.
[[src/codegen/builtins/crypto/helper_sha1_bytes.rs:BODY]]
Curve448 field elements use 16 × 28-bit limbs: a schoolbook product's largest
folded accumulator is 38 products of two `2^28` limbs (`< 2^61.3`), pinned by the
`gf448_mul_accumulators_fit_i63` unit test, and every field op returns carried
limbs so that bound holds for the next product.
[[src/codegen/builtins/crypto/helper_gf448_mul.rs:BODY]]
Keccak-f[1600] keeps each of its 25 lanes in one `Integer` as a raw 64-bit bit
pattern (the same representation as SHA-512's words) and touches it only through
`bits::` XOR/AND/NOT/`rl64` — Keccak has no addition, so no lane ever enters the
trapping arithmetic operators, and a lane with bit 63 set is simply a negative
`Integer`. The round is branch-free: every loop bound is a constant and every
index is a loop counter or an entry of the public rho/pi tables; the sponge's
only conditional tests the public output length.
[[src/codegen/builtins/crypto/helper_keccak_round.rs:BODY]]
[[src/codegen/builtins/crypto/helper_keccak_sponge.rs:BODY]]
SHA-512's 64-bit modular addition is done through a limb-split helper that never
lets an intermediate cross `2^63`. Poly1305 uses a 5 × 26-bit limb representation
(poly1305-donna) with explicit carry propagation. Ed25519 field elements use
16 × 16-bit limbs (TweetNaCl representation), whose products stay well within range.

## Security notes

- **Nonce discipline.** AEAD nonces must be unique per key. Generate with
  `crypto::randomBytes(12)`, store/transmit alongside the ciphertext, and never
  reuse a `(key, nonce)` pair.
- **Fail closed.** AEAD `open` returns no plaintext on tag mismatch — verification
  is not optional.
- **Curve attribution.** `KeyPair` carries no field naming the curve that
  produced it, and curves collide on size: Ed25519 and X25519 keys are both 32
  bytes on both halves (Ed448 is 57 and X448 56, so those two differ by one). A
  wrong-curve key is therefore **decidable exactly when a member is handed both
  halves of one pair**, which is `convert` alone — it re-derives one half from
  the other, as above. Every other member sees a single key: `encrypt` /
  `decrypt` a lone recipient key, `exchange` a lone private key beside a peer's
  public key, `sign` / `verify` a lone key with the curve named separately. For
  those, the curve is taken from the `AsymmetricCipher` / `Certificate` selector
  and the key bytes are trusted; no check is possible, because a 32-byte key of
  one curve is indistinguishable from a 32-byte key of the other, and a shape
  test on the public key would pass a Montgomery `u` about half the time.
  Wrong-curve use there is silent at the call and surfaces later — an
  `ErrAuthenticationFailed` at the recipient's `decrypt`, a shared secret the
  peer never reproduces, a signature under an unpublished public key — so the
  program, not the runtime, is what tracks the curve of each key.
- **Secret safety.** `KeyPair.privateKey` bytes are sensitive; `typeName` /
  `toString` / diagnostics are not security boundaries. Never log a `KeyPair`.
- **Determinism.** Key generation is random and ECDSA signatures are
  non-deterministic (fresh nonce per call), so those outputs are not reproducible
  across runs; only Ed25519 and Ed448 signing are. Verification of a given
  `(key, message, signature)` is total and identical everywhere.

## See Also

* `./mfb man crypto` — the per-function API reference.
* `./mfb spec stdlib encoding` — hex/Base64 stringification of digests and keys.
* `./mfb spec stdlib math-rng` — the non-cryptographic `math::rand` PCG64 RNG.
* `./mfb spec diagnostics error-codes` — `ErrAuthenticationFailed` and the shared
  `7-705-*` runtime codes.
