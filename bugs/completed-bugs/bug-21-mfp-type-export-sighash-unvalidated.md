# bug-21: `.mfp` decode recomputes/validates sigHashes only for callable exports; Type/Union/Enum ABI export hashes are trusted unverified

Last updated: 2026-07-08
Effort: medium (1h–2h)

`src/binary_repr/reader.rs::validate_abi_index` (`:1104-1123`) cross-checks each
export's ABI `sig_hash` against the actual binary representation by recomputing
`function_sig_hash` — but it iterates only `exports`, the decoded `EXPORT_TABLE`
entries, which `decode_callable_export_kind` restricts to **Func/Sub**. The
`AbiExport` entries of kind **Type/Union/Enum** in `abi.exports` (written at build
time by `AbiIndex::from_project`, `sections.rs:~589-604`) are **never** re-derived
against the decoded `TYPE_TABLE` via the existing `type_sig_hash` (`reader.rs:1263`,
used only at build time). A tampered package can therefore ship a Type/Union/Enum
ABI sigHash that disagrees with its real type definition, and decode accepts it;
`package_info` and downstream ABI-compatibility gating then trust the forged hash.

The single correct behavior a fix produces: every ABI export hash — callable
**and** type/union/enum — is recomputed from the decoded tables at decode time and
rejected on mismatch, so no ABI surface is trusted unverified.

Severity LOW / defense-in-depth: the impact is bounded to ABI-compatibility gating
(compatibility decisions, `check-abi`), not memory safety — the actual type is
still re-derived from the TYPE_TABLE when lowering, so a forged hash cannot inject
type-confused layout. It is the one ABI surface left unchecked while its callable
sibling is checked, so it is an asymmetry worth closing. Related to audit-1 PKG-02
(decoded package data not fully re-validated), but distinct: this is a specific
missing hash cross-check, not the blanket "no IR re-typecheck."

References:

- `src/binary_repr/reader.rs:1104-1123` (`validate_abi_index` export loop — keys
  off callable `exports`, recomputes `function_sig_hash` at `:1115`, rejects at
  `:1116`).
- `src/binary_repr/reader.rs:1263` (`type_sig_hash` — exists, build-time only,
  never invoked on the decode path).
- `src/binary_repr/sections.rs:~589-604` (`AbiIndex::from_project` writes
  Type/Union/Enum `AbiExport`s).
- audit-1 PKG-02 (decode trust-boundary theme).
- Found during goal-01 review of `src/binary_repr/**`.

## Failing Reproduction

Craft a `.mfp` that passes identity/signature checks but whose `ABI_INDEX`
carries a Type/Union/Enum `AbiExport` with a `sig_hash` that does **not** match
its TYPE_TABLE definition, then decode/`package_info` it:

- Observed: decode accepts the package; the forged type-export hash is surfaced
  and trusted by compatibility gating.
- Expected: decode rejects with a "sigHash disagrees" error, exactly as it does
  today for a tampered **callable** export hash.

Contrast: a tampered **callable** export hash IS rejected (`:1115-1122`); import
table / dependency-edge agreement is also checked (`:1125-1173`). Only
type-export hashes escape verification.

## Root Cause

The validation loop derives its work-list from `exports` (callable only). No loop
mirrors it over the Type/Union/Enum `abi.exports`, and `type_sig_hash` — the
function that would recompute them — is never called at decode.

## Goal

- `validate_abi_index` recomputes and compares the `sig_hash` for every ABI export
  kind (Func, Sub, Type, Union, Enum), rejecting any mismatch.

### Non-goals (must NOT change)

- Callable-export and import/dep-edge validation (already correct).
- The build-time hash computation.

## Blast Radius

- `validate_abi_index` (`reader.rs:1104-1173`) — extend to type exports. No other
  consumer trusts these hashes before validation.

## Fix Design

In `validate_abi_index`, after the callable-export loop, iterate `abi.exports` of
kind Type/Union/Enum: resolve each to its TYPE_TABLE id by name, recompute
`type_sig_hash(id, kind, …)`, and reject on mismatch (mirroring `:1115-1122`).
Ensure a type export with no matching TYPE_TABLE entry is itself an error.

## Phases

### Phase 1 — failing test + audit

- [ ] Add a decode test with a crafted package whose type-export hash disagrees
      with its type definition; assert rejection. Confirm it is accepted today.
- [x] Blast-radius audit complete (above).

### Phase 2 — the fix

- [ ] Add the type-export hash cross-check to `validate_abi_index`.

### Phase 3 — validation

- [ ] `scripts/test-accept.sh`; confirm well-formed packages still decode
      byte-identically and `check-abi` goldens are unaffected.

## Validation Plan

- Regression test(s): the crafted-type-export-hash decode-rejection test + a
  well-formed round-trip that must still pass.
- Full suite: `scripts/test-accept.sh`.

## Summary

Decode validates callable-export hashes but not type/union/enum ones; the fix
mirrors the existing callable check using the already-present `type_sig_hash`.
Bounded to ABI-compatibility gating (no memory-safety consequence), so LOW.
