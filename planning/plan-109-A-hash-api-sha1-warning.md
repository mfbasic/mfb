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
| no unfinished plan-109 implementation exists | `find planning -maxdepth 1 -name 'plan-109-*' -newer planning/plan-109-A-hash-api-sha1-warning.md` | MET at authoring; re-run |
| existing crypto KAT passes before the rename | `cargo build --release --bin mfb && scripts/test-accept.sh target/release/mfb /tmp/plan109-a-base 'rt-behavior/crypto/crypto-kat-valid'` | re-verify at kickoff |

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

- [ ] Add optional advisory metadata to `EnumVariant` in
      `src/codegen/registry/mod.rs`, preserving `None` at every existing variant.
- [ ] Thread registry enum/member lookup into the two user-HIR member-access
      checking paths and emit one `report_warning` diagnostic.
- [ ] Add the warning rule and severity to the diagnostics registry/spec, with
      unit tests proving warning-not-error and no duplicate emission.

Acceptance: a minimal `Hash.SHA1` fixture emits exactly one named warning and
still produces/runs an executable; a non-advisory enum member emits none.
Commit: —

### Phase 2 — rename SHA-2 and implement SHA-1

- [ ] Rename the four enum variants and all 105 measured non-golden uses; update
      docs/examples/benchmarks, resolver tests, ordinal comments, and helpers.
- [ ] Add pure-MFB SHA-1 schedule/compression/padding helpers and wire native
      `hash` plus generic HMAC/HKDF/PBKDF2 size/dispatch helpers.
- [ ] Add FIPS 180-4 KATs for empty, `abc`, and multi-block inputs and invalid
      argument fixtures for each modified public overload.
- [ ] Add a syntax fixture proving each removed old spelling is rejected and a
      warning fixture proving SHA1 warns in normal and MATCH contexts.

Acceptance: FIPS SHA-1/SHA-2 KAT bytes match; SHA-1 works through hash, HMAC,
HKDF, and PBKDF2; old spellings fail; warning count is exact and non-fatal.
Commit: —

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

## Open Decisions

- Warning rule spelling/code — recommend a crypto-specific advisory
  `CRYPTO_SHA1_INSECURE` in the existing warning registry rather than overloading
  declaration deprecation, because only enum-value uses need the behavior.

## Corrections

None yet.

## Summary

This letter lands the breaking names and warning infrastructure first. The main
risk is keeping five hash ordinals synchronized across two dispatch families;
HPKE and Curve448 remain untouched until later letters.
