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
| plan-109-A is archived/complete | `find planning -maxdepth 1 -name 'plan-109-A-*' \| wc -l` → `0` | MET 2026-08-29 (archived to `planning/completed/` after its full ledger: gate 0 diffs, full acceptance 1279 ran, `cargo test` green) |
| renamed SHA-2 + SHA-1 KAT is green | `scripts/test-accept.sh target/release/mfb /tmp/plan109-b-pre 'rt-behavior/crypto/crypto-kat-valid'` | MET 2026-08-29 (green in A's filtered and full runs; the Keccak work below was prototyped against a debug build while A's final acceptance drained the shared harness lock, and only committed after A archived) |

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

- [x] Add limb XOR/rotate helpers and Keccak-f[1600] state/round helpers as
      individually registered `helper_*.rs` modules — one `Integer` per lane
      (see Corrections), so no limb helpers: `helper_keccak_{rc,rc_table,rho,
      rho_table,pi,pi_table,zero,round,f}.rs`, `helper_le_lane.rs`,
      `helper_append_le_lane.rs`.
- [x] Add absorb/pad/squeeze helpers parameterized by public rate, suffix, and
      output length; add private `__crypto_shake256`
      (`helper_keccak_sponge.rs`; `helper_shake256.rs` + the public
      `func_shake256.rs` surface and `helper_shake256_text.rs` shim — see
      Corrections).
- [x] Test NIST permutation and SHAKE256 vectors, multi-block absorb, multi-block
      squeeze, empty input, and padding at rate−1/rate/rate+1
      (`tests/rt-behavior/crypto/crypto-sha3-kat-valid`, 46 oracle lines: every
      SHA-3 width at empty/`abc`/200×0xa3/rate±1 through both `hash` overloads;
      SHAKE256 empty-32, abc-64, abc-300, 135→136, 136→137, 137→1, a3→272,
      prefix property; `crypto-kdf-invalid` pins `shake256` length 0/−5 →
      `ErrInvalidArgument`, 1 ok).

Acceptance: NIST intermediate/permutation and SHAKE256 outputs match exactly;
structural audit finds no secret-dependent branch/index.
VERIFIED 2026-08-29: fixture output `diff`s clean against the hashlib oracle
(`ALL_MATCH`); `keccak_core_is_branch_free` (crypto `mod.rs` tests) asserts the
round/permutation/lane helpers contain no `IF`/`EXIT`/`TRAP`, the sponge's only
conditional is `len(out) < outLen`, and no state list is indexed by another
state list's contents (only `__CRYPTO_KECCAK_RHO`/`PI` by the loop counter).
Commit: 54cc2878c

### Phase 2 — public SHA-3 dispatch

- [x] Append four `Hash` variants and extend hash/digest/block/output dispatch
      (`SHA3_224..512` = ordinals 5–8; `gen_hash::ORD_SHA3_*` arms to
      `#crypto_sha3_<w>_bytes`; `__crypto_sha{Digest,BlockSize,OutputLen}` arms —
      HMAC block = sponge rate 144/136/104/72 per FIPS 202 §7).
- [x] Add both overloads and HMAC/HKDF/PBKDF2 coverage for each SHA-3 selector
      (`crypto-sha3-kat-valid`: `hash` String + bytes per width; HMAC per width
      incl. >block-size keys; HKDF per width incl. empty salt/info; PBKDF2 per
      width; `crypto-kdf-invalid` HKDF-SHA3_256 8160/8161 + PBKDF2-SHA3_512
      iterations 0).
- [x] Update man/spec algorithm and constant-time claims with source citations
      (`hash`/`hmac`/`hkdf`/`pbkdf2`/`shake256` descriptors, `MODULE_DESC`,
      `10_crypto.md` algorithm set + XOF row + numeric-representation paragraph
      citing `helper_keccak_round.rs:BODY`/`helper_keccak_sponge.rs:BODY`).

Acceptance: NIST SHA-3 KATs for all four widths match on empty, `abc`, and
multi-block messages; all hash-selected APIs produce correct independent oracle
outputs.
VERIFIED 2026-08-29: same fixture `diff` (the 200×0xa3 message is NIST's
1600-bit SHA-3 example; `hashlib` reproduces the published digests
`9376816a…`/`79f38ade…`/`1881de2c…`/`e76dfad2…`).
Commit: 54cc2878c

## Validation Plan

Run the release runtime KAT, differential outputs against NIST examples, full
`cargo test`, filtered acceptance, then the full artifact gate after regenerating
only proven importer/helper drift. Run the mandated two rustfmt commands. Render
`mfb man crypto --all` and `mfb spec stdlib crypto` with no leaked citations.

## Validation ledger (2026-08-29)

- rustfmt root + `repository/`: run. Fresh release build; `cargo test --bin mfb`
  3818 passed at B's commit.
- Full `cargo test --no-fail-fast` + all-target `artifact-gate` ran once after
  D (`54cc2878c`..`ce772e7a1` inclusive): `CARGO_EXIT=0`, 64 targets ok,
  **1265 tests, 1412 builds, 1744 goldens checked, 0 diffs**.
- Filtered release acceptance on every crypto importer: 15 ran, green.
- `mfb man crypto shake256` / `mfb spec stdlib crypto`: no leaked citations.

## Open Decisions

- Internal state container — recommend a flat `List OF Integer` of 50 limbs to
  match existing package collection operations; reject 25 record values because
  repeated record rebuilds amplify allocation churn. DECIDED: a flat
  `List OF Integer` of 25 full 64-bit lanes (see Corrections).

## Corrections

- The "Verified property" that a trapping `Integer` cannot carry a 64-bit
  Keccak lane is FALSE, and so is the premise that SHA-512 avoids it with limbs:
  `bits::` treats every `Integer` as a raw two's-complement 64-bit pattern
  (`bits::rl64`, `bxor`, `band`, `bnot`, `sl`, `sr` are total on it), and SHA-512
  already keeps each word in ONE `Integer` (`helper_be_word64.rs`,
  `helper_add64.rs` — only the *addition* is limb-split, to dodge the trapping
  `+`). Probe (`/tmp/p109-lane`, release binary): a lane built from LE bytes with
  bit 63 set (`0x8000000080008008`) round-trips, `rl64` by 1/32/63 and
  XOR/AND-NOT give the expected patterns, and `bits::sl(1, 63)` is simply a
  negative `Integer`, no trap. Keccak has no addition, so the state is a flat
  `List OF Integer` of **25** lanes (not 50 limbs) and rho is a single `rl64` —
  the Open Decision's 50-limb container is superseded.
- Private `__crypto_*` helpers are unreachable from user source
  (`SYMBOL_UNKNOWN_IDENTIFIER: Callable __crypto_sha1_bytes is not a top-level
  function`, `/tmp/p109-priv`), so a "private SHAKE256" has no runtime proof
  path. SHAKE256 therefore lands as the public member `crypto::shake256(data,
  length)` (`List OF Byte` and `String` overloads, `ErrInvalidArgument` below 1)
  — a variable-length XOF is its own member, which the non-goal ("not a public
  `Hash` variant") explicitly leaves open. Ed448 (D) still consumes the same
  `__crypto_shake256` core.
- Raw Keccak-f[1600] zero-state intermediates are likewise unreachable from
  user source; the permutation is proven through SHAKE256/SHA-3 outputs from an
  independent oracle (`/tmp/p109-keccak-oracle.py`: a from-scratch Python
  Keccak-f + sponge whose self-check equals Python `hashlib` on 16 messages × 4
  widths and 6 SHAKE lengths before it is trusted; the KAT expectations come
  from `hashlib` itself). The multi-block squeezes (300 bytes from `abc`, 272
  from the 200-byte message) exercise repeated permutations on evolving states;
  the rate−1/rate/rate+1 messages at every rate exercise pad10*1 in both the
  message-then-pad and padding-only-block shapes.
- Populations at execution: 187 crypto modules / 165 `helper_*.rs` before this
  letter (`find src/codegen/builtins/crypto -maxdepth 1 -type f | wc -l` after
  A's six SHA-1 helpers landed; the plan's 181/159 predates A). The letter adds
  22 helpers + `func_shake256.rs` → 210 / 187.

## Summary

The correctness risk is lane rotation/padding; KAT boundaries isolate it before
public dispatch. No Curve448 or HPKE work begins here.
