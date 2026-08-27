# plan-109-D: RFC 8032 Ed448 generation, signing, and verification

Last updated: 2026-08-27
Effort: large (3h–1d)
Depends on: plan-109-C

This letter adds `Certificate.Ed448` as portable PureEdDSA using B's SHAKE256 and
C's Curve448 field foundation where representation can safely be shared.

References: `planning/plan-109-C-x448-conversion.md`; RFC 8032 §§5.2 and 7.4;
existing Ed25519 helpers and `func_generate.rs`, `func_sign.rs`, `func_verify.rs`.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-109-C complete | `find planning -maxdepth 1 -name 'plan-109-C-*' \| wc -l` → `0` | NOT MET |
| X448/conversion KAT green | filtered C fixture | re-run |

## 1. Goal

- `generate(Certificate.Ed448)` returns a 57-byte RFC 8032 seed and 57-byte public
  key; `sign` returns deterministic 114-byte PureEd448 signatures; `verify`
  accepts valid RFC vectors and rejects malformed, non-canonical, wrong-key, and
  altered-message/signature inputs without secret-dependent control flow.

### Non-goals

- No Ed448ph or context API: the existing `sign(Certificate, key, message)`
  surface maps to PureEd448 with empty context.
- No platform-library Ed448 path and no changes to NIST-EC DER encodings.

## 2. Current State

Ed25519 is pure-MFB with 32-byte seed/public and 64-byte signatures. NIST curves
use platform backends. X25519/X448 are explicitly rejected by sign/verify.
RFC 8032 states Ed448 private/public keys are 57 bytes and signatures 114 bytes.

### Measured populations

| What | Count | Command |
|---|---:|---|
| current public-key operations sharing `Certificate` | 3 | `rg -l 'Certificate.*AbiFunction' src/codegen/builtins/crypto/func_{generate,sign,verify}.rs \| wc -l` |
| dedicated Ed25519-invalid fixture directories | 1 | `find tests/rt-behavior/crypto -maxdepth 1 -type d -name '*ed25519*' \| wc -l` |

### Verified properties

- Existing Ed25519 helpers cannot be parameter-switched to Ed448: field, base
  point, encoding, hash, scalar size, dom4, and signature sizes differ — verified
  against RFC 8032 and current helper bodies.
- Generated software-key branches exist separately in each platform-family body;
  ordinal wiring must remain symmetric.

## 3. Design Overview

Implement RFC 8032 §5.2 exactly: SHAKE256 seed expansion, pruning, Edwards448
point arithmetic/encoding/decoding, dom4 with empty context and prehash flag 0,
deterministic nonce/challenge reduction modulo the group order, and canonical
verification equation. Use complete formulas and fixed-iteration scalar
multiplication. Reuse C's field operations only where the Edwards/Montgomery
representation contract is explicitly compatible; do not force abstraction at
the expense of reviewability.

Append Ed448 after X448 so earlier ordinals stay fixed. Replace the prior
“reject all ordinals after Ed25519” shape with explicit signable-vs-key-agreement
classification, tested for both X variants.

Byte identity is not the gate; crypto codegen is expected to grow on all targets.
Correctness risk concentrates in point decoding/canonicality and scalar reduction;
schedule malformed-vector tests before dispatch integration.

## Compatibility / Format Impact

`Certificate.Ed448` keys use 57-byte raw RFC 8032 encodings; signatures are 114
bytes. Existing Ed25519 and NIST key/signature formats remain byte-for-byte.

## Phases

### Phase 1 — Ed448 primitive KAT core

- [ ] Add field/scalar/point helpers with fixed-time secret operations and strict
      canonical point/scalar decoding.
- [ ] Implement public-key derivation, PureEd448 sign, and verify helpers.
- [ ] Add all applicable RFC 8032 §7.4 vectors and adversarial canonicality,
      torsion/low-order, S≥L, truncated/extended input tests.

Acceptance: every RFC vector matches exactly and every malformed vector rejects;
structural constant-time review passes.
Commit: —

### Phase 2 — Certificate integration

- [ ] Append Ed448 and wire generate/sign/verify on macOS, Linux, and Windows
      software branches; preserve explicit X25519/X448 rejection.
- [ ] Update function descriptors, type docs, KAT/invalid fixtures, acceptance
      coverage, and stdlib spec.

Acceptance: generated Ed448 round-trip works for empty, short, and multi-block
messages; deterministic signatures repeat exactly; RFC vectors interoperate.
Commit: —

## Validation Plan

Runtime proof must assert exact key/signature bytes, not only Boolean verify.
Exercise wrong length, non-canonical point/scalar, altered message, wrong key, and
X-curve rejection. Run full `cargo test`, release acceptance, all-target artifact
gate with justified regeneration, supported-target execution, doc renders, and
the required root/repository rustfmt passes.

## Open Decisions

- Shared Curve448 arithmetic modules — recommend sharing field encode/reduce
  primitives only after direct body comparison; keep Edwards point and Montgomery
  ladder layers separate.

## Corrections

None yet.

## Summary

Ed448 lands only after SHAKE and Curve448 are proven. Strict decoding and RFC
vectors, not self-round-trip alone, are the acceptance oracle.
