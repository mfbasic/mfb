# plan-04-B: crypto — macOS software cores (the C-ABI gaps)

Last updated: 2026-06-28
Effort: large

Part **B** of plan-04 (Built-in `crypto` Package). Portable, constant-time implementations of the
primitives macOS's C ABI does not expose — ChaCha20-Poly1305 and Ed25519 — written on `bits` (+ the
`UMULH` encoder), used on macOS and as the BR/no-library fallback (**Linux uses OpenSSL for both**).
Shared design lives in the overview: [plan-04-crypto.md](plan-04-crypto.md).

- **Depends on:** plan-04-A (reuses `crypto::sha512` + `bits`).
- **Blocks:** plan-04-C (macOS Ed25519 signing routes here), plan-04-D (tests).
- **Spec/design:** overview §A.5 (AEAD), §A.7 (Ed25519), Part E.3 (x86_64 note).

## Phases

### Phase B1 — Software ChaCha20-Poly1305 + Ed25519

- [ ] ChaCha20-Poly1305 (RFC 8439): ChaCha20 as add-rotate-xor on `bits`; Poly1305 as a 130-bit modular multiply reusing `UMULH`.
- [ ] Ed25519 (RFC 8032): Curve25519 field/scalar arithmetic on `bits` + `UMULH`, reusing `crypto::sha512`; deterministic signing; seed from `randomBytes(32)`.

Acceptance: the macOS software ChaCha20-Poly1305 and Ed25519 cores match RFC 8439 / RFC 8032 vectors and are byte-identical to the Linux OpenSSL outputs.
Commit: —
