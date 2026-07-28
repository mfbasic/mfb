# plan-04-C: crypto — public-key bindings + dispatch

Last updated: 2026-06-28
Effort: medium

Part **C** of plan-04 (Built-in `crypto` Package). Adds public-key keygen/sign/verify — Ed25519 and
ECDSA (P-256/384/521) — bound to OpenSSL on Linux and Security.framework on macOS (macOS Ed25519
routes to the plan-04-B software core), with per-target dispatch. Shared design lives in the
overview: [plan-04-crypto.md](plan-04-crypto.md).

- **Depends on:** plan-04-A (bindings) and plan-04-B (macOS Ed25519 software core).
- **Blocks:** plan-04-D (tests cover cross-platform verify).
- **Spec/design:** overview §A.7 (key generation & signatures), §A.8 (verification).

## Phases

### Phase C1 — Public-key bindings + dispatch

- [ ] Linux (OpenSSL): `EVP_PKEY_keygen`/`EVP_EC_gen` for keygen; `EVP_PKEY_get_raw_private_key`/`get_raw_public_key` (Ed25519) and EC scalar/point getters for the §A.7 raw encodings; `EVP_DigestSign`/`Verify` for Ed25519 (no prehash) and ECDSA (SHA-256/384/512).
- [ ] macOS NIST (Security.framework): `SecKeyCreateRandomKey` (`kSecAttrKeyTypeECSECPrimeRandom`, 256/384/521), `SecKeyCopyExternalRepresentation` (normalize Apple's `04‖X‖Y‖K` to scalar + `04‖X‖Y`), `SecKeyCreateWithData`, `SecKeyCreateSignature`/`SecKeyVerifySignature` (`ECDSASignatureMessageX962SHA*`).
- [ ] macOS Ed25519: route to the plan-04-B software core.
- [ ] `crypto_package.mfb`: `generate*`/`*Sign`/`*Verify` validate key lengths (`ErrInvalidArgument`) and dispatch per target.

Acceptance: keygen/sign/verify for Ed25519 and ECDSA (P-256/384/521) work on both platforms and cross-verify (a macOS-signed message verifies on Linux and vice versa).
Commit: —
