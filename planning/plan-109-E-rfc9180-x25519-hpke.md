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
| plan-109-D complete | `find planning -maxdepth 1 -name 'plan-109-D-*' \| wc -l` → `0` | NOT MET |
| current bespoke behavior reproduced before replacement | release crypto encrypt/decrypt fixture plus fixed ephemeral test seam | re-run |

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

- [ ] Add encoded suite IDs and RFC labeled extract/expand helpers with length
      bounds and exact byte construction tests.
- [ ] Implement DHKEM(X25519, HKDF-SHA256) Encap/Decap and base key schedule.
- [ ] Add deterministic official-vector tests for shared secret, key schedule
      context, key, base nonce, exporter secret (internal check), nonce, and ct.

Acceptance: every intermediate from applicable official RFC/JSON vectors matches,
not only final ciphertext.
Commit: —

### Phase 2 — public one-shot replacement and interop

- [ ] Replace `__crypto_encrypt`/`decrypt`, delete `mfb-box-v1`/old asym-info
      construction, and preserve overload/default-AAD behavior.
- [ ] Update minimum-length errors, man pages, stdlib spec, KAT, tamper/wrong-key/
      wrong-AAD/low-order tests, and old-format rejection fixture.
- [ ] Add an independent-library interop harness/test vector in both directions
      for AES-256-GCM and ChaCha20Poly1305.

Acceptance: MFB decrypts independent HPKE `enc||ct`; independent HPKE decrypts MFB
output; tamper and legacy boxes fail closed with the documented errors.
Commit: —

## Validation Plan

Use official fixed vectors for deterministic internals and an external peer for
public interop. Run both data overloads, empty/non-empty plaintext/AAD, minimum
lengths, corrupt enc/tag/body, wrong recipient, and all-zero DH. Then full
`cargo test`, release runtime acceptance, all-target artifact gate with proven
regen, supported-target execution, doc render/citation checks, and both mandated
rustfmt passes.

## Open Decisions

- Public HPKE `info` — recommend fixed empty bytes to preserve the current API;
  adding an overload is outside the request and can be a later compatible feature.
- Independent oracle — recommend a standards-conformant library supporting raw
  X25519 HPKE AES-256-GCM and ChaCha20Poly1305; pin version and vectors in the test
  provenance rather than adding it as a production dependency.

## Corrections

None yet.

## Summary

The live bug is the KDF/key-schedule contract, not merely concatenation. This
letter replaces the whole bespoke construction and proves both intermediate RFC
values and two-way interoperability.
