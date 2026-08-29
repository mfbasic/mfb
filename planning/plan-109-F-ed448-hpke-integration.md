# plan-109-F: X448 HPKE suites, full crypto integration, docs, and goldens

Last updated: 2026-08-27
Effort: large (3h–1d)
Depends on: plan-109-E

The final letter adds the requested Ed448-named asymmetric suites using RFC 9180
DHKEM(X448, HKDF-SHA512), then closes the entire plan with full API, runtime,
documentation, and artifact verification.

References: plans 109-A–E; RFC 9180 KEM `0x0021`, KDF `0x0003`, AEAD `0x0002`/
`0x0003`; RFC 7748 X448; RFC 8032 Ed448; `src/docs/spec/stdlib/10_crypto.md`.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-109-E complete | `find planning -maxdepth 1 -name 'plan-109-E-*' \| wc -l` → `0` | MET 2026-08-29 (E's 6 boxes ticked + acceptance verified at `8098e81f8`; the file itself is archived in this letter's Phase 2 together with F, so the literal `find` is `1` until then — see Corrections) |
| X25519 HPKE interop is green | run E's two-way interop command | MET 2026-08-29: `cargo test --release --test rt_crypto_hpke_interop` → 2 passed (A.1 vector + both-ways) at `8098e81f8` |

## 1. Goal

- `AsymmetricCipher.Ed448_AES256GCM` and
  `Ed448_CHACHA20POLY1305` accept RFC 8032 Ed448 keys, convert them to X448, use
  RFC 9180 base-mode DHKEM(X448, HKDF-SHA512) with the selected AEAD, and emit/
  consume `enc(56) || ct`; the entire plan passes full runtime, warning, spec,
  man, and all-target artifact gates.

### Non-goals

- No authenticated HPKE modes, PSK, exporter API, streaming context, public X-key
  overloads, or legacy-box fallback.
- Do not change the two X25519 suite names or existing symmetric AEAD APIs.

## 2. Current State

After E, asymmetric dispatch supports two X25519 HPKE profiles. C provides X448
and Ed448 conversion; D provides Ed448 keys. The remaining work is suite selection,
variable `Nenc`/KDF hash handling, public docs, and whole-feature verification.

### Measured populations

| What | Count | Command |
|---|---:|---|
| requested new asymmetric variants | 2 | count the two names in this plan/user request |
| final public Hash variants | 9 | registry census after B: SHA1 + four SHA2 + four SHA3 |
| final public Certificate variants | 7 | registry census after D: 3 NIST + Ed25519/X25519/X448/Ed448 |
| final public AsymmetricCipher variants | 4 | registry census after this letter |

Re-measure all three registry counts at kickoff; these expected counts derive from
prior letters and are not implementation evidence until their census commands run.

### Verified properties

- RFC 9180 assigns DHKEM(X448, HKDF-SHA512) KEM ID `0x0021`, `Nenc=56`, and
  `Nsecret=64`; the HPKE KDF is HKDF-SHA512.
- Suite selection already needs to distinguish AEAD ID/key length from KEM/KDF;
  extending the E helpers by explicit profile properties avoids ordinal arithmetic.

## 3. Design Overview

Append the two enum variants. Replace any E-era binary ordinal assumptions with
explicit profile helpers returning KEM ID, KDF hash, AEAD ID, `Nenc`, key length,
and nonce length. Ed448 profiles convert recipient keys through
`Ed448ToX448`, run X448 Encap/Decap, and feed the same generic RFC base schedule.
The wire is `56-byte enc || ct`; minimum length is 72 bytes with a 16-byte tag.

Use fixed official X448 HPKE vectors if present in the RFC JSON corpus; otherwise
generate pinned vectors with two independent conformant implementations and retain
all RFC intermediate checks. Self-roundtrip is insufficient.

Correctness risk is accidental cross-suite ordinal/default dispatch. Schedule
explicit four-profile matrices and wrong-suite rejection. Byte identity remains a
drift sentinel only; all crypto targets are expected to change.

## Compatibility / Format Impact

Two new source enum values and two new wire profiles. X448 boxes begin with a
56-byte `enc`; X25519 boxes remain 32-byte `enc`. The suite argument determines
parsing; there is no self-describing algorithm byte and no cross-suite fallback.

## Phases

### Phase 1 — X448 HPKE profiles

- [x] Generalize profile selection without ordinal arithmetic and add the two
      appended enum variants/descriptions. VERIFIED: `helper_hpke_profile.rs`
      reads every property through `__crypto_hpkeIsX448` / `__crypto_hpkeIsAesGcm`
      (equality on the selector; `rg -n 'ordinal|toInteger' src/codegen/builtins/crypto/helper_hpke_*.rs` → 0);
      `AsymmetricCipher` = 4 variants (`mod.rs`, ordinals 0–3 appended);
      `cargo test --bin mfb` → 3820 passed.
- [x] Wire Ed448→X448 conversion, DHKEM(X448, HKDF-SHA512), labeled schedule, and
      both AEADs into encrypt/decrypt. VERIFIED: `helper_hpke_kem.rs`
      (`__crypto_hpkeDh/Base/RecipientPub/RecipientPriv` dispatch to `__crypto_x448`,
      `__crypto_x448Base`, `__crypto_ed448PubToX448`/`PrivToX448` with 57-byte
      checks); KEM `0x0021`, KDF `0x0003`/`Hash.SHA2_512`, `Nenc` 56, `Nsecret` 64;
      encrypt/decrypt/generate descriptors, enum descriptions, and
      `10_crypto.md` state the four suites and the 48/72-byte overheads.
- [x] Add fixed intermediate/final vectors, two-way independent interop, minimum
      length, wrong-suite, wrong-key, corrupt enc/body/tag, and AAD tests.
      VERIFIED: `tests/rt-behavior/crypto/crypto-hpke-x448-valid` (26 lines:
      4 oracle-built boxes — RFC 7748 §6.2 Alice/Bob scalars as the fixed
      ephemeral keys, RFC 8032 §7.4 seed as the recipient — opened by MFB;
      roundtrips; `box-len-empty=72`; tamper enc/body/tag; wrong recipient;
      cross-suite; cross-curve seal/open; short-71/short-0; low-order enc; bad
      pub length) — release output `diff` against the Python oracle
      (`/tmp/p109-hpke448-expect.py`, itself validated on RFC 9180 A.1) →
      `ALL_MATCH`. `tests/rt_crypto_hpke_interop.rs` now runs all four profiles
      both ways with a from-scratch Rust X448 (checked on RFC 7748 §5.2 1/1000
      iterations + §6.2) and Keccak/SHAKE256 (FIPS 202 KATs):
      `cargo test --release --test rt_crypto_hpke_interop` → 4 passed.

Acceptance: both X448 profiles interoperate independently; all four profile
roundtrips pass; cross-profile decrypt always fails closed. VERIFIED (the
interop test's four-profile loop + the two fixtures' `cross-suite` /
`cross-curve-open` = `ErrAuthenticationFailed` lines).
Commit: —

### Phase 2 — whole-plan surface and documentation closeout

- [ ] Sweep every remaining old SHA spelling and stale “sealed box/not HPKE” or
      Ed25519-only claim across source, man descriptors, spec, tests, and benchmark.
- [ ] Update acceptance/byte-identity crypto cover fixtures to exercise all nine
      hashes, seven certificates, two conversions, and four HPKE profiles; prove
      every modified overload has valid and invalid coverage.
- [ ] Regenerate only expected AST/IR/ncode/ncodesum drift after normalized diffs;
      inspect one fixture per distinct drift class before accepting.
- [ ] Run the complete validation ledger and archive each plan-109 letter only
      after its own acceptance is recorded.

Acceptance: no old API/stale-format claims remain (`rg` commands return 0), every
requested behavior has runtime or diagnostic proof, full suite/gates pass, and
rendered man/spec accurately state algorithms and wire sizes.
Commit: —

## Validation Plan

- Tests: RFC/NIST KATs; all overloads; warnings; malformed/canonicality cases;
  four-profile HPKE interop and cross-suite failure matrix.
- Coverage check: inspect acceptance source and generated coverage so every new
  selector branch is reached; green alone proves only covered code.
- Runtime proof: execute exact-vector programs on macOS and supported Linux/Win64
  systems; compare bytes to independent implementations.
- Doc sync: registry descriptions/examples and `src/docs/spec/stdlib/10_crypto.md`;
  diagnostic spec from A; render `mfb man crypto --all`, `types`, and
  `mfb spec stdlib crypto` with citation/link tests.
- Acceptance: required rustfmt pair; fresh release binary; full `cargo test`;
  filtered new-fixture release acceptance followed by the project's full accepted
  gate discipline; `scripts/artifact-gate.sh target/release/mfb all`; supported
  remote runtime matrix. Never run cargo concurrently with artifact/acceptance.

## Open Decisions

- X448 HPKE vector provenance — prefer official RFC JSON vectors; if that corpus
  lacks the exact AES-256 profile, require agreement between two independent
  libraries and record versions/inputs/outputs in the fixture comments.

## Corrections

- **Prerequisite row 1 is self-contradictory for a closeout letter.** Its command
  (`find planning -maxdepth 1 -name 'plan-109-E-*' | wc -l` → `0`) demands E be
  archived before F starts, while F's own Phase 2 says to "archive each plan-109
  letter only after its own acceptance is recorded" and E's `Commit:` lines are
  filled in a later commit than its work. Measured instead: E's six boxes are all
  `[x]` with verification notes and its acceptance passed at `8098e81f8`
  (`rg -c '^- \[ \]' planning/plan-109-E-*.md` → 0). E is archived in Phase 2 of
  this letter, together with F.
- **The cross-curve `decrypt` verdict is `ErrAuthenticationFailed`, not
  `ErrInvalidArgument`.** My first oracle expectation for opening an
  `Ed25519_AES256GCM` box (32-byte `enc`, 77 bytes total) with
  `Ed448_AES256GCM` assumed a length rejection; the box is ≥ 72 bytes, so it
  parses, its first 56 bytes become an (arbitrary, non-low-order) X448 `enc`,
  and the AEAD tag fails. HPKE has no self-describing curve byte (the plan's own
  Compatibility section), so this is the fail-closed path the plan requires; the
  expectation was corrected, not the code. Cross-curve *seal* with a 32-byte key
  under an `Ed448_*` suite is the length rejection (`ErrInvalidArgument`).
- **Official X448 vectors exist for both AEAD profiles, but only at the oracle
  layer.** The CFRG corpus behind RFC 9180 Appendix A
  (`https://raw.githubusercontent.com/cfrg/draft-irtf-cfrg-hpke/master/test-vectors.json`,
  128 entries) has base-mode DHKEM(X448, HKDF-SHA512)/HKDF-SHA512 vectors for
  AES-128-GCM, AES-256-GCM (= RFC A.6.1) and ChaCha20Poly1305; the Python oracle
  reproduces every intermediate of all three (`python3 /tmp/p109-a6-check.py`:
  `enc`, `shared_secret`, `key`, `base_nonce`, `exporter_secret`, `ct`, `Open`
  → `ALL_INTERMEDIATES_MATCH` ×3), and the Rust harness reproduces A.6.1
  (`hpke_rust_side_reproduces_rfc9180_a6`). They cannot be replayed through MFB
  directly for the same reason as in plan-109-E: the RFC gives a raw X448 `skRm`,
  while the public `decrypt` takes an Ed448 seed and derives
  `skR = SHAKE256(seed)[0..56]`, and the deterministic `__crypto_hpkeSealWith`
  seam is private. The `crypto-hpke-x448-valid` boxes are therefore pinned from
  the RFC-validated oracle (`cryptography` 49.0.0 / OpenSSL X448 + AEAD) for an
  Ed448-seeded recipient (RFC 8032 §7.4 test-1) with RFC 7748 §6.2 scalars as
  the fixed ephemeral keys, and cross-checked both ways by the second,
  from-scratch Rust implementation (own X448 on RFC 7748 §5.2/§6.2, own SHAKE256
  on FIPS 202 KATs, `ring` HKDF/AEAD) — satisfying both halves of the Open
  Decisions row.
- **The Rust harness cannot import a raw X448/X25519 private key through
  `ring`** (its agreement API only generates ephemeral keys), so both DH
  functions in the harness are written out (X25519 via `curve25519-dalek`
  `MontgomeryPoint::mul_clamped`, X448 from scratch); no `sha3` crate is
  vendored, so SHAKE256 for the Ed448→X448 private map is a from-scratch
  Keccak-f[1600] in the harness, validated before use.

## Summary

The last risk is profile cross-wiring, controlled by explicit properties and a
four-suite matrix. Nothing adds implicit algorithm negotiation or legacy fallback;
the caller-selected suite remains the sole parser/profile contract.
