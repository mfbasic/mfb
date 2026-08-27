# plan-109-C: X448 and Ed448-to-X448 key conversion

Last updated: 2026-08-27
Effort: large (3h–1d)
Depends on: plan-109-B

This letter adds RFC 7748 X448 generation/scalar multiplication and the requested
`KeyConvert.Ed448ToX448`, providing the KEM foundation for F.

References: `planning/plan-109-B-sha3-keccak.md`; RFC 7748 §§5–6 and vectors;
`helper_x25519.rs`, `helper_generate_x25519.rs`, `helper_convert.rs`; RFC 8032
Ed448 key derivation; a pinned, independently maintained Curve448 conversion
oracle selected in Phase 1.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-109-B complete | `find planning -maxdepth 1 -name 'plan-109-B-*' \| wc -l` → `0` | NOT MET |
| SHAKE256 KAT green | filtered plan-109-B runtime fixture | re-run |

## 1. Goal

- `Certificate.X448` generates 56-byte RFC 7748 key pairs; internal X448 agrees
  with RFC vectors and rejects all-zero shared secrets where used as KEM input;
  `KeyConvert.Ed448ToX448` converts both halves of a valid Ed448 key pair to the
  corresponding X448 pair with oracle-verified bytes.

### Non-goals

- X448 is key agreement, so `sign`/`verify` must reject `Certificate.X448` just
  as they reject X25519.
- No claim that RFC 8032 or RFC 7748 standardizes a generic Ed448↔X448 conversion;
  document the exact chosen mapping and interoperability oracle.

## 2. Current State

`Certificate` ends with `X25519` ordinal 4; X25519 generation and Montgomery
ladder are pure MFB helpers. `KeyConvert` has one ordinal and converts Ed25519
seed/public bytes to X25519. There is no Curve448 arithmetic.

### Measured populations

| What | Count | Command |
|---|---:|---|
| non-golden files referencing Ed25519/X25519 certificate or asymmetric variants | 17 | `rg -l 'Certificate\.(Ed25519|X25519)|AsymmetricCipher\.Ed25519' --glob '!**/golden/**' . \| wc -l` |
| current Certificate variants | 5 | inspect `src/codegen/builtins/crypto/mod.rs:package`; verify with registry test |
| current KeyConvert variants | 1 | same command/read at `KeyConvert` declaration |

### Verified properties

- RFC 7748 X448 inputs/outputs are 56 bytes and the base u-coordinate is 5.
- Existing `generate` has three target-family ordinal branches; new software
  ordinals must be handled in macOS/Linux/Windows arms, while sign/verify must
  explicitly reject the key-agreement variant.

## 3. Design Overview

Use the RFC 7748 Montgomery ladder with fixed 448 iterations, scalar pruning,
little-endian encoding, `p = 2^448 − 2^224 − 1`, and fixed-time conditional swap.
Choose a limb representation only after a Phase-1 arithmetic spike proves all
intermediates remain under the language's signed-63-bit trap boundary. Reuse
SHAKE256 from B for the Ed448 private-seed derivation.

The public conversion uses the documented Edwards448→Montgomery448 birational
map; the private conversion uses the Ed448 secret expansion/pruning required by
the selected interoperability convention. The plan must pin an external oracle
and check converted public equals `X448(converted private, 5)`; this invariant is
stronger than merely producing 56 bytes.

Behavior and codegen intentionally change; expect crypto/importer golden drift.
Correctness risk is finite-field reduction and conversion convention. Design
uncertainty is the exact interoperability mapping, so the oracle spike lands
before production helpers.

## Compatibility / Format Impact

Append `Certificate.X448` and `KeyConvert.Ed448ToX448`; do not reorder existing
values. X448 public/private keys are 56 bytes. The Ed448 inputs added by D will be
57-byte RFC 8032 seed/public keys, and conversion outputs are 56-byte X448 keys.

## Phases

### Phase 1 — conversion oracle and arithmetic proof

- [ ] Pin the conversion convention to a primary/reference implementation;
      record formulas, pruning, byte encoding, license/provenance, and vectors.
- [ ] Prototype the limb bounds and document maximum intermediates with an
      executable checked-arithmetic test; choose representation from evidence.

Acceptance: fixed Ed448 seed/public vectors convert to oracle X448 bytes, and
every arithmetic bound test stays below `2^63`.
Commit: —

### Phase 2 — X448 and API wiring

- [ ] Add constant-time field/ladder/encode/decode helpers and X448 generation.
- [ ] Append the Certificate/KeyConvert variants; wire generate, convert, and
      explicit sign/verify rejection across all three target-family branches.
- [ ] Add RFC 7748 function, iteration, Alice/Bob ECDH, low-order/all-zero, wrong
      key-length, and conversion-consistency fixtures.
- [ ] Update descriptors/spec with exact sizes and conversion convention.

Acceptance: all RFC 7748 X448 vectors and oracle conversion vectors match;
generated and converted pairs perform identical two-party ECDH; invalid KEM
outputs fail closed.
Commit: —

## Validation Plan

Cover generated and fixed-vector keys, both convert halves, rejection paths, and
all supported native targets through compilation. Runtime-prove macOS locally and
Linux x86-64/aarch64/riscv64 per `.ai/remote_systems.md` when implementation runs;
run Win64 execution because artifact bytes alone are not correctness. Finish with
fresh release, full `cargo test`, acceptance/artifact gates, doc render, and both
rustfmt commands.

## Open Decisions

- Ed448→X448 interoperability convention — recommend the Decaf/Goldilocks
  reference mapping after Phase 1 proves exact vectors; do not invent a mapping
  from the two RFCs, which specify the curves but not this combined API.

## Corrections

None yet.

## Summary

This letter deliberately separates Curve448 arithmetic and conversion from Ed448
signatures, so the highest-uncertainty mapping is proven independently.
