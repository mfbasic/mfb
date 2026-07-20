# plan-57-D: `kind = 2` — drop the lookup table for fixed-width lists

Last updated: 2026-07-19
Effort: medium (1h–2h)
Depends on: plan-57-A, plan-57-B (containment), plan-57-C (the order invariant)

The payoff. A list whose element type is a fixed-width scalar gets a second
block representation carrying **no lookup table at all**:

```
kind = 0 (List)                       kind = 2 (fixed-width List)
  CollectionHeader   40                 CollectionHeader   40
  LookupEntry[cap]   40 each            Data[dataCapacity]
  Data[dataCapacity]
```

`List OF Byte` goes from **41 bytes per byte to 1**. `dataBase` becomes a
constant `block + 40` instead of `block + 40 + capacity*40`, and element access
loses two dependent loads.

This is an **implementation detail, not a new type.** The type system, the
resolver, the spec's type table, and every MFBASIC program see `List OF Byte`
exactly as before. Representation is chosen by the emitter from the element type,
statically.

The single behavioral outcome: every existing program produces identical results,
and a `List OF Byte` of N elements allocates `40 + N` bytes instead of
`40 + 41N`.

References (read first):

- `planning/plan-57-C-fixed-lists-maintain-order.md` — establishes
  `entry[i].valueOffset == i * payloadSize`, machine-checked in its Phase 3. This
  sub-plan deletes the entries **on the strength of that check**; do not start
  until it passes.
- `planning/plan-57-A-*.md`, `planning/plan-57-B-*.md` — the helpers this edits.
  If the containment held, this sub-plan touches a handful of functions.
- `src/docs/spec/memory/05_collections.md:24-100` — the layout to amend;
  `:52-55` states "Version 1 uses a 40-byte `CollectionHeader` and 40-byte
  `LookupEntry`" and must gain the kind-2 exception.
- `src/target/shared/code/error_constants.rs:762-763` —
  `COLLECTION_KIND_LIST = 0`, `COLLECTION_KIND_MAP = 1`. `2` is the next free
  value.
- `src/target/shared/code/builder_collection_layout.rs:197-249`
  (`emit_flat_block_size`) — the **free path**. Getting this wrong corrupts the
  arena free list; that is bug-02, recorded at `:236-243`.
- `:310-448` (`copy_collection_tight`) — two verbatim block copies.
- `src/target/shared/code/builder_arena_transfer.rs:531-547`
  (`collection_needs_transfer_fix`) — compile-time, and false for these element
  types, so thread transfer is a plain `copy_flat_block`.
- `.ai/compiler.md` — Hard Completion Gate.

## 1. Goal

- `COLLECTION_KIND_LIST_FIXED = 2`, written into the header of every list whose
  element type is `Byte`, `Boolean`, `Scalar`, `Integer`, `Float`, `Fixed` or
  `Money`.
- Such a block carries **no `LookupEntry` array**. Layout is
  `CollectionHeader(40) + Data[dataCapacity]`, `dataBase = block + 40`, element
  `i` at `dataBase + i * payloadSize`.
- Header fields keep their meanings: `count` and `capacity` count **elements**;
  `dataLength = count * payloadSize`, `dataCapacity = capacity * payloadSize`.
- `emit_flat_block_size` returns `HEADER + dataCapacity` for `kind = 2`, so
  allocation, copy and free all agree.
- No user-visible change: no new type, no new type-string, no resolver change,
  no diagnostic, no `.mfp` format change.
- Every existing test passes with identical observable behavior.

### Non-goals (explicit constraints)

- **`kind` stays non-load-bearing at runtime.** It is written for
  self-description; dispatch is static, from the element type, in the emitter.
  Making generated code branch on `kind` would add a load-and-branch to every
  access that currently needs none — the opposite of the win. (Verified: `kind`
  has 25 references across 17 files today, **all stores, zero loads**, as do
  `flags` and `flagsVersion`.)
- **No new user-facing type.** There is no `FixedList`, no new type-string, no
  new spec type-table row. `List OF Byte` is one type with two representations.
- **Maps and variable-width lists are untouched.** `String`, records, unions and
  nested collections keep `kind = 0` and the entry table.
- **No `.mfp` format change.** The package format never encodes a runtime
  collection block — a list literal is an `IrValue::ListLiteral`
  (`src/ir/binary.rs:1292-1296`), and every `COLLECTION_*` constant lives under
  `src/target/shared/code/`. `BINARY_REPR_VERSION` is unchanged.
- Do not widen the element set to function values or pointer payloads here
  (plan-57-E).

## 2. Current State

After plan-57-A/B/C:

- "Where is element `i`" is `emit_element_value_offset` / `emit_element_address`
  in `builder_collection_layout.rs`.
- List construction is `emit_alloc_list`.
- List iteration is the `emit_list_iteration_*` trio.
- The data base is `emit_collection_data_pointer` /
  `emit_collection_data_pointer_into`.
- Fixed-width lists satisfy `entry[i].valueOffset == i * payloadSize` after every
  operation, machine-checked.

So the entry table is, for these element types, **provably redundant** — every
field is either a constant (`flags = USED`, `keyOffset = 0`, `keyLength = 0`,
`valueLength = payloadSize`) or derivable (`valueOffset = i * payloadSize`).

The one runtime read of `flags` is `builder_arena_transfer.rs:727-735`, a guard
that skips non-`USED` entries. Nothing in the tree ever clears the `USED` bit —
`removeAt` compacts rather than tombstoning, and all 32 references to
`COLLECTION_ENTRY_FLAG_USED` are stores of `1` or comparisons against `1`. There
are no tombstones in this codebase. The guard is dead, and it is on a path
(`collection_needs_transfer_fix`) that is false for fixed-width scalars anyway.

## Status (2026-07-19): BLOCKED on plan-57-B

plan-57-C's Phase 3 gate is **met** — `list-order-invariant-rt` passes 300 mixed
mutation steps with 0 violations and is proven non-vacuous by a negative control
— so the *invariant* precondition is satisfied.

But §3's six edits are stated against helpers plan-57-A/B were to have created,
and **three of the six do not exist**, because plan-57-B is only partially
landed:

| §3 edit site | exists? |
|---|---|
| `error_constants.rs` (the kind constant) | n/a — trivial |
| `emit_element_value_offset` | **yes** (plan-57-A) |
| `emit_collection_data_pointer` | **yes** (pre-existing) |
| `emit_collection_data_pointer_into` | **MISSING** — plan-57-B track 3 |
| `emit_alloc_list` | **MISSING** — plan-57-B track 2 |
| `emit_list_iteration_*` | **MISSING** — plan-57-B track 1 |
| `emit_flat_block_size` | **yes** (pre-existing) |

Starting D now would mean making the representation change at each
*un-consolidated* site by hand — the 90-site sweep that plan-57-A and -B exist
specifically to prevent, and with a guard that cannot see most of those sites
(`audio/`, `net/`, `tls/`, `crypto*`, `os`, `fs_helpers*` have **no** golden
coverage; see plan-57-A §Findings). Given that §3 names `emit_flat_block_size`
as the one edit whose failure mode is *heap corruption* rather than wrong data,
that is not a trade worth making.

**Do plan-57-B's remaining tracks first.** Its scope is now measured rather than
estimated, and is smaller than §2 assumed: **13** open-coded constructors across
10 files (not "~30"), of which 3 are already folded into one shared builder.
Note the same over-estimate appeared in plan-57-A, where "38 indexed read sites"
turned out to be 2 convertible ones — re-measure before scheduling.

## 3. Design Overview

Six edits, each in a function plan-57-A/B created or already owned:

| what | where |
|---|---|
| the kind constant | `error_constants.rs` |
| element address: `i * p` instead of an entry load | `emit_element_value_offset` |
| data base: `block + 40` instead of `+ capacity*40` | `emit_collection_data_pointer{,_into}` |
| allocation: no entry region, no entry-fill loop | `emit_alloc_list` |
| iteration: stride the data region by `p` | `emit_list_iteration_*` |
| block size (alloc/copy/free) | `emit_flat_block_size` |

plus the mutation paths' entry rewrites (plan-57-C §4.2 step 4) becoming no-ops,
and `copy_collection_tight` copying one region instead of two.

**Where the correctness risk concentrates: `emit_flat_block_size`.** It is the
size used by `arena_free` on scope drop. If it reports the kind-0 size for a
kind-2 block, the allocator frees a region larger than the block and corrupts the
free list — precisely bug-02, whose comment still sits at
`builder_collection_layout.rs:236-243` describing the last time this exact
function was wrong by a region. Every other error in this sub-plan produces wrong
data; this one produces heap corruption.

Second risk: **a fixed-width list embedded in a variable-width container.**
A `List OF List OF Integer` has variable-width elements (inlined blocks), so the
outer list keeps `kind = 0` and its `valueLength` must be the *inner* block's size
— now `HEADER + dataCapacity`, not `HEADER + cap*ENTRY + dataCapacity`. That flows
through `emit_payload_length_to_stack` (`:1500-1546`) and
`emit_inlined_block_size_from_ptr_slot` (`:625`). Both call `emit_flat_block_size`,
so getting that one function right fixes both — but it must be *tested* through a
nested-collection fixture, not assumed.

**Rejected alternative:** *make `kind` a runtime discriminator and branch in
generated code.* Rejected — see Non-goals. Static dispatch is available because
MFBASIC is monomorphized, and it costs nothing.

**Rejected alternative:** *keep a 8-byte entry holding only `valueOffset`.*
Rejected: after plan-57-C, `valueOffset` is exactly `i * payloadSize`, so an
8-byte entry stores a value the consumer can compute with one multiply. It would
be 9× rather than 1× for `List OF Byte` and would keep every entry-walking loop
alive for no benefit.

**Rejected alternative:** *drop `dataLength`/`dataCapacity` too, since they are
`count`/`capacity` × `payloadSize`.* Rejected: the header is a fixed 40 bytes
either way, so removing them saves nothing and would make the two kinds' headers
structurally different, which is exactly the complexity this design avoids. Keep
all four fields consistent.

## 4. Detailed Design

### 4.1 The constant and the predicate

```rust
/// A `List` whose element type is a fixed-width scalar: no `LookupEntry` array,
/// payloads packed at `HEADER + i * payloadSize` in index order (plan-57).
///
/// This is a representation, not a type. Source-level `List OF Byte` is one
/// type; the emitter picks the block shape from the element type. `kind` is
/// written for self-description only — dispatch is static, and no generated
/// code loads this field.
pub(crate) const COLLECTION_KIND_LIST_FIXED: usize = 2;
```

Representation choice is `list_element_is_fixed_width(element_type)` from
plan-57-C §4.1 — one predicate, already drift-tested against
`collection_payload_alignment`.

### 4.2 Layout

| field | kind 0 | kind 2 |
|---|---|---|
| `count` | elements | elements |
| `capacity` | lookup slots | elements |
| `dataLength` | used data bytes | `count * payloadSize` |
| `dataCapacity` | allocated data bytes | `capacity * payloadSize` |
| entries | `capacity * 40` bytes | **absent** |
| `dataBase` | `+40 + capacity*40` | `+40` |
| block size | `40 + cap*40 + dataCapacity` | `40 + dataCapacity` |

Alignment: `dataBase` is `block + 40`, and 40 is 8-aligned, so an 8-byte payload
stays 8-aligned. `Scalar` (4) and `Byte`/`Boolean` (1) are trivially satisfied.
No padding is needed — `list_element_padding_alignment` already returns `1` for
all seven types.

Capacity headroom (`05_collections.md:173-198`) is unchanged in meaning: `capacity
> count` is still legal, still means spare element slots, and `dataBase` is still
capacity-independent — in fact now trivially so.

### 4.3 `emit_flat_block_size`

```
kind 2:  size = COLLECTION_HEADER_SIZE + dataCapacity
```

It already reads header words only, so this is a branch on the static type, not
new machinery. **Add a unit test asserting the size matches what `emit_alloc_list`
allocated**, for each of the three payload widths and for a nested case. The
allocation size and the free size are computed by different functions; that
divergence is what bug-02 was.

### 4.4 What becomes dead

- The entry-fill loop in `emit_alloc_list` — skipped for kind 2.
- The entry rewrite in plan-57-C §4.2 step 4 — no entries to rewrite.
- `copy_collection_tight`'s entry block copy — one region instead of two.
- `mid`'s order probe (`builder_search.rs:876-908`) for fixed-width lists — the
  fast path is now unconditional. Leave the `String` path; plan-57-E cleans up.
- `builder_arena_transfer.rs:727`'s `flags` guard for these types — already
  unreachable via `collection_needs_transfer_fix`.

Delete what is genuinely dead; do not leave it behind a comment promising a later
phase (`AGENTS.md` is explicit, and bug-326 catalogued a dozen such promises that
had gone stale).

## Compatibility / Format Impact

- **Changes:** the in-memory block layout of fixed-width lists, and their
  allocation size. `List OF Byte` drops from `40 + 41N` to `40 + N`.
- **Changes:** `src/docs/spec/memory/05_collections.md` — a new kind, the kind-2
  layout, and an amendment to `:52-55`, which currently states the 40-byte
  `LookupEntry` unconditionally.
- **Unchanged:** every MFBASIC-visible semantic; the type system and type-string
  grammar; the `.mfp` format and `BINARY_REPR_VERSION`; all Map behavior; all
  variable-width list behavior; every diagnostic and rule.
- Golden churn is expected wherever codegen for a fixed-width list appears.
  Review each as a real diff.

## Phases

### Phase 1 — the constant, the size, and the spec — **IN PROGRESS**

Approach changed from §3's six independent edits to a single lever: the layout
formulas are all `HEADER + capacity * <stride>`, so `list_entry_stride` returning
**0** for a fixed-width element collapses every one of them to the kind-2 layout
without the formula changing shape — and makes it impossible for the alloc size,
the free size and the data base to disagree (which is bug-02's failure mode).

`KIND2_ENABLED = false` gates it while every site is threaded onto the two
predicates; the flip is then one commit. `MFB_KIND2=1` env-gates it during development so ONE binary can be exercised
both ways — the negative-control lever that proved `list-order-invariant-rt`.

**Done:** the constant; `list_entry_stride` / `list_block_kind`;
`CollectionTypeLayout::from_type` as the single representation choice; all 67
data-base sites threaded (and the untyped entry point made private, so a missed
site is a compile error); literal allocation sized by the stride; literal entry
stores skipped for kind 2.

**Status (end of session): the representation is live behind `MFB_KIND2=1` and
the full acceptance suite is down to ONE real failure.**

Kind-2 acceptance went 38 mismatches -> 2, of which one is the expected `.ncode`
churn on `list-ops-codegen-rt` (that anchor exists precisely to make this diff
visible). Working under the flag: literals, `get`/`getOr`, `FOR EACH`, `sum`,
`contains`, `find` (element and sublist), `distinct`, `append` (single and bulk),
`prepend`, `insert`, `removeAt`, value-`set`, in-place `set`, `mid`, `take`,
`drop`, `slice`, `sort`, `filter`, `replace`, `keys`/`values`, the `math::` array
kernels, `strings::toBytes`, and the fs/tls/audio/crypto/net byte-list helpers.

Converted along the way, each because it silently corrupted or leaked:
`emit_flat_block_size` (the free size — bug-02's failure mode),
`emit_copy_payload_to_collection`, `copy_collection_tight`,
`lower_simd_alloc_list`, `lower_map_projection`, `lower_list_replace`, and the
entry-fill loops in every byte-list runtime helper.

A structural lesson worth keeping: the data-base stride must be selected by the
BLOCK KIND, never inferred from the payload type. A `Map OF Scalar TO T` has a
fixed-width key and still keeps its entries. That is now an explicit
`stride_type` parameter (`""` for a map) on the payload loader, the three compare
helpers and the payload copier.

**The one remaining failure:** `encoding::base64Encode` / `base32Encode` leak
under kind 2 — a single call is correct (`Zg==`), but 3000 calls exhaust the
arena. `base64UrlEncode` does not leak, and the difference between them is
`pad = TRUE`. An inline replica of `__encoding_baseEncode` does NOT leak, so the
leak is specific to the bundled-package call path rather than the algorithm.
Not yet root-caused.

**Also outstanding:** the spec amendment, the golden re-baseline, and Phase 3's
proofs (the memory win, nested `List OF List OF Integer`, thread transfer).


No representation is produced yet; this teaches the size path to describe one.

- [x] Add `COLLECTION_KIND_LIST_FIXED = 2` (`error_constants.rs`) with the doc
      comment from §4.1.
- [x] Teach `emit_flat_block_size` (`builder_collection_layout.rs:197-249`) the
      kind-2 arm.
- [ ] Amend `src/docs/spec/memory/05_collections.md`: the kind table, the kind-2
      layout block, the `:52-55` entry-size statement, and a note that the choice
      is an implementation detail invisible to source. Cite
      `[[src/target/shared/code/error_constants.rs:COLLECTION_KIND_LIST_FIXED]]`.
  **NOT DONE** — deliberately last: the spec should describe the
      representation that ships, and the flag is still off.
- [ ] Tests: unit tests asserting `emit_flat_block_size` agrees with what
      `emit_alloc_list` would allocate, for payload widths 1/4/8, kind 0 and
      kind 2, and a nested `List OF List OF Integer`.

Acceptance: the size/alloc agreement tests pass; `artifact-gate` byte-identical
(nothing produces kind 2 yet); `cargo test --bin mfb spec` green with no leaked
`[[` markers.
Commit: —

### Phase 2 — flip the representation

The single behavioral commit. Small, because plan-57-A/B did the fan-out.

  **NOT DONE.** This is the highest-value remaining test — alloc size vs
      free size disagreeing is bug-02, and this sub-plan hit exactly that.
- [x] `emit_element_value_offset`: fixed-width arm returns `index * payloadSize`
      and the constant `payloadSize`, no entry load.
- [x] `emit_collection_data_pointer{,_into}`: fixed-width arm returns
      `block + HEADER`.
- [x] `emit_alloc_list`: fixed-width arm sizes `HEADER + count * payloadSize`,
      writes `kind = 2`, skips the entry region and the entry-fill loop.
- [x] `emit_list_iteration_*`: fixed-width arm strides the data region by
      `payloadSize`.
- [x] Mutation paths: drop the now-vacuous entry rewrites.
- [x] `copy_collection_tight`: one region copy for kind 2.
- [ ] Delete the dead code from §4.4.

Acceptance: the whole existing suite passes with **identical observable
behavior**; a `List OF Byte` of 1,000,000 elements allocates ~1 MB rather than
~41 MB (assert via `os::` memory introspection or an arena probe, not by
eyeballing); on macOS/aarch64 and Linux/{aarch64,x86_64,riscv64}.
Commit: —

### Phase 3 — prove the win and the absence of loss

  **NOT DONE** — nothing is dead yet; both arms are live while the flag
      selects between them. This belongs with flipping the flag to `true`.
- [ ] Memory: a runtime test allocating a large `List OF Byte`, `List OF Integer`
      and `List OF Scalar` and asserting the block size matches `40 + N*p`.
  **NOT DONE** — this is the proof of the payoff (40 + N vs 40 + 41N) and
      has not been measured even once.
- [ ] Performance: benchmark `get`, `FOR EACH`, `append`, `prepend` over each
      payload width, before and after (`benchmark/`). Expect improvements
      everywhere; `get` loses two dependent loads and `prepend` moves `p` bytes
      per element instead of 40.
  **NOT DONE.**
- [ ] Nested: a `List OF List OF Integer` fixture exercising construction, read,
      copy, and drop — the inlined-block size path (§3) is the subtlest
      interaction and has no other coverage.
  **NOT DONE** — §3 calls this out as the second risk concentration
      (a fixed-width list inlined in a variable-width container), and it is
      untested.
- [ ] Thread transfer: transfer a `List OF Integer` between threads and assert
      contents, exercising `copy_flat_block` over a kind-2 block.
  **NOT DONE.**

Acceptance: memory assertions hold on every target; no benchmark regression; the
nested and thread-transfer fixtures pass.
Commit: —

## Validation Plan

- Tests: the size/alloc agreement unit tests (Phase 1); the memory, nested, and
  thread-transfer fixtures (Phase 3); the whole existing collection suite as the
  non-regression assertion.
- Runtime proof: **required (Hard Completion Gate).** Two claims need it — that
  behavior is unchanged (the existing suite, on every target) and that the
  memory actually dropped (a measured allocation, not an inferred one).
- Free-path proof: run the collection suite under whatever arena-integrity
  checking exists, and add a fixture that allocates and drops many fixed-width
  lists in a loop. A wrong `emit_flat_block_size` corrupts the free list silently
  and may only surface much later — this is the failure mode that most needs a
  deliberate test rather than incidental coverage.
- Doc sync: `src/docs/spec/memory/05_collections.md` (Phase 1). No rule or
  error-code change, so `01_rule-codes.md` and `02_error-codes.md` are untouched.
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual`,
  with churn reviewed rather than re-baselined.

## Open Decisions

- **Assert `kind` is never loaded, mechanically?** It is write-only today and
  this design depends on that staying true. Recommend a unit test grepping the
  emitters for a load of `COLLECTION_OFFSET_KIND` — cheap, and it documents the
  invariant where a future contributor will trip over it. Alternative: rely on
  review, which is how `flagsVersion` quietly became write-only in the first
  place.
- **Should `capacity` for kind 2 mean elements or bytes?** Recommend elements, as
  specified in §4.2 — it keeps `count`/`capacity` comparable and the headroom
  rules unchanged. Bytes would make `dataCapacity` redundant but would break every
  `count < capacity` comparison.
  Decision: Elements

## Summary

If plan-57-A/B/C did their jobs, this is six function edits and a spec change.
The risk is concentrated almost entirely in `emit_flat_block_size`: every other
mistake here yields wrong data, that one yields a corrupted arena free list, and
that function has been wrong by exactly one region before (bug-02). Hence a unit
test pinning allocation size against free size, and a deliberate allocate-and-drop
fixture rather than incidental coverage.

Untouched: the type system, the `.mfp` format, maps, and variable-width lists.
bug-365 remains open for `List OF String`.
