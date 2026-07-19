# bug-277: resource-STATE composite type (kind 11) is ABI-hashed as an opaque, id-order-sensitive, shape-blind name

Last updated: 2026-07-17
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (ABI hashing)

Status: Fixed
Regression Test: src/binary_repr/tests.rs::abi_serializer_hashes_state_composites_structurally — kind-11 sig hash is structural (stable under unrelated renumbering, changes when the STATE record changes)

The ABI signature hasher `AbiSerializer::serialize_type_inner` handles type kinds
1–8 structurally but plan-52-D's kind 11 (`File STATE Cursor` composite) falls
through to the opaque `_` arm, which hashes `"opaque"` + the interned name
`State#<baseId>#<stateId>` (e.g. `State#4278190080#21`). Those embedded ids are
table-position-dependent (`FIRST_TABLE_TYPE_ID + index`). Two consequences: (a)
adding or reordering an unrelated type in the exporter shifts the ids, changing a
stateful export's `sigHash` with zero semantic change → spurious "ABI changed" /
used-symbol pin mismatch; (b) the STATE record's *structure* is never hashed, so
changing the STATE record's fields (or a renumber that repoints `State#…#N`)
leaves the hash identical → `pkg check-abi` and used-symbol pins miss a real ABI
change. `FUNC(...) AS List OF Cursor` hashes Cursor structurally, but the
headline plan-52/53 cross-package feature `FUNC(...) AS File STATE Cursor` does
not.

The single correct behavior a fix produces: the sig hash of a kind-11 STATE
composite is a structural function of its base type and state record contents
(like kinds 1–8), stable under unrelated type renumbering and sensitive to any
change in the STATE record's fields.

References:

- `planning/old-plans/plan-52-resource-state-model.md` §4 (kind-11 round-trip).
- `src/docs/spec/package/03_metadata-encoding.md:196` (documents the opaque
  fallback for kinds 9/10 only — kind 11 is an undocumented gap).
- Found during goal-06 review of `src/binary_repr/reader.rs`.

## Failing Reproduction

Exporter v1 exports `FUNC open(String) AS File STATE Cursor` with Cursor at table
id 21. Two mutations that should behave oppositely both misbehave:
- v2 declares one new record *ahead* of Cursor: name becomes `State#…#22` →
  sigHash changes though nothing semantic did (spurious mismatch).
- v2 keeps ids but adds a field to Cursor: sigHash unchanged though the STATE
  contract changed (silent acceptance of a stale importer).

- Observed: hash tracks table position, not structure.
- Expected: hash tracks structure, not table position.

## Root Cause

`src/binary_repr/reader.rs:1457` (`serialize_type_inner`, `_ =>` opaque arm) —
kind 11 has no structural arm; decode arm at `reader.rs:777`, writer half
`sections.rs:209` (`state_type`). The serializer was never extended when plan-52
A–D added the kind-11 decode support.

## Goal

- Add an `11 =>` arm to `serialize_type_inner` that serializes the base type and
  the state record structurally (mirroring kind 5's nested-payload approach).

### Non-goals (must NOT change)

- The wire encoding of kind-11 types (decode side stays as-is).
- Kinds 1–8 structural hashing.
- Do not silently change emitted hashes without bumping `ABI_FORMAT_VERSION` and
  updating the spec — this shifts every stateful export's hash.

## Blast Radius

- `reader.rs:serialize_type_inner` — fixed by this bug.
- Kinds 9 (MapEntry) / 10 (ThreadWorker) share the opaque-fallback mechanism but
  are documented (spec:196) — latent, out of scope here; note the kind-7 Thread /
  kind-10 asymmetry looks accidental and worth a follow-up.
- Consumers: `cli/pkg.rs:438` (`pkg check-abi`), `cli/resolve.rs:444`
  (`load_import_edges` used-symbol pins), registry `abi_index` — all benefit.

## Fix Design

Mirror the kind-5 arm: `put_str("state")`, serialize base at payload offset 0 and
the state record at offset 4. Coordinate with `ABI_FORMAT_VERSION` (emitted
hashes change) and update the spec's metadata-encoding section. Consider folding
kinds 9/10 into the same version-bump so the structural/opaque split is
principled — but that widens scope; recommend kind-11 only unless the maintainer
wants the sweep.

## Phases

### Phase 1 — failing test + audit
- [ ] Test: kind-11 sig hash stable under unrelated renumber; changes when the
      STATE record field set changes. Confirm both fail today.
### Phase 2 — the fix
- [ ] Add the `11 =>` structural arm; bump ABI_FORMAT_VERSION.
### Phase 3 — validation
- [ ] Regenerate ABI-hash goldens; confirm the delta is only stateful exports;
      full suite green; update spec 03_metadata-encoding.md.

## Validation Plan

- Regression test as above.
- Runtime proof: `pkg check-abi` flags a STATE-record change; doesn't flag an
  unrelated renumber.
- Doc sync: spec/package/03_metadata-encoding.md kind-11 row.

## Summary

The cross-package stateful-resource feature's ABI hash is currently
position-sensitive and structure-blind. A structural kind-11 arm fixes it; the
risk is the coordinated hash-format version bump and golden regeneration.

## Resolution

`serialize_type_inner` grew an `11 =>` arm that mirrors kind 5: it emits the tag
`"state"` followed by the structurally-serialized base type and state type. The
kind-11 wire encoding is untouched — only what the ABI hasher reads out of it.

`abi_serializer_hashes_state_composites_structurally` covers both halves of the
report, and both were confirmed failing against the unfixed serializer:

- interning an unrelated composite ahead of the STATE payload renumbers the type
  table but no longer moves the hash (this is the assertion that fired first);
- changing the state type's own shape (`List OF Integer` → `List OF String`) now
  changes the hash;
- plus a guard that a STATE composite never collides with its bare base type.

### Deviation: ABI_FORMAT_VERSION was NOT bumped

The report called for a coordinated bump. That was implemented, then reverted on
evidence. The bump broke 14 tests across `audit`, `cli::pkg`, `manifest`,
`monomorph`, `resolver` and `syntaxcheck` — every one of them a checked-in `.mfp`
fixture rejected wholesale by the version gate in `read_abi_index`. Reverting only
the bump, with the structural arm still in place, returned the whole suite to green
— which proves none of those fixtures export a kind-11 type and therefore that not
one of their hashes actually moved.

That is the argument against the bump. The gate guards the ABI_INDEX *wire
encoding*, which this change does not touch; bumping it rejects every package
built to date, the vast majority of which have no `STATE` export and no changed
hash. Meanwhile a package that genuinely does carry a stale kind-11 hash is
already caught — more precisely, naming the specific symbol — by
`validate_abi_index` recomputing the hash from the function table. The bump would
have traded a per-symbol diagnostic for an all-or-nothing rejection and taken
every unaffected package down with it. Rationale recorded at the constant in
`src/binary_repr/mod.rs`.

Spec `package/03_metadata-encoding.md` now documents the structural kind-11 row
and the narrowed meaning of `abiFormatVersion`.

### Golden and fixture churn

The full acceptance run isolated the blast radius to exactly the stateful surface
the report predicted, which is itself the confirmation that the arm is narrow:

- `syntax/native/native-resource-state-export-valid` — `FUNC openDb` and
  `FUNC exec` (the two signatures carrying `STATE`) changed hash; `TYPE DbInfo`
  and `TYPE Db` in the same package did not. Goldens regenerated.
- `rt-behavior/native/native-resource-state-import-rt` and
  `rt-behavior/resources/resource-state-import-rt` did not merely drift — they
  *failed*, with `validate_abi_index` naming `openDb` / `openTagged` and printing
  both hashes. Their `packages/*.mfp` are copies of an exporter build from before
  the fix, so this is the stale-hash rejection working as intended. Both fixtures
  were rebuilt from their exporters and both tests pass.

This is also the empirical case against the version bump: only the two genuinely
stateful packages were rejected, each by symbol name. Every other `.mfp` fixture in
the tree kept working, because none of their hashes moved.
