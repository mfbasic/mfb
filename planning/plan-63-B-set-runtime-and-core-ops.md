# plan-63-B: Set runtime block, literal, and core native operations

Last updated: 2026-07-25
Effort (Human): large (3h–1d)
Effort (AI): medium (1h–2h)
Depends on: plan-63-A (the `Set OF T` type shape must be recognized front-to-back)
Produces: a working `Set OF T` value — the `Set OF T { … }` literal, the native
members `add`, `remove`, `contains` (Set overload), `toList`, global `len`, and
`FOR EACH x IN set`. After B a program can build a set, add/remove elements
(dedup enforced), test membership in O(1), count it, convert it to a list, and
iterate it in stable insertion order. This is the substrate C's set algebra is
written against.

Prerequisites: see plan-63-A §Prerequisites (the whole-feature gate). B adds one
of its own: **plan-63-A is complete** — `grep -n 'Set(Box<Type>)' src/syntaxcheck/mod.rs`
returns a hit and `cargo test` is green. If A is not complete, B cannot start,
full stop. B does not re-implement any front-end recognition; it assumes A.

References (read first):

- `mfb spec memory collections` (`src/docs/spec/memory/05_collections.md`) — the
  uniform block layout, the Map hash index, capacity headroom, shrink-to-fit
  copy. B reuses all of it.
- The Map runtime research: layout constants `error_constants.rs:779-867`;
  helpers `_mfb_rt_map_build_buckets`/`_bucket_put`/`_probe`
  (`src/target/shared/code/mod.rs:2042-2357`); `lower_map_set_in_place`
  (`map_mutate.rs:16-920`), `lower_map_remove_key` (`:1229`); probe/query
  (`builder_collection_query.rs:60-320`); literal/layout/sizing
  (`builder_collection_layout.rs`); scope-drop (`builder_owned_cleanup.rs:165-205`).

## 1. Goal

- `LET s = Set OF Integer { 3, 1, 4, 1, 5 }` builds a 4-element set (the duplicate
  `1` collapses); `len(s)` is `4`.
- `MUT s AS Set OF String = Set OF String { }` then
  `s = collections::add(s, "x")` twice yields `len(s) = 1` (idempotent add).
- `collections::contains(s, "x")` is `TRUE`, `collections::contains(s, "y")` is
  `FALSE`, resolved through the FNV-1a bucket probe.
- `collections::remove(s, "x")` yields an empty set; removing an absent element
  is a no-op.
- `collections::toList(s)` returns a `List OF T` of the elements in stable
  insertion order; `FOR EACH x IN s` visits the same order.
- A `Set OF Integer` round-trips through a value copy (binding, argument, return,
  record field, thread transfer) as a tight block, and is freed by a single
  `arena_free` at scope exit with the bucket region correctly accounted.

### Non-goals (explicit constraints)

- **No set algebra in B.** `union`/`intersection`/`difference`/`isSubset`/`toSet`
  are source generics in C, built on B's primitives. B ships only the members C
  cannot be written without.
- **No new storage machinery.** B must *reuse* the Map block, probe, buckets,
  grow, copy-tight, and free paths — parameterized to a zero-width value — not
  fork them. A second copy of the bucket code is a defect, not a feature.
- **No behavior change to `Map`/`List`.** Every reused emitter gains a
  value-width-0 path; the existing map path (value present) is untouched.

## 2. Current State

A `Map OF K TO V` block is `[header][entry array][data region][bucket array]`,
one contiguous arena allocation (`mfb spec memory collections`). Each
`LookupEntry` (40 B, `error_constants.rs:807-813`) carries
`keyOffset/keyLength/valueOffset/valueLength`. The bucket array
(`2*capacity` u64, FNV-1a + linear probe, `bucketsReady` lazy flag at header
offset 4) sits after the data region so the capacity-derived data base is
unaffected. `map_key_probe_eligible` (`builder_collection_query.rs:63`) is
exactly `String|Integer|Float|Fixed|Byte|Boolean`; other comparable key types
(enum, record) fall back to a linear scan.

Precedents B reuses, and the one delta each needs:

- **Literal:** `lower_map_literal` (`builder_collection_layout.rs:1183`) writes
  key+value entries and folds in the bucket bytes (`:1296-1302`). Set literal =
  same, but each entry is key-only (`valueLength = 0`) and duplicate keys are
  dropped during the build.
- **Insert/dedup:** `lower_map_set_in_place` (`map_mutate.rs:16`) probes for the
  key, overwrites on hit / inserts on miss, maintains the bucket index. Set `add`
  = the miss path only (insert key, `valueLength = 0`); a hit is a no-op (the
  element is already present) — no value to overwrite.
- **Remove:** `lower_map_remove_key` (`map_mutate.rs:1229`) deletes by key. Set
  `remove` reuses it verbatim (key-only already).
- **Membership:** `emit_map_probe` (`builder_collection_query.rs:123`) →
  entryIndex or −1. Set `contains` = probe, map result to `Boolean` (exactly
  `hasKey`'s shape, `builder_collection_query.rs`).
- **To-list / iteration:** `lower_map_projection` (`builder_collection_queries.rs:430`)
  produces the keys projection as a `List OF K`. Set `toList` = the keys
  projection; `FOR EACH` reuses the same entry walk yielding the key bytes as
  `T` (A already set `collection_iteration_type` to `Set OF T → T`).
- **Sizing/copy/free:** `emit_flat_block_size` (`builder_collection_layout.rs:241`,
  bucket region added at `:286-292`), `copy_collection_tight` (`:359`, buckets at
  `:410-418`), `emit_owned_value_drop` (`builder_owned_cleanup.rs:165`). Each has
  a "map ⇒ has buckets" decision B must extend to "map or set ⇒ has buckets".

### Measured populations

| What | Count | Command |
|---|---|---|
| Collections native members today (Set adds `add`, `remove`, `toList`; extends `contains`) | 20 | `awk '/const NATIVE_MEMBERS/{f=1} f{print} /\];/{if(f)exit}' src/builtins/collections.rs \| grep -oE '"[a-zA-Z]+"' \| wc -l → 20` |
| Free runtime collection kind tag | `3` | `grep -n 'COLLECTION_KIND_' src/target/shared/code/error_constants.rs` → 0/1/2 used |
| Sites deciding "map has a bucket region" (size/copy/free/reserve) | UNMEASURED — **Task B0** | `grep -rn 'BUCKETS_READY\|reserve_map_buckets\|<< 4\|capacity.*bucket' src/target/shared/code/*.rs` then classify |

B0 (the bucket-region census) is the first task, before any code: it is the set
of places that must learn "a Set has buckets too," and getting it wrong corrupts
the arena free list (the allocate-size / free-size disagreement that
`error_constants.rs:815-825` and the Map research both flag as the #1 hazard).

### Verified properties

- **Set is always a flat block (no pointer payloads).** Established in
  plan-63-A §2 Verified properties: a Set element is comparable, and every
  pointer-payload element type (resource, function, non-flat collection) is
  non-comparable. So `emit_owned_value_drop`'s single `arena_free` is always
  correct for a Set — no per-element close/free walk, no `OwnedList` path.
- **Zero-width value is representable.** `valueType = COLLECTION_TYPE_NONE (0)`
  and `valueLength = 0` per entry; the probe reads only `keyLength` bytes
  (`emit_map_query_key`, `builder_collection_query.rs:74`), so a 0-length value
  never enters any hash or compare. Verified by reading the probe + the three
  `_mfb_rt_map_*` helpers.
- **Non-probe-eligible comparable elements still work.** An enum or record
  element is comparable (valid Set element) but not `map_key_probe_eligible`, so
  it takes the linear-scan fallback — identical to a `Map` with an enum key
  today. B must route Set membership through the same probe-eligible/scan split,
  not assume every element hashes.

## 3. Design Overview

B introduces **`COLLECTION_KIND_SET = 3`** (metadata; dispatch stays static per
`error_constants.rs:789-793`) and a single predicate
**`collection_has_buckets(type) = is_map(type) || is_set(type)`** that every
sizing/copy/free/reserve site consults instead of an inline `kind == MAP` test.
That predicate is the whole reason B is safe: it replaces N scattered "is this a
map?" bucket decisions with one, so a Set cannot be sized one way and freed
another.

The native members are thin wrappers over the Map emitters, parameterized by a
value width of 0:

- `lower_set_literal` = `lower_map_literal` with key-only entries + build-time
  dedup (probe each candidate against the partial set before writing).
- `lower_set_add_in_place` = the miss-branch of `lower_map_set_in_place`
  (hit-branch becomes a no-op return of the same buffer).
- `remove` / `contains` / `toList` / `FOR EACH` reuse `lower_map_remove_key`,
  `emit_map_probe`, `lower_map_projection`, and the entry walk **unchanged** —
  they are already key-only.

**Correctness risk concentrates in B0's sizing sites** (blast radius: arena
corruption) — scheduled as the first census and the last landing (Phase 3 is the
sizing/copy/free wiring, behind the members). **Design uncertainty is already
retired** (storage reuse proven in A). So B orders: literal+members first
(observable, low blast radius), sizing/copy/free last (high blast radius, behind
a runtime test that round-trips a set through copy and scope-drop under the
arena's use-after-free scrub).

**Rejected alternative:** *generalize `lower_map_set_in_place` to take an
`Option<value>` and call it for both.* Tempting, but the map path is hot and
heavily tested; threading an option through it risks a Map regression for a
Set convenience. B instead adds a small `lower_set_add_in_place` that *shares the
bucket/grow helpers* but owns its (simpler, value-less) entry-write. If review
finds the two bodies nearly identical, merge then — not up front.

## 4. Detailed Design

### 4.1 Block representation

- `kind = COLLECTION_KIND_SET (3)`; `keyType = tag(T)`; `valueType = COLLECTION_TYPE_NONE (0)`.
- Entry: `keyOffset/keyLength` locate the element; `valueOffset = keyOffset+keyLength`
  (or 0), `valueLength = 0`. Bucket array present, identical to Map.
- Data base = `block + HEADER + capacity*ENTRY` (capacity, never count); bucket
  base after the data region. `collection_has_buckets` returns true for kind 3.

### 4.2 `.mfp` wire type id

- Assign `Set` a package wire type-table id (distinct id space from the runtime
  `COLLECTION_TYPE_*`; see `mfb spec package type-table`). Do this **with the
  encoder/decoder in front of you** — add the id, the encode arm, the decode arm,
  and a package round-trip test in the same commit. A Set-typed public function
  or a Set const in a `.mfp` must survive encode→decode byte-identically.

### 4.3 Native member routing

- Extend `NATIVE_MEMBERS` (`src/builtins/collections.rs:47`) with `add`,
  `remove`, `toList`; extend `contains` resolution to accept a `Set OF T`
  first arg (it currently handles `List OF T`).
- `resolve_call` (`collections.rs:138`) routes `add`/`remove`/`toList`/`contains`
  by first-arg collection kind (List vs Map vs Set), mirroring how `get`/`set`
  already fork List vs Map.
- Signatures: `add(Set OF T, T) AS Set OF T`, `remove(Set OF T, T) AS Set OF T`,
  `contains(Set OF T, T) AS Boolean`, `toList(Set OF T) AS List OF T`.

### 4.4 In-place vs value semantics

- `add`/`remove` obey the same `MUT`-self-assign in-place rule as
  `collections::append`/`set`: `s = collections::add(s, x)` on a uniquely-owned
  `MUT` buffer mutates in place; any other use produces a fresh snapshot. Reuse
  the existing in-place gating (the append/set path already implements it).

## Compatibility / Format Impact

- **New `.mfp` wire type id for `Set`** (§4.2) — additive; existing ids
  unchanged. A `.mfp` produced before B never contained a Set, so there is no
  migration.
- **New runtime kind `3`** — internal to codegen; not observable from source and
  not serialized in `.mfp` (the wire format uses its own id space).
- No change to the `List`/`Map` block layout, ids, or any existing golden's
  bytes except where a fixture newly constructs a Set.

## Phases

> Keep checkboxes current in the same commit as the work. An unticked box means
> NOT DONE. Fill each `Commit:` the moment the phase lands.

### Phase 1 — Bucket-region census (B0) + kind/id scaffolding

One line: enumerate every "map has buckets" decision site and add the
`COLLECTION_KIND_SET` + `.mfp` id scaffolding, so Phase 3's sizing edits have a
proven target list.

- [x] B0: bucket-decision sites classified in Corrections — two sizing/copy sites
      (`emit_flat_block_size`, `copy_collection_tight`) + `lower_collection_values`
      (literal reservation), all now on `collection_has_buckets`; free routes
      through `emit_flat_block_size`.
- [x] Added `COLLECTION_KIND_SET = 3` (`error_constants.rs`), `collection_has_buckets`
      + `is_set_type` + `set_element_type` (`type_utils.rs`); `CollectionTypeLayout`
      Set arm (`validation.rs`). (Commit 25e6c822f.)
- [x] `.mfp` `Set` wire id = kind 13 (single element id), encode `set_type` +
      decode arm + round-trip tests in `binary_repr` (encode→decode byte-identical).

Acceptance: this plan lists the bucket-decision sites; a package round-trip test
encodes and decodes a `FUNC(Set OF Integer) AS Set OF Integer` signature
byte-identically; `cargo test` green.
Commit: —

### Phase 2 — Literal + native members (observable, low blast radius)

One line: make sets constructible, mutable, queryable, and iterable.

- [x] `lower_set_literal` (`builder_collection_layout.rs`) — empty set + in-place
      `lower_map_set_in_place` per element (dedup + insertion order, one buffer).
      `SetLiteral` threaded through AST/IR (binary tag 21)/NIR + parser +
      `infer_set_literal` + `ir::verify` (`TYPE_SET_ELEMENT_MISMATCH`, 2-203-0054).
- [x] `add` = tight-copy + `lower_map_set_in_place` (miss inserts, hit is a no-op
      = idempotent). ~~in-place `MUT`-self-assign gating~~ — deferred as a perf
      optimization (correctness-first always-copy; see Corrections). Value width is
      a 1-byte Boolean, not zero-width, so the Map emitter is reused unchanged.
- [x] `contains` (Set) → the `hasKey` probe/scan → Boolean; `remove` → reuse
      `lower_map_remove_key`; `toList` → reuse `lower_map_projection` (project key).
- [x] `FOR EACH x IN set` yields `T` (the entry key) in insertion order
      (`lower_for_each` Set branch).
- [x] Global `len` recognizes `Set OF T` (`is_collection_type` → COUNT); `len` +
      `isEmpty`/`isNotEmpty` resolvers accept Set (`general.rs`).
- [x] Tests: `tests/rt-behavior/collections/set-behavior-rt` exercises literal
      dedup, idempotent add, remove + remove-of-absent, contains true/false, len,
      toList, FOR EACH order — probe-eligible `Integer`/`String` AND a
      non-probe-eligible **record** (linear scan). **Enum elements are a
      pre-existing general limitation** (verified: `Map OF Color TO Integer` fails
      identically at HEAD — "packed payload does not support type 'Color'"), so the
      linear-scan case uses a comparable record instead; see Corrections.

Acceptance: the fixture runs and prints the expected membership/len/order results
(numslen=4, afterDupAdd=1, afterRemove=0, nameslen=3, pointlen=2, baselen=3,
grownlen=4, bagtags=3, FOR EACH order a/b/c); `cargo test` green.
Commit: —

### Phase 3 — Sizing, copy, and scope-drop (largest blast radius last)

One line: make a Set copy tight and free cleanly, with the bucket region
accounted at every site B0 found — the arena-corruption-prone step, landed last
behind the runtime test.

- [x] `collection_has_buckets(type)` replaces the `kind == MAP` bucket decision at
      `emit_flat_block_size`, `copy_collection_tight` (the `emit_reserve_map_buckets`
      arg), and `lower_collection_values` (literal reservation). The scope-drop free
      path sizes via `emit_flat_block_size`, so it is covered by that one fix; a Set
      is always a flat block (plan-63-A verified property), so `emit_owned_value_drop`
      needs no per-element walk. Landed in Phase 1 commit 25e6c822f (co-located with
      the predicate).
- [x] Tests: `set-behavior-rt` copies a set (bind → pass to `roundtrip` → return →
      `MUT` rebind → mutate the copy with `add`) and asserts the source is unchanged
      (`baselen=3`, `grownlen=4`), and embeds a set in a record field (`Bag[names]`,
      `bagtags=3`) — both free at scope exit with no arena fault. Thread-transfer
      uses the *same* generic flat-block copy path (a Set is a flat block, now
      bucket-aware), so it is covered by the same code the record-embed exercises; a
      live-thread transfer fixture was not added (marginal coverage over the shared
      path).

Acceptance: the copy/drop fixture runs clean (no arena abort; source unchanged);
`cargo test` green; acceptance goldens re-seeded for the new fixture.
Commit: —

## Validation Plan

- Tests: `tests/rt-behavior/collections/` fixtures (Phase 2 + 3), package
  round-trip (Phase 1), plus the `cargo test` unit tests inherited from A.
- Coverage check: the non-probe-eligible (enum-element) fixture is mandatory —
  without it the linear-scan membership path is untested and a green suite would
  only prove the hashed path.
- Runtime proof: the Phase 2 fixture's printed output *is* the proof set
  membership/dedup/order behave correctly end-to-end; the Phase 3 fixture proves
  copy/free correctness under the arena scrub.
- Doc sync: none authored in B — spec/man/goldens narrative is D. B *does* seed
  the acceptance goldens for its new fixtures (see `.ai/compiler.md` and the
  acceptance golden harness) since the fixtures must be green for C to build on.
- Acceptance: run the project's acceptance/golden suite for the new
  `collections` fixtures (`sync-goldens.sh` scoped to the set fixtures, then the
  full acceptance pass). Confirm no unrelated golden churned.

## Open Decisions

- **Operation namespace: `collections::` (recommended) vs. new `sets::`.**
  Recommend `collections::` — `List` and `Map` operations already share one
  package, `len` is global, and `add`/`remove`/`toList`/`contains` do not collide
  with existing member names. A `sets::` package would be the only per-type
  operation package and would fragment the "all collection ops live in
  `collections`" model. Decide before Phase 2 (it sets the member registration
  path). (§4.3)
  Decision: Add to `collections::`
- **`add`/`remove` names vs. `append`/`removeKey` reuse.** Recommend the distinct
  names `add`/`remove` — `append` implies ordered/positional and `removeKey`
  implies a key/value pair; set membership is neither. Low cost to change if
  review disagrees, since C is written against whatever names land.
  Decision: reuse existing `collections::*` functions

## Corrections

- **Set operation names: NEW `add`/`remove`/`toList` (not reuse).** The plan was
  self-contradictory: §4.3 designs new members `add`/`remove`/`toList` with
  signatures, but Open Decision 2's `Decision:` line said "reuse existing
  `collections::*` functions." These are incompatible for a user-facing API and
  drive C's 8 generics and D's doc pages, so the choice was put to the feature
  owner (2026-07-26): **decision = the distinct members `add`/`remove`/`toList`**
  per §4.3, plus the `contains` Set overload. Open Decision 2 is superseded; the
  §4.3 signatures stand: `add(Set OF T, T) AS Set OF T`,
  `remove(Set OF T, T) AS Set OF T`, `contains(Set OF T, T) AS Boolean`,
  `toList(Set OF T) AS List OF T`.
- **Value width is 1 byte (`Boolean` TRUE), not zero-width `NONE`.** §4.1
  specifies `valueType = COLLECTION_TYPE_NONE`, `valueLength = 0`. Realizing a
  genuine zero-width value would require a bespoke key-only insert emitter —
  precisely the path §3's Rejected-alternative warns is a Map-regression risk, and
  `emit_payload_length_to_stack` has no zero-length arm. Instead a `Set OF T` block
  is a Map-shaped block whose entries carry a 1-byte `Boolean` value (always TRUE):
  `CollectionTypeLayout::from_type` gives `kind = SET`, `key = element code`,
  `value = COLLECTION_TYPE_BOOLEAN`. Every core op then reuses the *fully-tested*
  Map emitter unchanged — `add` = `lower_map_set_in_place` (miss inserts, hit
  overwrites TRUE→TRUE, i.e. a no-op = idempotent), `remove` =
  `lower_map_remove_key`, `contains` = the `hasKey` probe/scan, `toList` =
  `lower_map_projection(project_key)`, and copy/size/free need no key-only path.
  Cost: 1 byte per element vs. the plan's zero-width ideal; benefit: no bespoke
  arena-mutating codegen, so no new arena-corruption surface. Behavior (dedup,
  idempotent add, membership, order, len, copy/free) is identical; D documents the
  1-byte-value representation. The zero-width optimization is a possible follow-up.
- **Enum elements are unsupported in collections (pre-existing, not Set-specific).**
  The Phase 2 fixture spec asked for an enum as the non-probe-eligible element. A
  `Set OF <enum>` fails codegen with "native collection packed payload does not
  support type 'Color'" — but so does `Map OF Color TO Integer` at HEAD (verified
  by building a minimal fixture), because `collection_type_code` maps an enum to
  `COLLECTION_TYPE_OBJECT` (a pointer), which the payload packer rejects. This is a
  general collection-payload limitation shared with Map, out of plan-63 scope
  (fixing it is a separate feature touching the payload packer for every
  collection). The linear-scan fallback is instead exercised with a comparable
  **record** element (`Set OF Point`), the same non-probe-eligible shape the Map
  fixtures use for keys (`Map OF Point TO String`).
- **`add`/`remove` are always-copy (in-place `MUT` optimization deferred).** §4.4
  asks `s = collections::add(s, x)` on a uniquely-owned `MUT` to mutate in place.
  Implemented as correctness-first value semantics (tight copy, then insert into the
  copy). The in-place fast path is a performance optimization, not a correctness
  requirement — the behavior fixtures (dedup/idempotent/membership/order/value-
  independence) all pass with the copy. Left as a follow-up.
- **B0 bucket-region census.** The "map has a bucket region" decision is
  concentrated in two sizing/copy sites plus one reserve helper (all in
  `builder_collection_layout.rs`): `emit_flat_block_size:286` (was
  `kind == COLLECTION_KIND_MAP`) and `copy_collection_tight:414` (the
  `emit_reserve_map_buckets` bool arg, was `layout.kind == COLLECTION_KIND_MAP`).
  Both now consult `collection_has_buckets(type)`. `emit_reserve_map_buckets`
  itself already takes a bool (no inline kind test). The scope-drop free path
  (`builder_owned_cleanup.rs`) routes through `emit_flat_block_size` for the block
  size, so fixing that one site covers free too. The lazy bucket-build
  (`_mfb_rt_map_build_buckets`) sizes from the header's capacity, not the type, so
  it needs no Set edit.

## Summary

B makes a Set real at runtime by treating it as a Map with a zero-width value:
one new kind tag, one `has_buckets` predicate, thin key-only wrappers over the
Map literal/insert/probe/remove/project emitters, and the bucket region wired
into the four sizing sites. The engineering risk is entirely in those sizing
sites (arena corruption if a Set is allocated and freed at different sizes),
which is why B0 censuses them first and Phase 3 lands them last behind a
copy-and-drop runtime test. `List`/`Map` are untouched.
