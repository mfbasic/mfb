# plan-04-D: crypto — man pages, docs, golden tests

Last updated: 2026-06-28
Effort: medium

Part **D** of plan-04 (Built-in `crypto` Package). Closes the feature: man pages, user
documentation, and the golden known-answer + negative + cross-platform test suite. Shared design
lives in the overview: [plan-04-crypto.md](plan-04-crypto.md).

- **Depends on:** plan-04-A, plan-04-B, plan-04-C (documents and tests the whole surface).
- **Spec/design:** overview Part A (surface), Part B (error codes), Part D (worked examples).

## Phases

### Phase D1 — Man pages

- [ ] Author per-function pages under `src/man/builtins/crypto/` (one `.txt` per function + `package.txt`) following `.ai/man_template.txt` / `.ai/man_type_template.txt`; `build.rs` compiles them into `generated::MAN_PACKAGES` and `src/man/mod.rs` surfaces them — add `crypto` to that module's `PACKAGE_ORDER` (precedent: `bits`/`encoding` from plan-02).
- [ ] Cite FIPS 180-4, FIPS 186 (ECDSA), RFC 2104/5869/8018/8439/8032/4122/4648; include the nonce-uniqueness, private-key secret-safety, and constant-time warnings.

Acceptance: `mfb man crypto` and every per-function page render via the man pipeline with the required citations and safety warnings.
Commit: —

### Phase D2 — User documentation

- [ ] New stdlib topic `src/spec/stdlib/09_crypto.md` (+ a `spec.md` index entry), mirroring `07_math-rng.md` / `08_encoding.md`; cross-reference `07_math-rng.md` noting `math::rand` is non-cryptographic and pointing to `crypto::randomBytes`.
- [ ] `src/spec/diagnostics/02_error-codes.md`: add `7-705-0016 ErrAuthenticationFailed`.
- [ ] `src/spec/language/17_native-libraries.md` (and the stdlib index): list the `crypto` package; note it links libcrypto (Linux) / CommonCrypto+Security.framework (macOS) and depends on `bits`/`encoding` (plan-02).

Acceptance: the crypto stdlib topic, the `ErrAuthenticationFailed` error-code entry, and the native-libraries listing are published and cross-referenced; `mfb spec` surfaces them.
Commit: —

### Phase D3 — Tests (golden)

- [ ] Known-answer vectors: FIPS 180-4 (SHA-2), RFC 4231 (HMAC), RFC 5869 (HKDF), RFC 6070 (PBKDF2), NIST GCM (AES-256-GCM), RFC 8439 §2.8.2 (ChaCha20-Poly1305), RFC 8032 (Ed25519), NIST CAVP (ECDSA verify), RFC 4648 (Base64).
- [ ] Negative: AEAD tag tamper → `ErrAuthenticationFailed`; bad key/nonce length → `ErrInvalidArgument`; malformed hex/Base64 → `ErrInvalidFormat`.
- [ ] Equality matrices: native↔BR identical; macOS↔Linux identical for all deterministic outputs; Ed25519 macOS-SW↔OpenSSL identical; cross-platform verify (both ECDSA and Ed25519).
- [ ] CSPRNG sanity: length, distribution smoke test, `randomInt` unbiasedness, `uuid4` version/variant nibbles and format.
- [ ] `constantTimeEqual`: equal / unequal / different-length inputs.

Acceptance: all known-answer, negative, equality-matrix, CSPRNG, and constant-time tests pass on both platforms; the acceptance suite is green.
Commit: —
