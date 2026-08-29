# plan-109-E: Replace the bespoke X25519 sealed box with RFC 9180 HPKE

Last updated: 2026-08-27
Effort: large (3h–1d)
Depends on: plan-109-D

This letter fixes the reported crypto bug: the two Ed25519-named asymmetric suites
become one-shot RFC 9180 base-mode HPKE using DHKEM(X25519, HKDF-SHA256), and the
returned wire value is exactly `enc || ct` (where HPKE `ct` includes the AEAD tag).

References: RFC 9180 §§4–7, §10, Appendix A and official JSON vectors;
`helper_encrypt.rs`, `helper_decrypt.rs`, `helper_asym_info.rs`; commit
`6ceec02ad` (introduced the bespoke construction); `func_encrypt.rs` and
`func_decrypt.rs` (currently document that it is not HPKE).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-109-D complete | `find planning -maxdepth 1 -name 'plan-109-D-*' \| wc -l` → `0` | MET 2026-08-29 (archived `84db74365` after the B+C+D full suite + gate: 0 diffs) |
| current bespoke behavior reproduced before replacement | release crypto encrypt/decrypt fixture plus fixed ephemeral test seam | MET 2026-08-29: the pre-E release binary (`ce772e7a1`) round-tripped `crypto-decrypt-short-box-invalid` and `tests/acceptance` (encrypt/decrypt group) green, and two of its `mfb-box-v1` boxes for the RFC 8032 test-1 recipient were captured (`/tmp/p109-legacy`) — they are the `legacy-*` rejection vectors of the new fixture. The bespoke ephemeral seam was never user-reachable (private helper), so "reproduced" means the captured boxes, not a seam. |

## 1. Goal

- `Ed25519_AES256GCM` and `Ed25519_CHACHA20POLY1305` use RFC 9180 base mode with
  KEM `0x0020`, KDF `0x0001`, AEAD `0x0002` or `0x0003`, empty HPKE `info`, caller
  `aad`, sequence number 0, and output `enc(32) || ct`; official vectors decrypt
  and an independent HPKE implementation decrypts MFB output (and vice versa).

### Non-goals

- No compatibility decoder for `mfb-box-v1`; accepting two unauthenticated format
  interpretations invites downgrade/ambiguity. Old ciphertext becomes invalid.
- No multi-message context, exporter, PSK, auth, or non-empty `info` API. The
  existing one-shot API is a valid fixed-empty-info base-mode profile.
- Keep Ed25519 input keys and their specified conversion to X25519; do not rename
  these two public suite variants in this plan.

## 2. Current State

The current helper derives `HKDF-SHA256(dh, salt=enc||pkR,
info="mfb-box-v1"||ordinal, 44)`, splits key/nonce, and emits
`ephemeralPublicKey || ciphertext || tag`. The decrypt helper reverses it and
requires at least 48 bytes. Both were introduced in commit `6ceec02ad`; man text
explicitly says the format is bespoke and not RFC 9180.

### Measured populations

| What | Count | Command |
|---|---:|---|
| public encrypt/decrypt overload-bearing descriptor files | 2 | `printf '%s\n' src/codegen/builtins/crypto/func_{encrypt,decrypt}.rs \| wc -l` |
| non-golden fixture/benchmark files calling encrypt/decrypt | 2 | `rg -l 'crypto::encrypt|crypto::decrypt' tests benchmark --glob '!**/golden/**' \| wc -l` |
| old-format fixed overhead | 48 bytes | read `helper_decrypt.rs`: 32-byte eph key + 16-byte tag |

### Verified properties

- RFC 9180 base setup outputs separate `enc` and AEAD `ct`; X25519 `Nenc=32`.
- HPKE requires labeled extract/expand and suite IDs at both DHKEM and full
  ciphersuite layers; ordinary HKDF with an ad-hoc salt/info is not equivalent.
- AES-256-GCM and ChaCha20Poly1305 use 32-byte keys and 12-byte nonces; AEAD IDs
  are distinct and therefore domain-separated by the HPKE suite ID.

## 3. Design Overview

Add RFC primitives `I2OSP`, `LabeledExtract`, `LabeledExpand`, DHKEM extract-and-
expand, base key schedule, and nonce computation. For encryption: convert recipient
Ed25519 public to X25519, generate ephemeral X25519, run Encap, build base-mode
context with empty info, seal once at sequence 0, return `enc || ciphertext || tag`.
For decryption: split 32-byte `enc`, convert private seed to X25519, Decap, rebuild
the same context, and open the remaining HPKE `ct`.

Introduce a deterministic internal setup seam taking fixed ephemeral IKM/key only
for KATs; the public function always uses OS randomness. Ensure the seam cannot be
resolved from user source. Test all-zero DH output and AEAD failure as fail-closed.

This is a wire-format and cryptographic behavior change, so byte identity is the
wrong correctness gate. Crypto `.ir`/`.ncode` and callers are expected to drift;
the proof is RFC vectors plus cross-implementation interop. Any unrelated fixture
diff remains a bug-hunt trigger.

## Compatibility / Format Impact

The outer byte layout still visually resembles `32-byte enc || AEAD output`, but
the derived key/nonce changes completely because RFC labeled KDF inputs replace
`mfb-box-v1`; no old ciphertext interoperates. Minimum box length remains 48 bytes
for these suites. AAD semantics remain caller-visible and authenticated.

## Phases

### Phase 1 — labeled KDF and deterministic RFC vector seam

- [x] Add encoded suite IDs and RFC labeled extract/expand helpers with length
      bounds and exact byte construction tests (`helper_hpke_i2osp2.rs`,
      `helper_hpke_labeled_extract.rs`, `helper_hpke_labeled_expand.rs`,
      `helper_hpke_profile.rs` — `"KEM" ‖ kem_id` and `"HPKE" ‖ kem ‖ kdf ‖ aead`
      by explicit property; lengths are the fixed 32/12/`Nsecret` requests, all
      under `255·HashLen`, and the byte construction is pinned end-to-end by the
      RFC A.1 reproduction below).
- [x] Implement DHKEM(X25519, HKDF-SHA256) Encap/Decap and base key schedule
      (`helper_hpke_kem.rs`: `Dh`/`Base`/`RecipientPub`/`RecipientPriv`/
      `ExtractAndExpand`; `helper_hpke_key_schedule.rs`; the deterministic
      `helper_hpke_seal_with.rs` seam that `__crypto_encrypt` feeds a fresh
      `randomBytes` ephemeral key — private, hence unreachable from user source).
- [x] Add deterministic official-vector tests for shared secret, key schedule
      context, key, base nonce, exporter secret (internal check), nonce, and ct —
      see Corrections for where each layer is checked: the RFC 9180 A.1 vector
      (every intermediate) validates both oracles (`/tmp/p109-hpke-oracle.py`
      and the Rust side of `tests/rt_crypto_hpke_interop.rs`,
      `hpke_rust_side_reproduces_rfc9180_a1`), and MFB is checked against those
      oracles through the public surface.

Acceptance: every intermediate from applicable official RFC/JSON vectors matches,
not only final ciphertext.
VERIFIED 2026-08-29 at the oracle layer (A.1: derived `skE`/`pkE`/`skR`/`pkR`,
`shared_secret`, `key_schedule_context`, `key`, `base_nonce`, `exporter_secret`,
`ct` — all byte-exact in the Python oracle; `pkE`, `ct`, and `Open` in the Rust
oracle); MFB's intermediates are exercised through the oracle-produced
`independent-*` boxes it opens (any wrong intermediate fails the AEAD open).
Commit: 8098e81f8

### Phase 2 — public one-shot replacement and interop

- [x] Replace `__crypto_encrypt`/`decrypt`, delete `mfb-box-v1`/old asym-info
      construction, and preserve overload/default-AAD behavior
      (`helper_encrypt.rs`/`helper_decrypt.rs` rewritten; `helper_asym_info.rs`
      and `helper_asym_aead.rs` deleted — `__crypto_hpkeAead` is the suite→AEAD
      map; `func_encrypt`/`func_decrypt` descriptors unchanged in shape, so the
      `String` overload and the `aad` `Fill` default are untouched).
- [x] Update minimum-length errors, man pages, stdlib spec, KAT, tamper/wrong-key/
      wrong-AAD/low-order tests, and old-format rejection fixture
      (minimum stays `Nenc + 16 = 48` → `ErrInvalidArgument`; low-order `enc`/
      recipient → `ErrInvalidArgument`; `encrypt`/`decrypt` pages and the
      `AsymmetricCipher` variants rewritten for RFC 9180; `10_crypto.md`
      public-key-encryption row; `tests/rt-behavior/crypto/crypto-hpke-x25519-valid`
      = 26 oracle lines incl. `legacy-aes`/`legacy-chacha` captured from the
      pre-E binary → `ErrAuthenticationFailed`).
- [x] Add an independent-library interop harness/test vector in both directions
      for AES-256-GCM and ChaCha20Poly1305 (`tests/rt_crypto_hpke_interop.rs`:
      an RFC 9180 implementation over `curve25519-dalek` + `ring`, itself pinned
      to RFC A.1; per suite it seals with a random ephemeral key for MFB to open,
      and opens MFB's own random-ephemeral output, plus wrong-aad/flipped-byte
      rejection on the independent side; dev-dependencies `ring`/`curve25519-dalek`
      resolve from the existing lockfile).

Acceptance: MFB decrypts independent HPKE `enc||ct`; independent HPKE decrypts MFB
output; tamper and legacy boxes fail closed with the documented errors.
VERIFIED 2026-08-29: fixture `diff` clean against the oracle (`ALL_MATCH`);
`cargo test --test rt_crypto_hpke_interop`: 2 passed (both directions, both
suites).
Commit: 8098e81f8 (both phases landed in one commit; the letter's boxes were
ticked in it, the hashes recorded here in the next commit per the skill's rule)

## Validation Plan

Use official fixed vectors for deterministic internals and an external peer for
public interop. Run both data overloads, empty/non-empty plaintext/AAD, minimum
lengths, corrupt enc/tag/body, wrong recipient, and all-zero DH. Then full
`cargo test`, release runtime acceptance, all-target artifact gate with proven
regen, supported-target execution, doc render/citation checks, and both mandated
rustfmt passes.

### Validation ledger (2026-08-29, at `8098e81f8`)

| Check | Command | Result |
|---|---|---|
| Oracle pinned to the RFC | `python3 /tmp/p109-hpke-oracle.py` (A.1 every intermediate; A.2 ct) | all asserts pass |
| Rust oracle pinned to the RFC | `cargo test --test rt_crypto_hpke_interop hpke_rust_side_reproduces_rfc9180_a1` | ok |
| MFB opens/produces interoperable boxes | `cargo test --test rt_crypto_hpke_interop` | 2 passed |
| Fixture vs oracle | `tests/rt-behavior/crypto/crypto-hpke-x25519-valid` release output `diff /tmp/p109-hpke-expected.txt` | `ALL_MATCH` (26 lines incl. `legacy-aes`/`legacy-chacha` → `ErrAuthenticationFailed`) |
| Unit tests | `cargo test --bin mfb` | all passed (registry census 20 functions, 4 `AsymmetricCipher` after F) |
| Goldens | `scripts/sync-goldens.sh target/release/mfb 'rt-behavior/crypto/*' 'byte-identity/crypto' 'rt-error/crypto/*' 'syntax/crypto/*' 'syntax/security/bug96_audit_tls_http_crypto'` + `regen-ncodesum.sh` + `/tmp/p109-regen-ec.sh` | synced; filtered `test-accept.sh` 0 mismatches |
| Whole-plan gates | `cargo test --no-fail-fast`, `artifact-gate.sh target/release/mfb all`, full `test-accept.sh`, `mfb test tests/acceptance` | recorded in plan-109-F's ledger (closeout letter) |

## Open Decisions

- Public HPKE `info` — recommend fixed empty bytes to preserve the current API;
  adding an overload is outside the request and can be a later compatible feature.
- Independent oracle — recommend a standards-conformant library supporting raw
  X25519 HPKE AES-256-GCM and ChaCha20Poly1305; pin version and vectors in the test
  provenance rather than adding it as a production dependency.

## Corrections

- "Deterministic official-vector tests for [every intermediate]" cannot be
  written against MFB directly: the deterministic seam (`__crypto_hpkeSealWith`
  taking `skE`) is private and therefore unreachable from user source (plan-109-B
  Corrections), and the RFC's own vectors give the recipient as a raw X25519
  `skRm`, which the public `decrypt` cannot take (it takes an Ed25519 seed and
  derives `skR = clamp(SHA-512(seed)[0..32])`, so no seed yields the RFC's
  `skRm`). The proof chain is therefore: (1) the RFC A.1 vector validates the two
  oracles intermediate-by-intermediate; (2) the oracles produce boxes for an
  Ed25519-seeded recipient (RFC 8032 §7.1 test-1) with fixed ephemeral keys;
  (3) MFB opens them (`mfb-opens-*`), which fails on any wrong intermediate;
  (4) the Rust oracle opens MFB's own output. A.2 (ChaCha) is covered by the same
  code path with a different AEAD id; A.1 itself uses AES-128-GCM, which the
  package does not expose, so the Rust harness carries a 16-byte-key profile
  purely to reproduce A.1.
- The independent implementation is in-tree Rust (`curve25519-dalek` X25519 +
  `ring` HKDF/AEAD), not an external HPKE library: no HPKE crate is available
  offline (only `ring` and `curve25519-dalek` are already compiled through the
  repository crate's `rustls`/`ed25519-dalek` deps), and Python `cryptography`
  (OpenSSL) has no HPKE API. Both oracles are from-the-spec implementations
  validated on the RFC's published vectors before use, which is the pinning the
  Open Decision asks for.
- RFC 9180 §7.1.4 leaves the all-zero X25519 output check to the KEM; this
  implementation fails closed with `ErrInvalidArgument` on both the sender side
  (low-order recipient key) and the recipient side (low-order `enc`), matching
  `crypto::exchange`.

## Summary

The live bug is the KDF/key-schedule contract, not merely concatenation. This
letter replaces the whole bespoke construction and proves both intermediate RFC
values and two-way interoperability.
