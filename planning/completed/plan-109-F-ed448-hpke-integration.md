# plan-109-F: X448 HPKE suites, full crypto integration, docs, and goldens

Last updated: 2026-08-29
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
Commit: 70e479f3f

### Phase 2 — whole-plan surface and documentation closeout

- [x] Sweep every remaining old SHA spelling and stale “sealed box/not HPKE” or
      Ed25519-only claim across source, man descriptors, spec, tests, and benchmark.
      VERIFIED 2026-08-29:
      `rg -n 'Hash\.SHA(224|256|384|512)\b' src tests benchmark examples packages --glob '!**/golden/**'`
      → only the 4 lines of `tests/syntax/crypto/hash-removed-spellings-invalid`
      (the negative fixture that proves the spellings are gone);
      `rg -n -i 'sealed box|mfb-box-v1|X25519-only|Ed25519-only|only Ed25519' src tests benchmark examples packages --glob '!**/golden/**'`
      → every remaining hit describes the *rejected* legacy format or the
      "a sealed box is anonymous" security note; the stale ones — the acceptance
      TGROUP title "asymmetric sealed box", its flow comment, `10_crypto.md`'s
      "only Ed25519 signing is [deterministic]", the `X25519`/`X448` Certificate
      descriptions, `func_generate`'s "take Ed25519 keys", and the
      `helper_encrypt`/`helper_decrypt` doc/body comments — were rewritten for
      Ed25519+Ed448 / RFC 9180. `mfb man crypto --all`, `mfb spec stdlib crypto`
      render with 0 `[[` and list `shake256`, `exchange`, the `Ed448_*` suites,
      and the `SHA1` advisory paragraph.
- [x] Update acceptance/byte-identity crypto cover fixtures to exercise all nine
      hashes, seven certificates, two conversions, and four HPKE profiles; prove
      every modified overload has valid and invalid coverage.
      VERIFIED 2026-08-29: `tests/acceptance/src/crypto.mfb` (580 → 846 lines)
      — census by
      `python3 -c` over the source: enum variants used = all 9 `Hash.*`, all 7
      `Certificate.*`, both `KeyConvert.*`, all 4 `AsymmetricCipher.*`; per
      member calls/`expectTrap`: hash 32/0 (its only invalid inputs are the
      removed spellings — a syntax fixture — and the SHA1 advisory, a warning),
      shake256 9/2, hmac 17/0 (no error path exists: any key/data length is
      valid), hkdf 13/3 (incl. the SHA-1 `255·20` ceiling), pbkdf2 12/2,
      generate 54/0 (no error path), sign 24/4, verify 26/3 (+ FALSE verdicts),
      convert 8/2 (wrong-curve pairs both ways), exchange 19/6, encrypt 21/2,
      decrypt 28/18. New KATs: SHA-1/SHA-3/SHAKE256 (FIPS), HMAC/HKDF/PBKDF2
      with SHA-1 (RFC 5869 A.4, RFC 6070) and SHA-3 (OpenSSL reference), Ed448
      RFC 8032 §7.4 test-1 + an OpenSSL-computed "abc" signature, X25519/X448
      RFC 7748 §6.1/§6.2 both directions.
      `target/release/mfb test tests/acceptance` → `Tests: 732  Pass: 732  Fail: 0`.
      `tests/byte-identity/crypto/src/main.mfb` (93 → 141 lines) now calls every
      overload of every member incl. SHA-1/SHA-3/SHAKE256, Ed448 sign/verify,
      X25519/X448 exchange, both conversions, and all four HPKE profiles
      (bytes/String × aad/no-aad); executed once by hand (all lengths/verdicts
      as expected) and compile-only in the gate as before.
      Found while writing the invalid cases: `convert(KeyConvert.Ed25519ToX25519,
      <57-byte Ed448 pair>)` silently mis-mapped (no length check, unlike the
      Ed448 arm) — fixed in `helper_convert.rs` (32-byte check →
      `ErrInvalidArgument`), documented in `func_convert.rs`/`10_crypto.md`,
      covered by the new `expectTrap`.
- [x] Regenerate only expected AST/IR/ncode/ncodesum drift after normalized diffs;
      inspect one fixture per distinct drift class before accepting.
      VERIFIED 2026-08-29: two drift classes, both expected. (1) `.ir` of every
      crypto importer (19 fixtures via `sync-goldens.sh`): inspected
      `crypto-aead-invalid.ir` — uniform `line` shifts of the embedded package
      (+2 for the two appended `AsymmetricCipher` variants, +3 for the convert
      length check), the two new enum variants, and the new/rewritten
      `__crypto_hpke*`/`__crypto_convert` bodies; no user-code IR changed.
      (2) `.ncodesum` of `byte-identity/crypto` (5 targets, `regen-ncodesum.sh`)
      and `crypto-ec-valid` (4 targets, hand regen) — the package body changed,
      so every crypto target's bytes change, as the plan predicted ("all crypto
      targets are expected to change"); no non-crypto golden moved (`git status`
      after regen: only `tests/**/crypto/**` + `bug96_audit_tls_http_crypto`),
      and the standalone artifact gate then reports 0 diffs across 1748 goldens.
- [x] Run the complete validation ledger and archive each plan-109 letter only
      after its own acceptance is recorded. VERIFIED 2026-08-29: ledger below,
      measured on the merged tree; A–D were archived at `b48ad824b`/`84db74365`
      with their own ledgers, E at this letter's closeout (its ledger was filled
      at `dfff39d14`'s predecessor commit `70e479f3f`), F itself with this
      commit. Remote runtime matrix (`.ai/remote_systems.md` boxes; `/tmp/p109-remote.sh`
      cross-builds the eight crypto exact-vector fixtures with `-target`, scp's
      them, runs them, and diffs stdout against the macOS golden output):
      linux-x86_64 glibc (:2228) 8/8, linux-x86_64 musl (:2227) 8/8,
      linux-aarch64 glibc (:2223) 8/8, linux-riscv64 musl (:2229) 8/8,
      windows-x86_64 (:2230) 8/8 — 40/40 byte-identical (SHA-1/SHA-2/SHA-3/SHAKE
      KATs, X448, Ed448, both HPKE profiles' oracle boxes, the SHA-1 advisory
      program, and the platform-ECDSA `crypto-ec-valid`). Boxes :2222, :2224,
      :2226, :2231, :2232 refused the connection (`ssh: connect … Connection
      refused`) and were skipped; every supported target/libc pair is still
      covered by at least one box.

Acceptance: no old API/stale-format claims remain (`rg` commands return 0), every
requested behavior has runtime or diagnostic proof, full suite/gates pass, and
rendered man/spec accurately state algorithms and wire sizes. VERIFIED (boxes
above + ledger).
Commit: dfff39d14 (fixtures, sweep, convert fix) + 12b41236c (main merge) + the
closeout commit that archives this file (see `git log -- planning/completed/plan-109-F-*`).

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

### Validation ledger (2026-08-29, whole plan, on the merged tree `12b41236c`+)

| Check | Command | Result |
|---|---|---|
| Interop, four profiles both ways + RFC pins | `cargo test --release --test rt_crypto_hpke_interop` | 5 passed (A.1, A.6.1, RFC 7748 X448, FIPS 202 SHAKE256, both-ways ×4) |
| Oracle vs official X448 vectors | `python3 /tmp/p109-a6-check.py` | `ALL_INTERMEDIATES_MATCH` for AES-128-GCM, AES-256-GCM, ChaCha20Poly1305 |
| X448 fixture vs oracle | `crypto-hpke-x448-valid` release output `diff /tmp/p109-hpke448-expected.txt` | `ALL_MATCH` (26 lines) |
| Unit tests | `cargo test --bin mfb` (pre-merge) | 3820 passed |
| Acceptance TESTING app | `target/release/mfb test tests/acceptance` (pre-merge) | `Tests: 732  Pass: 732  Fail: 0` |
| Filtered fixtures | `scripts/test-accept.sh target/release/mfb /tmp/accept-out-p109 'rt-behavior/crypto/*' 'byte-identity/crypto' 'syntax/crypto/*' 'rt-error/crypto/*' 'syntax/security/bug96_audit_tls_http_crypto'` | 19 ran, passed |
| Man/spec renders | `mfb man crypto --all`, `mfb spec stdlib crypto` | 0 `[[`; SHA1 advisory, shake256, exchange, Ed448_* suites present |
| Full Rust suite (merged tree) | `cargo test --no-fail-fast` | every target ok (bin 3820 passed, mfb_repository 318 + 21, all integration targets) except `golden::artifact_gate_all`, which failed in 0.20s on the peer-held gate lock ("Another artifact-gate (pid 7499) is running"); re-run standalone below |
| All-target artifact gate | `cargo test --test golden` (= `artifact-gate.sh target/release/mfb all`, standalone) | `1267 tests, 1414 build(s), 1748 golden(s) checked, 0 diff(s)` — ok in 203.8s |
| Full acceptance harness | `scripts/test-accept.sh target/release/mfb /tmp/accept-out-p109` | `acceptance tests passed (1283 test(s) ran)` |
| Acceptance TESTING app (merged tree) | `target/release/mfb test tests/acceptance` | `Tests: 732  Pass: 732  Fail: 0` |
| Remote runtime matrix | `bash /tmp/p109-remote.sh <port> <target> <suffix>` ×5 boxes | 40/40 PASS (see the box-4 note) |
| rustfmt pair | `cargo fmt --all` + `cargo fmt --all --manifest-path repository/Cargo.toml` | clean (no churn) |

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
- **Two of the three first-run failures of the extended acceptance suite were
  my expectations, one was a real fail-open.** (1) `verify(Ed448, 56-byte key,
  …)` returns `FALSE`, not `ErrInvalidArgument` — that is the documented contract
  shared with Ed25519 (`func_verify.rs` DESC: "a wrong-length key or signature is
  simply FALSE"; only the NIST curves raise); the test now asserts `FALSE`, and the
  file's header comment, which stated the NIST rule for all curves, was
  corrected. (2) A 58-byte X25519 box under an `Ed448_*` suite is below the
  72-byte overhead → `ErrInvalidArgument`; the test now covers both a short (58)
  and a long (77-byte) cross-curve box (`ErrAuthenticationFailed`). (3)
  `convert(KeyConvert.Ed25519ToX25519, <Ed448 pair>)` returned a KeyPair: the
  Ed25519 arm had no length check while the Ed448 arm did, and
  `func_convert`'s "beyond the length check" wording implied one existed for
  both. Fixed (32-byte check → `ErrInvalidArgument`), documented, and covered.
- **`main` advanced during the plan** (fork `2299b6326` → `5f17afd7c`, "retire
  the last package.mfb companions — registry-modeled enums" for `term`/`strings`).
  Merged into `worktree-P-109` at `12b41236c`: no textual conflicts (main only
  deleted `.mfb` companions this branch never touched), but its 13 new
  `EnumVariant` literals in `term/mod.rs` predate plan-109-A's `advisory` field
  and needed `advisory: None` to compile. The full validation ledger below is
  measured on the merged tree.
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
