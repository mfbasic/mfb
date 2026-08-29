# plan-109-A: Hash API rename, SHA-1, and the SHA-1 compile-time warning

Last updated: 2026-08-27
Overall Effort: huge (> 3d) — plan-109 spans A–F
Effort: large (3h–1d)
Depends on: nothing (first letter)

Plan-109 updates `crypto` to the requested hash and Curve448 surface and replaces
its bespoke sealed box with RFC 9180 HPKE. This first letter makes the breaking
SHA-2 spelling change, adds SHA-1, and establishes the enum-value advisory seam
needed to warn on every source use of `crypto::Hash.SHA1` without rejecting the
program.

References:

- `src/codegen/builtins/crypto/mod.rs:package` — current `Hash` declaration.
- `src/codegen/builtins/crypto/gen_hash.rs:emit_hash_dispatch` and
  `helper_sha_digest.rs` — native/member and source-helper dispatch seams.
- `src/syntaxcheck/inference.rs:report_warning` and
  `src/docs/spec/diagnostics/02_error-codes.md` — non-fatal compiler advisories.
- FIPS 180-4 — SHA-1 and SHA-2 definitions and known-answer vectors.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| no unfinished plan-109 implementation exists | `rg -l 'SHA2_256\|SHA3_\|X448\|Ed448' src tests benchmark \| wc -l` → `0` | MET 2026-08-29 (0; the original `find -newer` command listed the sibling letters B–F because they were saved seconds after A — see Corrections) |
| existing crypto KAT passes before the rename | `cargo build --release --bin mfb && scripts/test-accept.sh target/release/mfb /tmp/plan109-a-base 'rt-behavior/crypto/crypto-kat-valid'` | MET 2026-08-29 (`acceptance tests passed (1 test(s) ran)`) |

Everything below assumes those checks pass. The Status column is a snapshot; the
Command column is the truth and must be re-run before implementation.

## 1. Goal

- `Hash.SHA2_224`, `SHA2_256`, `SHA2_384`, and `SHA2_512` replace the four old
  `SHA###` spellings; `Hash.SHA1` computes the FIPS 180-4 digest through every
  hash-selected API, and every explicit source use of `Hash.SHA1` emits one
  non-fatal compile-time warning while the program still builds and runs.

### Non-goals (explicit constraints)

- No compatibility aliases for `SHA224`/`SHA256`/`SHA384`/`SHA512`: “rename” is
  a deliberate source break, and retaining aliases would create duplicate enum
  ordinals and prolong the old API.
- Do not warn on compiler-injected/internal uses; warn on user-authored enum
  member access, including MATCH patterns, exactly once per source occurrence.
- Do not weaken SHA-1 at runtime. The warning communicates collision weakness;
  the selected algorithm still returns the standard digest and remains usable
  for legacy interoperability.

## 2. Current State

`Hash` has four SHA-2 variants with ordinals 0–3
(`src/codegen/builtins/crypto/mod.rs:package`). `crypto::hash` dispatches those
ordinals in `gen_hash.rs`; HMAC, HKDF, and PBKDF2 dispatch through
`helper_sha_digest.rs`, `helper_sha_block_size.rs`, and
`helper_sha_output_len.rs`. The warning pipeline already supports non-fatal
rules through `SyntaxChecker::report_warning`, but `EnumVariant` carries only a
name and description and no diagnostic metadata.

### Measured populations

| What | Count | Command |
|---|---:|---|
| non-golden files using old `Hash.SHA###` spellings | 18 | `rg -l 'Hash\.SHA(224|256|384|512)' --glob '!**/golden/**' . \| wc -l` |
| non-golden old-spelling occurrences | 105 | `rg -n 'Hash\.SHA' benchmark tests src --glob '!**/golden/**' \| wc -l` |
| SHA spelling/ordinal references inside crypto Rust modules | 76 | `rg -n 'SHA224|SHA256|SHA384|SHA512' src/codegen/builtins/crypto --glob '*.rs' \| wc -l` |
| existing crypto fixture directories | 9 | `find tests/rt-behavior/crypto tests/syntax/crypto tests/byte-identity/crypto -name project.json \| wc -l` |

### Verified properties

- Enum values are HIR `MemberAccess` nodes in both expressions and MATCH
  literals — verified by reading `syntaxcheck/inference.rs` arms at the two
  `HirExpression::MemberAccess` matches.
- Warnings do not set `had_error` — verified in
  `syntaxcheck/mod.rs:report_warning`.
- Hash ordinals are observable only inside generated package bodies/codegen;
  no serialized public ABI promises their numeric values — verified by reading
  `RegistryEnum::render`, which emits names in order and no explicit values.

## 3. Design Overview

Put `SHA1` first, then the renamed SHA-2 variants, and update all ordinal
constants/dispatches together. Extend `EnumVariant` with optional advisory
metadata (rule + detail), make registry lookup expose it, and have syntaxcheck
report it when resolving a user HIR enum member. This general seam avoids a
crypto-name special case and is covered with a registry unit test plus a syntax
fixture.

Implement SHA-1 in the same pure-MFB, 32-bit-masked style as SHA-256. Extend the
generic digest/block/output helpers so HMAC/HKDF/PBKDF2 accept SHA-1 consistently
(20-byte output, 64-byte block); the warning applies regardless of which public
function consumes the selector.

This is behavior-changing work; byte identity is not a correctness gate. The
crypto `.ir`/`.ncode` sentinels and all importing fixture AST/IR are expected to
change because enum names, ordinals, and injected helper bodies change. Any
other target diff must be localized with one `.ncode`/objdump before goldens are
regenerated.

Correctness risk concentrates in ordinal drift across the native `hash` path and
source-helper HMAC/KDF path. Design uncertainty concentrates in emitting one
warning for both normal expressions and MATCH patterns without warning on
injected package source; prove that seam first.

Rejected: hard-code `Hash.SHA1` in syntaxcheck (not reusable, bypasses registry
authority); deprecate all SHA-1-taking functions (overbroad); retain old SHA-2
aliases (contradicts the requested rename and makes ordinal behavior ambiguous).

## Compatibility / Format Impact

Source using the four old SHA names stops compiling until renamed. `SHA1` adds a
warning-only diagnostic and standard 20-byte outputs. No key or ciphertext wire
format changes in this letter.

## Phases

### Phase 1 — generic enum-value advisory seam

- [x] Add optional advisory metadata to `EnumVariant` in
      `src/codegen/registry/mod.rs`, preserving `None` at every existing variant
      (`EnumAdvisory { rule, detail }`; 57 literal sites got `advisory: None`,
      `rg -c 'advisory: None' src`; lookup `Registry::enum_variant_advisory`
      keyed by owning package so a same-named user enum never inherits it).
- [x] Thread registry enum/member lookup into the two user-HIR member-access
      checking paths and emit one `report_warning` diagnostic — both paths
      (expression and `MATCH` literal, via `check_match_pattern` →
      `infer_expression`) resolve at the single enum arm of
      `inference.rs:infer_member_access`, gated on `!file.internal`; see
      `syntaxcheck/mod.rs:builtin_enum_member_advisory`.
- [x] Add the warning rule and severity to the diagnostics registry/spec, with
      unit tests proving warning-not-error and no duplicate emission
      (`2-203-0136 CRYPTO_SHA1_INSECURE`, warn; `01_rule-codes.md` row + the
      nine-warn-rules sentence; `builtin_enum_advisory_warns_once_per_user_occurrence`,
      `builtin_enum_without_advisory_never_warns`,
      `enum_variant_advisory_is_keyed_by_package_enum_and_member`).
- [x] (added) Land `Hash.SHA1` + the SHA-1 core with the seam — the acceptance
      below needs a *running* `Hash.SHA1` program, and a variant without a core
      would silently fall through to the SHA-224 arm (see Corrections).

Acceptance: a minimal `Hash.SHA1` fixture emits exactly one named warning and
still produces/runs an executable; a non-advisory enum member emits none.
VERIFIED 2026-08-29: `tests/rt-behavior/crypto/crypto-sha1-advisory-valid`
build.log pins exactly one `warn[2-203-0136 CRYPTO_SHA1_INSECURE]` per
occurrence (line 12 MATCH literal, line 22 expression), none for `Hash.SHA2_256`
in the same two contexts, `[exit 0]`, and the run prints the FIPS 180-4 digest.
Commit: f27f3f343

### Phase 2 — rename SHA-2 and implement SHA-1

- [x] Rename the four enum variants and all 105 measured non-golden uses; update
      docs/examples/benchmarks, resolver tests, ordinal comments, and helpers
      (123 occurrences in 22 files at execution — see Corrections; `mod.rs`
      variants + `gen_hash::ORD_SHA2_*` + the three dispatch helpers; man
      descriptors of `hash`/`hmac`/`hkdf`/`pbkdf2` and `10_crypto.md` list the
      five selectors with the SHA-1 advisory; `rg 'Hash\.SHA(224|256|384|512)'
      --glob '!**/golden/**' .` → only the removed-spelling fixture).
- [x] Add pure-MFB SHA-1 schedule/compression/padding helpers and wire native
      `hash` plus generic HMAC/HKDF/PBKDF2 size/dispatch helpers (landed in the
      Phase 1 commit: `helper_rotl32`/`helper_sha1_f`/`helper_sha1_k`/
      `helper_sha1_schedule`/`helper_sha1_bytes`/`helper_sha1_text`, reusing
      `__crypto_pad512`; `gen_hash` ordinal 0 arm; SHA1 arms in
      `__crypto_sha{Digest,BlockSize,OutputLen}`).
- [x] Add FIPS 180-4 KATs for empty, `abc`, and multi-block inputs and invalid
      argument fixtures for each modified public overload — KATs landed in the
      Phase 1 commit (`crypto-kat-valid`: SHA-1 §A.1/§A.2 + empty, both
      overloads; HMAC-SHA1 RFC 2202 #2; HKDF-SHA1 RFC 5869 #4; PBKDF2-HMAC-SHA1
      RFC 6070 #1/#2). Invalid coverage: runtime
      `rt-behavior/crypto/crypto-kdf-invalid` (HKDF `255*L` ceiling per
      selector — 5100/5101 for SHA1, 8160/8161 for SHA2_256 — plus length 0;
      PBKDF2 iterations/length 0 for SHA1 and SHA2_512; all
      `ErrInvalidArgument`, in-range boundary calls succeed) and compile-time
      argument typing of `hash`/`hmac` (both overloads) in the syntax fixture
      below (`TYPE_CALL_ARGUMENT_MISMATCH`). `hash`/`hmac` are total at runtime,
      so their only invalid inputs are type-level.
- [x] Add a syntax fixture proving each removed old spelling is rejected and a
      warning fixture proving SHA1 warns in normal and MATCH contexts —
      `syntax/crypto/hash-removed-spellings-invalid` pins
      `TYPE_UNKNOWN_ENUM_MEMBER` for each of `SHA224`/`SHA256`/`SHA384`/`SHA512`
      (`[exit 1]`); the warning fixture is `crypto-sha1-advisory-valid`
      (Phase 1 commit).

Acceptance: FIPS SHA-1/SHA-2 KAT bytes match; SHA-1 works through hash, HMAC,
HKDF, and PBKDF2; old spellings fail; warning count is exact and non-fatal.
VERIFIED 2026-08-29: `crypto-kat-valid` build.log run block is byte-identical
before/after the rename apart from the advisory text (FIPS/RFC SHA-1 lines
present through `hash` both overloads, `hmac`, `hkdf`, `pbkdf2`);
`hash-removed-spellings-invalid` `[exit 1]` with one
`TYPE_UNKNOWN_ENUM_MEMBER` per old name; `crypto-sha1-advisory-valid` two
warnings for two occurrences, `[exit 0]`.
Commit: 166017205 (spec citation placement follow-up: b5dfbec8b)

## Validation Plan

- Tests: registry/syntaxcheck unit tests; `tests/rt-behavior/crypto` KATs; new
  `tests/syntax/crypto` warning and removed-name fixtures.
- Coverage check: ensure the KAT calls both `List OF Byte` and `String` hash
  overloads and every hash-selected function.
- Runtime proof: build/run the release KAT and compare printed digests to FIPS
  vectors, not merely to compiler goldens.
- Doc sync: update registry man descriptors and `src/docs/spec/stdlib/10_crypto.md`;
  add the warning to the diagnostics constant registry if it has a code.
- Acceptance: `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`;
  fresh `cargo build --release --bin mfb`; full `cargo test`; filtered release
  acceptance; regenerate only expected crypto/importer drift; then
  `scripts/artifact-gate.sh target/release/mfb all`.

## Validation ledger (2026-08-29)

- rustfmt root + `repository/`: run. Fresh `cargo build --release --bin mfb`.
- `cargo test --no-fail-fast`: 63 targets ok; the single red, `golden.rs
  artifact_gate_all`, failed in 0.17s on "Another artifact-gate (pid 97419) is
  running" (a peer session's gate holding the global lock) — re-run standalone
  as `scripts/artifact-gate.sh target/release/mfb all`: **1262 tests, 1409
  builds, 1738 goldens checked, 0 diffs**.
- Filtered release acceptance on every crypto importer (`rg -l 'IMPORT crypto'
  tests benchmark`, 14 fixtures): green after golden sync; `mfb test
  tests/acceptance` 707/707.
- Full `scripts/test-accept.sh` (frozen copy of the letter-A binary): 1279 ran,
  the only mismatches are the two plan-109-B fixtures already in the tree
  (`crypto-sha3-kat-valid`, `crypto-kdf-invalid`'s `shake256` lines), which
  this binary cannot build by design.
- Docs: `mfb man crypto --all`, `mfb man crypto types`, `mfb spec stdlib crypto`
  render with zero leaked `[[` citations.

## Open Decisions

- Warning rule spelling/code — recommend a crypto-specific advisory
  `CRYPTO_SHA1_INSECURE` in the existing warning registry rather than overloading
  declaration deprecation, because only enum-value uses need the behavior.

## Corrections

- Prerequisite row 1's command (`find planning -maxdepth 1 -name 'plan-109-*' -newer
  planning/plan-109-A-…`) is not a test for an unfinished implementation: it lists
  B–F, which were saved seconds after A (`ls -la planning/plan-109-*`, all
  `Aug 29 06:12`). Replaced with a source census for any plan-109 spelling
  (`rg -l 'SHA2_256|SHA3_|X448|Ed448' src tests benchmark | wc -l` → 0), which
  measures the property the row actually gates on.
- The plan cites `src/docs/spec/diagnostics/02_error-codes.md` for compiler
  advisories; that file is the RUNTIME `errorCode::` registry. Compiler rules
  (severity `warn`/`error`) live in `src/rules/table.rs:RULES` and are pinned
  to `src/docs/spec/diagnostics/01_rule-codes.md` by
  `rules::tests` (`rules missing from src/docs/spec/diagnostics/01_rule-codes.md`).
- Phase 1's acceptance ("a minimal `Hash.SHA1` fixture … still produces/runs an
  executable") cannot be met by the seam alone: a `SHA1` variant whose ordinal
  has no core would compile and silently return the SHA-224 digest
  (`gen_hash::emit_dispatch` falls through on the unmatched ordinal). So the
  SHA-1 core, its `hash`/HMAC/HKDF/PBKDF2 wiring, and the FIPS/RFC KATs moved
  from Phase 2 into the Phase 1 commit; Phase 2 keeps the rename, the
  removed-spelling fixture, the invalid-argument fixtures, and the doc sweep.
  Ordinals shifted once here (SHA1=0 pushes the four SHA-2 ordinals to 1–4) and
  the names change in Phase 2 — one golden regen per commit.
- Rename census at execution: 22 non-golden files / 123 `Hash.SHA###`
  occurrences (`rg -l 'Hash\.SHA(224|256|384|512)' --glob '!**/golden/**' .`
  before the sweep; the plan's 18/105 predates the Phase 1 KAT/advisory fixtures
  and the syntaxcheck unit tests, which added the rest). A mechanical
  `Hash.SHA###` → `Hash.SHA2_###` sweep must EXCLUDE the removed-spelling
  fixture (`tests/syntax/crypto/hash-removed-spellings-invalid`), whose whole
  point is to keep the old names — the first pass rewrote it and was reverted.
- `benchmark/mfb` cannot be built locally to prove its rename compiles: it
  declares an uninstalled `bench_workers` package
  (`IMPORT_PACKAGE_NOT_INSTALLED`, no `benchmark/mfb/packages/` in the tree),
  so `benchmark/mfb/src/crypto.mfb` is covered only by the same API the
  acceptance/KAT fixtures exercise.
- "The two user-HIR member-access checking paths" is one path: the `MATCH`
  literal check (`check_match_pattern`) infers its literal through the same
  `infer_member_access` enum arm as an expression, so the warning is emitted
  at exactly one site and once-per-occurrence follows structurally (verified:
  10 occurrences → 10 warnings in the probe, 2 → 2 in the fixture).

## Summary

This letter lands the breaking names and warning infrastructure first. The main
risk is keeping five hash ordinals synchronized across two dispatch families;
HPKE and Curve448 remain untouched until later letters.
