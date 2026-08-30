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
| plan-109-B complete | `find planning -maxdepth 1 -name 'plan-109-B-*' \| wc -l` → `0` | MET 2026-08-29 (B committed `54cc2878c`; archived with its ledger once the post-C full suite/gate ran — the C prototype was built against a debug binary while B's ledger drained, see Corrections) |
| SHAKE256 KAT green | filtered plan-109-B runtime fixture | MET 2026-08-29 (`crypto-sha3-kat-valid` green in the 15-fixture filtered run) |

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

- [x] Pin the conversion convention to a primary/reference implementation;
      record formulas, pruning, byte encoding, license/provenance, and vectors —
      libdecaf's (`decaf_ed448_convert_{public,private}_key_to_x448`, MIT):
      public `u = y²/x²` (the RFC 7748 §4.2 edwards448→curve448 4-isogeny,
      evaluated as `y²·(1 − d·y²)/(1 − y²)`, `d = −39081`, no square root;
      only the 56-byte `y` is read), private `SHAKE256(seed)[0..56]` (the
      RFC 8032 §5.2.5 scalar bytes before pruning; RFC 8032 pruning ≡ RFC 7748
      clamp on those bytes); 57-byte LE inputs, 56-byte LE outputs. Oracle spike
      `/tmp/p109-c448-oracle.py`: a from-scratch big-int X448 and Ed448 equal
      OpenSSL (`cryptography` 49.0.0) on the base point, random inputs, and
      random seeds; the isogeny sends the edwards448 base point to `u = 5`; and
      `X448(convertedPriv, 5) == convertedPub` holds for random seeds against
      both oracles. Vectors: RFC 7748 §5.2 iteration-1/1000 outputs reproduced
      (`3f482c8a…4113`, `aa3b4749…9f38`), §6 Alice/Bob, RFC 8032 §7.4 seed 1.
- [x] Prototype the limb bounds and document maximum intermediates with an
      executable checked-arithmetic test; choose representation from evidence —
      16 × 28-bit limbs; `gf448_mul_accumulators_fit_i63` (crypto `mod.rs`
      tests) mirrors `__crypto_gf448Mul`'s fold weights with all limbs at 2^28:
      worst output limb 8 = 9 + 2·7 + 15 = 38 products ≈ 2^61.25 < 2^62;
      add/sub-bias/mulSmall(39081)/carry inputs all `< 2^63`. A Python model of
      the exact limb algorithms (`/tmp/p109-gf448-model.py`) agrees with big-int
      arithmetic on 200 random pairs for unpack/add/sub/mul/pack.

Acceptance: fixed Ed448 seed/public vectors convert to oracle X448 bytes, and
every arithmetic bound test stays below `2^63`.
VERIFIED 2026-08-29: `crypto-x448-valid` `convert-pub-{1,2}` equal the oracle
bytes and `convert-invariant-{1,2}=TRUE`; the bound test passes.
Commit: 1a2402d89

### Phase 2 — X448 and API wiring

- [x] Add constant-time field/ladder/encode/decode helpers and X448 generation
      (`helper_gf448_{zero,one,carry,add,sub,mul,mul_small,inv,select,unpack,
      pack}.rs`, `helper_clamp_scalar448.rs`, `helper_x448.rs` (branch-free
      select swap, 448 fixed iterations, a24 = 39081), `helper_x448_base.rs`,
      `helper_generate_x448.rs`).
- [x] Append the Certificate/KeyConvert variants; wire generate, convert, and
      explicit sign/verify rejection across all three target-family branches
      (`Certificate.X448` = ordinal 5 with `gen_cert::ORD_X448`; macOS/Linux/
      Windows `lower_generate` arms → `#crypto_generateX448`; `lower_sign`/
      `lower_verify` reject `ORD_X448` alongside `ORD_X25519`;
      `KeyConvert.Ed448ToX448` = ordinal 1 in `__crypto_convert`).
- [x] Add RFC 7748 function, iteration, Alice/Bob ECDH, low-order/all-zero, wrong
      key-length, and conversion-consistency fixtures
      (`tests/rt-behavior/crypto/crypto-x448-valid`, 33 oracle lines, through
      the new public `crypto::exchange` — see Corrections — plus X25519's RFC
      7748 vectors as a bonus).
- [x] Update descriptors/spec with exact sizes and conversion convention
      (`generate` table + sizes, `sign`/`verify`, `convert` (Ed448ToX448
      section), new `exchange` page, `Certificate`/`KeyConvert` variant docs,
      `10_crypto.md` key-agreement + key-conversion rows and the limb-bound
      paragraph, all with source citations).

Acceptance: all RFC 7748 X448 vectors and oracle conversion vectors match;
generated and converted pairs perform identical two-party ECDH; invalid KEM
outputs fail closed.
VERIFIED 2026-08-29: fixture `diff` clean against the oracle (`ALL_MATCH`):
iteration-1, Alice/Bob public keys and shared secret both ways, generated-pair
and converted-pair ECDH agreement, `u = 0`/`u = 1`/wrong-length/signing-cert
inputs → `ErrInvalidArgument`.
Commit: 1a2402d89

## Validation Plan

Cover generated and fixed-vector keys, both convert halves, rejection paths, and
all supported native targets through compilation. Runtime-prove macOS locally and
Linux x86-64/aarch64/riscv64 per `.ai/remote_systems.md` when implementation runs;
run Win64 execution because artifact bytes alone are not correctness. Finish with
fresh release, full `cargo test`, acceptance/artifact gates, doc render, and both
rustfmt commands.

## Validation ledger (2026-08-29)

- rustfmt root + `repository/`: run. Fresh release build; `cargo test --bin mfb`
  3819 passed at C's commit.
- Full `cargo test --no-fail-fast` + all-target `artifact-gate` ran once after
  D: `CARGO_EXIT=0`, 64 targets ok, **1265 tests, 1412 builds, 1744 goldens
  checked, 0 diffs** (cross-target codegen of every new helper compiles for
  linux-x86_64/aarch64/riscv64 and windows-x86_64 through the `.ncodesum` rows).
- Filtered release acceptance on every crypto importer: 16 ran, green.
- Remote runtime rows (Linux x86-64/aarch64/riscv64, Win64 execution): not run
  from this session — the plan-wide remote matrix is plan-F's closeout item.
- `mfb man crypto exchange` / `mfb spec stdlib crypto`: no leaked citations.

## Open Decisions

- Ed448→X448 interoperability convention — recommend the Decaf/Goldilocks
  reference mapping after Phase 1 proves exact vectors; do not invent a mapping
  from the two RFCs, which specify the curves but not this combined API.

## Corrections

- The X448 ladder has no user-reachable input surface (`generate` is random,
  `__crypto_x448` is private and private helpers are not callable from user
  source — plan-109-B Corrections), so the "RFC 7748 function, iteration,
  Alice/Bob ECDH, low-order/all-zero" fixtures this letter mandates were
  unwritable as planned. Added the public member
  `crypto::exchange(type AS Certificate, privateKey, publicKey)` — X25519/X448
  Diffie-Hellman, `ErrInvalidArgument` on a signing certificate, a wrong key
  length, or an all-zero secret (RFC 7748 §6.1) — as the ladders' public face
  (also the primitive E/F's HPKE KEM reuses). It fills a real API gap: before
  it, a generated `X25519` pair could not be used for anything.
- The oracle-first ordering paid for itself: the first MFB build produced wrong
  X448 outputs while unpack/add/sub/mul/pack matched the Python limb model
  exactly; the culprit was `__crypto_gf448Inv` skipping only bit 1 of
  `p − 2`, whereas `2^448 − 2^224 − 3` has zero bits at **both** 1 and 224
  (`python3 -c` over the exponent). Fixed; the fixture then matched on the
  first run.
- libdecaf itself is not installed here (no pip package), so its conversion
  convention is pinned by formula (documented in `helper_ed448_pub_to_x448.rs`
  / `helper_ed448_priv_to_x448.rs`) and proven by the base-point image (`u = 5`)
  and the `X448(priv', 5) == pub'` invariant against OpenSSL's X448 and Ed448 —
  a stronger check than byte-comparing against one library, since it ties both
  halves to two independent implementations of the underlying curves.
- `type` is a reserved word as an MFB parameter name (descriptor `Parameter`
  names may use it, injected bodies may not): the first `__crypto_exchange`
  failed to parse the whole injected package; renamed to `cert`.
- Population: 17 non-golden files referencing Ed25519/X25519 at authoring;
  at execution `rg -l 'Certificate\.(Ed25519|X25519)|AsymmetricCipher\.Ed25519'
  --glob '!**/golden/**' .` also matched the new B/C fixtures; the
  `Certificate` census is now 6 variants (`P256 P384 P521 Ed25519 X25519 X448`)
  and `KeyConvert` 2, so plan-F's "7 certificates" is reached by D's `Ed448`.
- Ledger note: B's full `cargo test` run was started before C's edits and
  compiled the injected package mid-edit (a `type`-parameter parse error), so
  that run is void; the post-C full suite + all-target gate stands for both
  letters.

## Summary

This letter deliberately separates Curve448 arithmetic and conversion from Ed448
signatures, so the highest-uncertainty mapping is proven independently.
