# plan-41-B: Scalar primitive — wire format & package ABI

Last updated: 2026-07-13
Effort: medium
Depends on: plan-41-A

Give `Scalar` a place in the portable binary representation: a wire type id, a
constant-pool encoding for a 32-bit scalar constant, the reverse (wire→name)
decode, and the compact runtime collection-type code. After this sub-plan a
`Scalar` can be serialized into a `.mfp` package — in a function signature, an
exported type, or a constant pool — and decoded back byte-for-byte. This is the
foundation the native codegen (plan-41-C) relies on to emit scalar constants.

The load-bearing fact from the census: **there is no free low wire id.** Ids 1–9
are taken (`TYPE_MONEY = 9` took the last vacancy), and `FIRST_TABLE_TYPE_ID =
10`. A new primitive must take id **10** and bump the table-type base, which
renumbers every table (record/union/etc.) type id in the wire format. That is an
ABI change; because `.mfp` files are build artifacts and the repo regenerates
goldens, we regenerate rather than preserve, and rely on `check-abi` to catch
anything stale.

**Since the renumber cost is paid once regardless, we push the base out with
room to spare:** `Scalar` takes id **10**, ids **11–19 are reserved for future
primitives**, and `FIRST_TABLE_TYPE_ID` becomes **20**. Reserving the band now
means the *next* primitive claims a reserved id (11, 12, …) as a pure additive
edit — no second table renumber, no second golden regeneration. The gap costs
nothing (wire ids are a `u32`; nine unused values are free) and buys additive
headroom for the primitives that will follow `Scalar`.

References (read first):

- `mfb spec package` — `src/docs/spec/package/04_type-table.md:21-35` (wire-id
  table, `7 = Byte`, `9 = Money`, `FIRST_TABLE_TYPE_ID = 10`) and
  `05_constant-pool.md:30,43` (per-type const encoding).
- `mfb spec memory scalar-storage` (`src/docs/spec/memory/01_scalar-storage.md`)
  — the scalar payload-size table.
- plan-41-A (the front-end this builds on) and the `Money`/`Byte` wire precedents.

## 1. Goal

- `Scalar` serializes as wire type id **10**; ids **11–19** are reserved for
  future primitives; `FIRST_TABLE_TYPE_ID` becomes **20** and table types
  renumber accordingly.
- A `Scalar` constant round-trips through the constant pool as a 4-byte
  little-endian codepoint payload: encode → `.mfp` → decode yields the identical
  codepoint.
- A function signature or exported type mentioning `Scalar` round-trips through
  `.mfp` encode/decode with byte-identical output on re-encode.
- `check-abi` recognizes `Scalar` in an ABI index; the compact
  `COLLECTION_TYPE_SCALAR` runtime code (next free = 9) is assigned for
  `List OF Scalar`/`Map ... TO Scalar` layout.

### Non-goals (explicit constraints)

- **No codegen.** This sub-plan only encodes/decodes; emitting a scalar immediate
  or comparison is plan-41-C.
- **No silent ABI drift.** The `FIRST_TABLE_TYPE_ID` bump (10→20) renumbers table
  types — that must be reflected in the spec (deferred text to plan-41-E, but the
  numbers must be correct here) and all in-repo `.mfp`/golden artifacts
  regenerated in one commit so the tree is internally consistent.
- **The reserved band 11–19 stays unmapped.** Reserved ids get no name→id entry
  and no `primitive_type_name` arm — decoding one is an error until a real
  primitive claims it. Do not pre-wire placeholder names.
- **Payload width is 4 bytes.** Not 8 (that would waste space and misalign with
  `Byte`'s single-byte precedent); not 1 (a codepoint needs 21 bits).

## 2. Current State

Wire ids: `src/binary_repr/mod.rs:47-69` — `TYPE_BYTE = 7` (:53),
`TYPE_MONEY = 9` (:58), `FIRST_TABLE_TYPE_ID = 10` (:69). Name→id map:
`src/binary_repr/sections.rs:80-146` (`"Byte" => TYPE_BYTE` :145,
`"Money" => TYPE_MONEY` :146). Const-pool `ConstEntry` per-type encoding:
`src/binary_repr/sections.rs:374-395` — `Fixed`/`Money` write `kind = wire id`
plus an LE i64; `Byte` writes kind 7 plus a single byte. Reverse decode
(wire→name): `src/binary_repr/reader.rs:802-812` (`primitive_type_name`, Money
:809, Byte :811).

Compact runtime collection-type code (a **separate** namespace from the wire id):
`src/target/shared/code/error_constants.rs:638-651` —
`COLLECTION_TYPE_BYTE = 7` (:644), `COLLECTION_TYPE_MONEY = 8` (:648); next free
= **9** (containers start at 20).

ABI index / `check-abi`: driven by the type-name spellings and signature hashes
that the `FIRST_TABLE_TYPE_ID` renumber affects; the resolver/ABI path consumes
`primitive_type_name` and the `type_id` map above.

`Byte` is the exact template for the const-pool arm (a small fixed-width payload
rather than Money's i64), differing only in width (1 → 4 bytes).

## 3. Design Overview

Three additive edits and one renumber:

1. **Assign the id + reserve the band.** `TYPE_SCALAR = 10`;
   `FIRST_TABLE_TYPE_ID = 20` (ids 11–19 reserved, unmapped). Add
   `"Scalar" => TYPE_SCALAR` to the name→id map and `TYPE_SCALAR => "Scalar"` to
   `primitive_type_name`. Document the reserved range with a comment at the id
   constants so the next primitive knows to fill from 11.
2. **Const-pool arm.** A new `ConstEntry` arm writing `kind = TYPE_SCALAR` + a
   4-byte LE codepoint, and the matching decode. Mirror `Byte`, widen to 4 bytes.
3. **Compact collection code.** `COLLECTION_TYPE_SCALAR = 9`.
4. **Regenerate.** Rebuild all in-repo `.mfp`/golden artifacts so the renumber is
   consistent tree-wide, and confirm re-encode is byte-identical.

**Risk concentrates in the `FIRST_TABLE_TYPE_ID` renumber** — it silently shifts
every table-type id in every serialized package. The mitigation is a single
atomic regeneration + a round-trip byte-identity test that fails loudly if any
artifact was missed. The const-pool arm is low-risk (a 4-byte mirror of `Byte`).

Rejected alternatives:
- *Reserve a high fixed id to avoid the renumber* — table types occupy the base
  `..N` dynamically, so there is no safe id above the moving base; any new
  primitive id collides with the table range unless the base moves. The renumber
  is unavoidable — so we do it once and move the base far enough (to 20) to leave
  a reserved primitive band below it.
- *Bump the base only to 11 (no reserved band)* — rejected; the renumber+golden
  regeneration is the entire cost, and it is identical whether the base moves to
  11 or 20. Stopping at 11 would force a second full renumber for the very next
  primitive. Reserving 11–19 spends nothing extra now to avoid that.
- *Reserve a huge band (e.g. base 256)* — unnecessary; nine slots comfortably
  cover the foreseeable primitives, and an oversized gap makes the table-type ids
  needlessly large in every serialized package. If the band ever fills, that
  primitive pays one more renumber — the same trade we are making once here.
- *8-byte payload to match Money* — wasteful and breaks the "narrowest correct
  width" precedent set by `Byte`.

## Compatibility / Format Impact

**This is the ABI-affecting sub-plan.** `FIRST_TABLE_TYPE_ID` 10→20 renumbers all
table-type wire ids (with ids 11–19 reserved for future primitives); a new
const-pool `kind = 10` payload format is added. Externally: any previously
compiled `.mfp` is stale and must be recompiled — `check-abi` will flag
mismatches. In-repo artifacts are regenerated in this sub-plan's commit.
Unchanged: primitive ids 1–9, the `Byte`/`Money`/`Fixed` payload formats, and the
compact collection-code base (containers still start at 20). Forward note: a
later primitive claiming a reserved id (11–19) is a purely additive edit — no
further table renumber and no golden regeneration.

## Phases

### Phase 1 — Wire id + name maps

- [ ] `TYPE_SCALAR = 10`, bump `FIRST_TABLE_TYPE_ID` to 20 in
      `src/binary_repr/mod.rs:47-69`, with a comment marking 11–19 as reserved for
      future primitives (fill from 11).
- [ ] Add `"Scalar" => TYPE_SCALAR` to `src/binary_repr/sections.rs:80-146` and
      `TYPE_SCALAR => "Scalar"` to `primitive_type_name` in
      `src/binary_repr/reader.rs:802-812`. Leave ids 11–19 unmapped (decoding one
      is an error).
- [ ] `COLLECTION_TYPE_SCALAR = 9` in
      `src/target/shared/code/error_constants.rs:638-651`.
- [ ] Tests: a unit test asserting name↔id round-trips for `Scalar`, that
      `FIRST_TABLE_TYPE_ID == 20`, and that a reserved id (e.g. 11) has no
      `primitive_type_name` mapping.

Acceptance: `Scalar` maps to id 10 and back by name; table-type ids start at 20;
reserved ids 11–19 are unmapped; unit round-trip test passes.
Commit: —

### Phase 2 — Constant-pool encode/decode

- [ ] Add the `Scalar` arm to `ConstEntry` encoding at
      `src/binary_repr/sections.rs:374-395` — `kind = TYPE_SCALAR`, 4-byte LE
      codepoint — mirroring the `Byte` arm widened to 4 bytes.
- [ ] Add the matching decode arm so a `Scalar` constant decodes to the same
      codepoint.
- [ ] Tests: a constant-pool round-trip unit test — encode a module containing a
      `Scalar` constant, decode, assert the codepoint is identical and re-encode
      is byte-identical.

Acceptance: a `Scalar` constant survives `.mfp` encode→decode→re-encode
byte-for-byte.
Commit: —

### Phase 3 — Signature/ABI round-trip + regeneration (highest-risk last)

- [ ] Confirm `Scalar` flows through function-signature and exported-type
      serialization (it rides the same `type_id`/`primitive_type_name` paths from
      Phase 1); add coverage where a `FUNC f(c AS Scalar) AS Scalar` signature
      round-trips.
- [ ] Regenerate all in-repo `.mfp`/golden artifacts affected by the
      `FIRST_TABLE_TYPE_ID` renumber in a single commit (use the repo's golden
      sync tooling); verify the working tree is internally consistent.
- [ ] Confirm `check-abi` accepts a `Scalar`-bearing signature and rejects a
      stale pre-renumber package.
- [ ] Tests: signature round-trip unit test + a `check-abi` test over a
      `Scalar`-typed exported function.

Acceptance: a package exporting `FUNC f(c AS Scalar) AS Scalar` encodes, decodes,
and re-encodes byte-identically; `check-abi` passes on it and flags a stale
package; regenerated goldens are consistent tree-wide.
Commit: —

## Validation Plan

- Tests: wire round-trip, const-pool round-trip, signature round-trip, and
  `check-abi` unit tests (`cargo test`).
- Runtime proof: N/A — no binary runs here; the proof is byte-identical
  encode/decode/re-encode. End-to-end runtime is plan-41-C onward.
- Doc sync: the numeric renumber (`FIRST_TABLE_TYPE_ID = 20`, `TYPE_SCALAR = 10`,
  reserved band 11–19) must be reflected in
  `src/docs/spec/package/04_type-table.md` and `05_constant-pool.md`; the full
  prose sync is plan-41-E but the numbers land here so the spec is never
  wrong-in-tree.
- Acceptance: `cargo test` green; regenerated goldens match; artifact/ABI gate
  (`scripts/artifact-gate.sh`) clean.

## Open Decisions

_All resolved 2026-07-13 (user)._

- **Renumber now vs. reserve a high id — DECIDED: renumber
  (`FIRST_TABLE_TYPE_ID` → 20).** There is no collision-free high id and `.mfp`
  are regenerable build artifacts. A single high reserved id for `Scalar` alone
  would require reworking the table-id allocation scheme for no compatibility
  benefit in a tree that recompiles everything. (§3)
- **Reserved-band size — DECIDED: reserve nine slots (base 20, ids 11–19
  reserved).** The renumber cost is identical whether the base moves to 11 or 20,
  so we buy additive headroom for future primitives for free and avoid a second
  renumber. Nine is enough for the foreseeable primitives without bloating
  table-type ids; a future primitive that overflows the band pays one more
  renumber — the same one-time trade made here. (§3)

## Summary

The one real hazard is the `FIRST_TABLE_TYPE_ID` renumber (10→20) touching every
table-type id in the wire format; it is contained by an atomic golden
regeneration and a byte-identity round-trip test. Because the renumber is
one-time and free to over-shoot, ids 11–19 are reserved so future primitives are
additive — no second renumber. The const-pool arm and compact collection code are
low-risk mirrors of `Byte`.
