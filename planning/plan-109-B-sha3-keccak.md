# plan-109-B: Constant-time Keccak-f[1600], SHA-3, and SHAKE256

Last updated: 2026-08-27
Effort: large (3h–1d)
Depends on: plan-109-A

This letter adds the portable FIPS 202 sponge foundation, exposes the four
requested SHA-3 hashes, and provides private SHAKE256 needed by Ed448 in D.

References: `planning/plan-109-A-hash-api-sha1-warning.md`; NIST FIPS 202;
`src/codegen/builtins/crypto/helper_add64.rs` and SHA-512 helpers (the existing
two-limb 64-bit arithmetic precedent); `src/docs/spec/stdlib/10_crypto.md`.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-109-A is archived/complete | `find planning -maxdepth 1 -name 'plan-109-A-*' \| wc -l` → `0` | NOT MET at authoring |
| renamed SHA-2 + SHA-1 KAT is green | `scripts/test-accept.sh target/release/mfb /tmp/plan109-b-pre 'rt-behavior/crypto/crypto-kat-valid'` | re-run |

This letter must not start until A is complete, full stop.

## 1. Goal

- `Hash.SHA3_224`, `SHA3_256`, `SHA3_384`, and `SHA3_512` produce FIPS 202
  digests through every hash-selected API, using a data-independent
  Keccak-f[1600] software permutation, and private SHAKE256 produces arbitrary
  output for Ed448.

### Non-goals

- Do not expose SHAKE as a public `Hash` variant: its variable output contract
  does not fit the fixed-digest selector requested here.
- No platform crypto calls, secret-indexed tables, data-dependent branches, or
  64-bit values that can cross MFBASIC's signed 63-bit arithmetic boundary.

## 2. Current State

The package has no Keccak/SHA-3/SHAKE helpers (`rg -n 'Keccak|SHA3|SHAKE'
src/codegen/builtins/crypto` → 0 at authoring). SHA-512 already represents 64-bit
words as safe limbs and is the arithmetic model to reuse.

### Measured populations

| What | Count | Command |
|---|---:|---|
| crypto source modules before this work | 181 | `find src/codegen/builtins/crypto -maxdepth 1 -type f \| wc -l` |
| helper modules before this work | 159 | `find src/codegen/builtins/crypto -maxdepth 1 -name 'helper_*.rs' \| wc -l` |
| existing Keccak/SHA-3/SHAKE references | 0 | `rg -n 'Keccak|SHA3|SHAKE' src/codegen/builtins/crypto \| wc -l` |

### Verified properties

- MFBASIC trapping `Integer` cannot directly carry arbitrary unsigned 64-bit
  Keccak lanes; existing SHA-512 limb helpers avoid that — verified by reading
  `helper_add64.rs`, rotation helpers, and `.ai/codegen-invariants.md`.
- FIPS 202 defines SHA3-224/256/384/512 and SHAKE256 over Keccak-f[1600]; use
  NIST vectors as the independent oracle.

## 3. Design Overview

Represent each of 25 lanes as two masked 32-bit limbs in a flat 50-word state.
Implement the 24 fixed rounds (theta, rho, pi, chi, iota) with fixed-bound loops,
constant rotation offsets, constant round constants, and indices derived only
from public loop counters. Absorb/squeeze with public message/output lengths;
use domain suffix `0x06` for SHA-3 and `0x1f` for SHAKE, plus the final `0x80` bit.
Rates are 144/136/104/72 bytes for SHA3-224/256/384/512 and 136 for SHAKE256.

“Constant-time” here means no control flow or memory address depends on message,
state, key, or digest contents; runtime necessarily depends on public input and
requested-output lengths. Add a structural source test for forbidden
secret-indexed lookup/data-dependent exits plus differential KATs.

Behavior and injected package bytes intentionally change; byte identity is not
the gate. Expected `.ncode` drift is crypto and every importer embedding the new
always-registered helpers. Unexpected targets must be localized before regen.

## Compatibility / Format Impact

Four enum variants append after A's five variants, preserving A's ordinals.
Existing digest bytes remain unchanged; new variants add fixed digest outputs.

## Phases

### Phase 1 — permutation and sponge

- [ ] Add limb XOR/rotate helpers and Keccak-f[1600] state/round helpers as
      individually registered `helper_*.rs` modules.
- [ ] Add absorb/pad/squeeze helpers parameterized by public rate, suffix, and
      output length; add private `__crypto_shake256`.
- [ ] Test NIST permutation and SHAKE256 vectors, multi-block absorb, multi-block
      squeeze, empty input, and padding at rate−1/rate/rate+1.

Acceptance: NIST intermediate/permutation and SHAKE256 outputs match exactly;
structural audit finds no secret-dependent branch/index.
Commit: —

### Phase 2 — public SHA-3 dispatch

- [ ] Append four `Hash` variants and extend hash/digest/block/output dispatch.
- [ ] Add both overloads and HMAC/HKDF/PBKDF2 coverage for each SHA-3 selector.
- [ ] Update man/spec algorithm and constant-time claims with source citations.

Acceptance: NIST SHA-3 KATs for all four widths match on empty, `abc`, and
multi-block messages; all hash-selected APIs produce correct independent oracle
outputs.
Commit: —

## Validation Plan

Run the release runtime KAT, differential outputs against NIST examples, full
`cargo test`, filtered acceptance, then the full artifact gate after regenerating
only proven importer/helper drift. Run the mandated two rustfmt commands. Render
`mfb man crypto --all` and `mfb spec stdlib crypto` with no leaked citations.

## Open Decisions

- Internal state container — recommend a flat `List OF Integer` of 50 limbs to
  match existing package collection operations; reject 25 record values because
  repeated record rebuilds amplify allocation churn.

## Corrections

None yet.

## Summary

The correctness risk is lane rotation/padding; KAT boundaries isolate it before
public dispatch. No Curve448 or HPKE work begins here.
