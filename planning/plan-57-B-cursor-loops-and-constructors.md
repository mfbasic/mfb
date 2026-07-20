# plan-57-B: convert cursor-stride loops and consolidate the list constructors

Last updated: 2026-07-19
Effort: medium (1h–2h)
Depends on: plan-57-A (the element-access helpers)

The second half of the containment work. plan-57-A covered *indexed* reads; two
larger populations remain, and both bake the entry table into their control flow
rather than into an address computation:

- **~20 cursor-stride loops** hold a raw entry pointer and advance it by
  `COLLECTION_ENTRY_SIZE` per iteration. They cannot call
  `emit_element_address(index)` because they have no index.
- **~30 construction sites** — every runtime helper that builds a `List OF Byte`
  or `List OF String` — open-code the 40-byte header write plus an N-iteration
  entry-fill loop, each with its own copy of the size arithmetic.

Both must be routed through shared code before plan-57-D can change the
representation, and both must land **byte-identically**.

The single behavioral outcome: nothing changes. `scripts/artifact-gate.sh` is
byte-identical; `scripts/test-accept.sh` is green with zero golden churn.

References (read first):

- `planning/plan-57-A-element-access-containment.md` — the helpers this builds on
  and the byte-identity discipline it inherits.
- `src/target/shared/code/builder_control.rs:1201` (`lower_for_each`) — the
  canonical cursor loop: cursor initialized to the entry base (`:1266-1270`),
  advanced by `COLLECTION_ENTRY_SIZE` (`:1387`), payload loaded from `:1366, 1371`.
- `src/target/shared/code/builder_collection_queries.rs:1985-2120` — the three
  shared loop helpers that `transform`/`filter`/`reduce` already use
  (`initialize_collection_loop_slots`, `load_collection_loop_item`,
  `advance_collection_loop`). **The precedent: three of the higher-order ops are
  already consolidated.** This sub-plan extends that pattern to the rest.
- `src/target/shared/code/audio/alsa.rs:1273-1326` (`emit_alloc_byte_list`), its
  verbatim twin at `audio/macos.rs:1546`, and the near-variant
  `crypto_ec.rs:183-245` (`emit_build_byte_list`) — three copies of one routine.
- `src/target/shared/code/error_constants.rs:762-779` — the constants.
- `bugs/bug-333-string-collection-builder-duplication.md` — the standing
  duplication record; this sub-plan retires part of it.
- `bugs/bug-365-linear-data-region-readers-ignore-entry-order.md` §Scope — the
  unverified worklist of linear consumers overlaps heavily with the construction
  sites below. Auditing them here is cheap; **do not fix them here** (that is
  bug-365's own change), but record what you find.
- `.ai/compiler.md` — register lifetimes: `_mfb_arena_alloc` destroys `x0`-`x17`,
  and every constructor below allocates.

## 1. Goal

- A shared `emit_list_iteration_*` trio covering every cursor-strided List walk,
  modeled on the existing `builder_collection_queries.rs:1985-2120` helpers, with
  `element_type` threaded through for plan-57-D.
- A single shared `emit_alloc_list(element_type, count, ...)` replacing the three
  copies of `emit_alloc_byte_list` and the ~30 open-coded header+entry writers.
- All ~32 open-coded `header + capacity * entrySize` data-base computations
  routed through `emit_collection_data_pointer`, or — where no `CodeBuilder`
  exists — through a free-function equivalent.
- **Byte-identical output** at every step.
- A recorded audit of which construction/consumption sites read the data region
  linearly (bug-365's worklist), as a by-product of touching all of them.

### Non-goals (explicit constraints)

- **No behavior change, no bug fixes.** Sites that look wrong get *recorded*, not
  corrected — a fix riding inside a byte-identity refactor destroys the guard that
  makes the refactor safe. bug-365 is fixed in its own change.
- **Map paths untouched.** The Map arms of `lower_for_each` (`:1296-1328`) and
  every `lower_map_*` stay exactly as they are.
- No change to the block layout, any constant, or any allocation size.
- Do not merge `emit_build_byte_list`'s copy-from-source behavior into the shared
  allocator; keep it a thin wrapper. The allocator allocates, the wrapper copies.

## 2. Current State

**Cursor loops.** `lower_for_each` (`builder_control.rs:1201`) sets its cursor to
`collection + COLLECTION_HEADER_SIZE` and adds `COLLECTION_ENTRY_SIZE` per step,
loading `valueOffset`/`valueLength` from the cursor each iteration. So a
`FOR EACH x IN listOfInteger` performs two dependent 8-byte loads from a
40-byte-strided table, plus a data-region add, to reach an 8-byte payload.

The same shape recurs at roughly twenty sites — `builder_collection_queries.rs:174,
969-971, 1121, 1310, 2092`, `builder_collection_query.rs:460, 645`,
`builder_strings.rs:399-404` (`lower_list_replace`),
`builder_strings_package.rs:279`, `entry_and_arena.rs:714, 1447`,
`fs_helpers_paths.rs:1115, 1693, 1769`, `os.rs:951, 1980`,
`fs_helpers_io.rs:2102`, `crypto_ec.rs:172, 240`.

Three are already consolidated: `transform`, `filter` and `reduce` share
`initialize_collection_loop_slots` / `load_collection_loop_item` /
`advance_collection_loop` (`builder_collection_queries.rs:1985-2120`). That trio
is the template — the work is extending it, not inventing it.

**Constructors.** `emit_alloc_byte_list` exists three times
(`audio/alsa.rs:1273`, `audio/macos.rs:1546`, and as `emit_build_byte_list` at
`crypto_ec.rs:183`), and the header+entry-fill idiom is open-coded in ~30 runtime
helpers across `os.rs`, `fs_helpers_{io,paths,atomic}.rs`, `net/io.rs`,
`tls/{openssl,macos}.rs`, `audio/{alsa,macos}.rs`, `crypto{,_ec}.rs`,
`entry_and_arena.rs`, `builder_strings*.rs`. Each writes:

```
kind, keyType, valueType, flagsVersion       (4 × store_u8)
count, capacity, dataLength, dataCapacity    (4 × store_u64)
for i in 0..N: flags=USED, keyOffset=0, keyLength=0, valueOffset=i, valueLength=1
```

and computes `size = HEADER + N*ENTRY + N` with its own arithmetic.

**Data-base computations.** `emit_collection_data_pointer`
(`builder_collection_layout.rs:1725`) has ~40 callers, but **32 sites open-code
the same math** because they run inside standalone `CodeFunction` runtime helpers
where the `CodeBuilder` is unavailable. That structural split is the reason a
single helper has not already absorbed them, and it is the thing this sub-plan
must solve — with a free function, not by contorting the builder method.

## 3. Design Overview

Three independent tracks, landable in any order, each byte-identical:

1. **Iteration.** Generalize the existing loop trio to cover every List cursor
   walk. Signature gains `element_type`.
2. **Construction.** One `emit_alloc_list`, one size calculation, one entry-fill
   loop. Both a `CodeBuilder` method and a free-function form, sharing an inner
   implementation, so the ~30 runtime helpers can use it.
3. **Data base.** A free-function `emit_collection_data_pointer_into` that the
   builder method delegates to, so the 32 open-coded sites can call it.

**Where the risk concentrates:** the construction track, for two reasons. First,
every constructor allocates, and `_mfb_arena_alloc` destroys all caller-saved
registers — so consolidating them means consolidating their *spill discipline*,
and a helper that spills differently than an inlined sequence produces different
bytes at best and a clobber bug at worst. Second, `.ai/compiler.md` is explicit
that this bug class *passes small tests* and only faults past a threshold, so
byte-identity is doing more work here than the test suite is.

**Rejected alternative:** *convert cursor loops into indexed loops and reuse
plan-57-A's `emit_element_address`.* Tempting — it would collapse both
populations into one helper. Rejected: an indexed loop recomputes
`index * ENTRY_SIZE` every iteration where the cursor form does one add, so the
emitted code would differ and byte-identity would be lost. Keep the cursor form;
plan-57-D changes what the cursor strides by, not that there is one.

**Rejected alternative:** *skip the ~30 constructors and let plan-57-D update them
individually.* Rejected — they are the sites that must stop writing an entry
table, so leaving them open-coded moves the 30-site sweep into the commit that
also changes the layout, which is exactly what plan-57-A/B exist to prevent.

## 4. Detailed Design

### 4.1 Iteration

```rust
/// Initialize a List walk: cursor to the first entry, bound to `count`.
/// `element_type` is unused today; plan-57-D strides the data region directly
/// for fixed-width scalars, where there are no entries to walk.
fn emit_list_iteration_begin(&mut self, ..., element_type: &str)
fn emit_list_iteration_load(&mut self, ..., element_type: &str)
fn emit_list_iteration_advance(&mut self, ..., element_type: &str)
```

Bodies are lifted verbatim from `builder_collection_queries.rs:1985-2120`. Convert
`lower_for_each`'s **List arm only**, leaving the Map arm inline — the two arms
diverge in plan-57-D and forcing them into one helper now would have to be undone.

### 4.2 Construction

```rust
/// Allocate a `List OF <element_type>` with `count` elements: header written,
/// entries filled, payload left uninitialized. Size and layout in one place.
fn emit_alloc_list(element_type: &str, count: ..., ...) -> Result<(), String>
```

Size: `COLLECTION_HEADER_SIZE + count * COLLECTION_ENTRY_SIZE + count *
payloadSize`, align 8. Header: `kind`, `keyType = 0`, `valueType`,
`flagsVersion = 1`, and `count`/`capacity`/`dataLength`/`dataCapacity`. Entry
loop: `flags = USED`, keys `0`, `valueOffset = i * payloadSize`,
`valueLength = payloadSize`.

Note the existing three copies hardcode `payloadSize = 1` (they are byte-list
specific). Generalizing to `element_type` is required for plan-57-D and must be
done **without** changing what the byte-list callers emit — pass `"Byte"` and the
arithmetic must fold to the same instructions. Verify with `artifact-gate`; if
multiplying by a literal `1` emits an extra instruction, special-case it.

`crypto_ec.rs:183`'s `emit_build_byte_list` becomes a thin wrapper: call
`emit_alloc_list`, then its existing copy loop.

### 4.3 Data base

```rust
/// `list + COLLECTION_HEADER_SIZE + capacity * COLLECTION_ENTRY_SIZE`.
/// Free-function form for standalone `CodeFunction` runtime helpers, which have
/// no `CodeBuilder`. The builder method delegates here.
fn emit_collection_data_pointer_into(
    dst: &str, list: &str, scratch: &str,
    instructions: &mut Vec<CodeInstruction>,
)
```

Convert all 32 open-coded sites (the table in the read-path audit; start from
`grep -n "COLLECTION_ENTRY_SIZE" src/target/shared/code/ | grep -v builder_`).

**While converting each site, record whether it then reads the data region
linearly.** That is bug-365's unverified worklist — `audio::write`, the `net`/
`tls` writes, `fs` byte IO, `crypto`. Write findings into the bug, fix nothing.

### 4.4 `lower_list_replace` — all three tracks in one function

`builder_strings.rs:306-620` is the only site that belongs to every track in this
sub-plan, which is why it is called out separately rather than buried in a list.
It was mis-filed as an indexed read in an early draft of plan-57-A (and
mis-cited to `builder_search.rs`); it is neither.

It makes **two** cursor passes over the source list, each seeding a cursor at
`collection + COLLECTION_HEADER_SIZE` and advancing by `COLLECTION_ENTRY_SIZE`:

- a **length-measuring** pass (`:366-405`) that loads each entry's
  `valueOffset`/`valueLength`, calls
  `emit_collection_payload_matches_value_branch`, and accumulates either the
  replacement's length or the original's — so the output size depends on how many
  elements match;
- a **copy** pass (`:540+`) that walks the same entries again and emits either the
  new or the old payload.

It is also a **constructor**: it sizes `count*ENTRY + HEADER + data_len` through
`emit_checked_size_multiply`/`emit_checked_size_add_immediate` (`:413-422`) and
writes a fresh header and entry table.

Three consequences for this sub-plan:

1. It needs the iteration trio (Phase 3) **and** `emit_alloc_list` (Phase 2), so
   it cannot be finished until both exist. Convert it last.
2. Its size arithmetic is overflow-checked, and the comment at `:419-420` records
   why (**bug-60**: an unchecked multiply undersized the allocation and the copy
   pass overran it). `emit_alloc_list` must preserve that checking, not just the
   arithmetic — if the shared helper computes sizes unchecked, this site must keep
   its own guard rather than lose it to consolidation.
3. Its output length is data-dependent, so it is the one constructor whose
   `count` is not known before the first pass. `emit_alloc_list` must accept a
   runtime count, which it does — but verify that before converting, since every
   other caller passes something statically derivable.

Under plan-57-D this function gets materially simpler for fixed-width elements:
both passes become data-region strides and the entry table disappears. Do not
attempt that simplification here.

### 4.5 `lower_simd_alloc_list` — resolved, and a good first conversion

An earlier draft of this plan flagged `entry_and_arena.rs:1398-1401` as striding
by `ENTRY_SIZE + 8` and called it anomalous. **It is neither anomalous nor a
stride.** Reading it in full:

```rust
// alloc size = COLLECTION_HEADER_SIZE + count*(ENTRY_SIZE + 8) (lookup + data).
move_immediate(stride, COLLECTION_ENTRY_SIZE + 8);
multiply_registers(ARG[0], count, stride);
add_immediate(ARG[0], ARG[0], COLLECTION_HEADER_SIZE);
```

That is `HEADER + count*ENTRY + count*8` — the ordinary block size for a list of
8-byte payloads, factored as one multiply instead of two. The **entry** stride in
the fill loop is a plain `add_immediate(entry, entry, COLLECTION_ENTRY_SIZE)`,
exactly like every other constructor. Nothing to investigate; the name `stride`
on the local vreg is what made it look otherwise.

It is a good **early** conversion for the Phase 2 track, for three reasons:

1. It is a standalone `CodeFunction` runtime helper (symbol
   `SIMD_ALLOC_LIST_SYMBOL`), not a `CodeBuilder` method — so it exercises the
   free-function form of `emit_alloc_list` that the ~30 `os`/`fs`/`net`/`tls`/
   `audio`/`crypto` helpers need, on a small self-contained function.
2. It is already the cleanest constructor in the tree: one allocation, a header
   write, and one entry loop, with the arena clobber discipline handled by vregs
   rather than by hand.
3. It hardcodes the payload width at 8 (`shift_left_immediate(data_len, count, 3)`,
   `valueLength = 8`, `value_off += 8`) while taking `valueTypeCode` as a
   **runtime** argument. So it is generic over *which* 8-byte type
   (`Integer`/`Float`/`Fixed`/`Money`) but not over the width — which is precisely
   the shape `emit_alloc_list(element_type, ...)` generalizes. Confirm the
   converted form still emits a shift rather than a multiply for width 8, or it
   will not be byte-identical.

Under plan-57-D this function nearly disappears: `kind = 2` removes the entry
loop entirely, the size becomes `HEADER + count*8`, and its doc comment's data
region — *"`count` contiguous 8-byte lanes at `base + 40 + count*40`"* — becomes
`base + 40`. Update `kind` from `0` to `COLLECTION_KIND_LIST_FIXED` there, not
here.

**One cleanup to fold in while converting it.** Its doc comment
(`entry_and_arena.rs:1382-1384`) states `_mfb_arena_alloc`'s clobber set as
`x0,x1,x9,x10,x14,x15,x16,x20-x28`. That contradicts `.ai/compiler.md:52` in both
directions: compiler.md says **all** of `x0`–`x17` are clobbered (the comment
omits `x2`–`x8`, `x11`–`x13`, `x17`) and that `x19`–`x28` are **preserved** by the
helper's frame (the comment claims `x20`–`x28` are clobbered). The code itself is
safe — it uses vregs and the allocator's model treats the call as `ALL_INT`
clobber, as the inline comment at `:1389-1390` says — so this is a stale
*comment*, not a miscompile. But it is stale in the exact way bug-350 and the
`arena-alloc-clobbers-x14-x15` note warn about, sitting in a file this sub-plan
is consolidating. Replace the enumeration with a pointer to `.ai/compiler.md`
rather than a second, narrower list.

## Compatibility / Format Impact

Nothing changes. No layout, format, rule, spec, or diagnostic change.

## Phases

### Phase 1 — data-base consolidation (lowest risk, highest site count)

- [ ] Add `emit_collection_data_pointer_into`; make
      `emit_collection_data_pointer` (`builder_collection_layout.rs:1725`)
      delegate to it.
- [ ] Convert all 32 open-coded sites, one commit per area (`os`/`fs`, `net`/`tls`,
      `audio`/`crypto`, `entry_and_arena`/`builder_*`).
- [ ] Append the linear-reader audit findings to
      `bugs/bug-365-linear-data-region-readers-ignore-entry-order.md` §Scope,
      converting its unverified table into confirmed/cleared entries.

Acceptance: `artifact-gate` byte-identical after each commit; bug-365's worklist
has no unverified rows left.
Commit: —

### Phase 2 — constructor consolidation

- [ ] Add `emit_alloc_list` (§4.2), generalized over `element_type`.
- [ ] Convert `lower_simd_alloc_list` (`entry_and_arena.rs:1387`) **first** — it
      is the smallest self-contained constructor and exercises the free-function
      form (§4.5). Fix its stale `_mfb_arena_alloc` clobber comment
      (`:1382-1384`) in the same commit.
- [ ] Repoint `audio/alsa.rs:1273`, `audio/macos.rs:1546`, `crypto_ec.rs:183`;
      delete the duplicates.
- [ ] Convert the ~30 open-coded header+entry writers, one commit per area.
- [ ] For each, verify the spill discipline around `_mfb_arena_alloc` matches
      what the site did before (`.ai/compiler.md`). A differing spill is a
      byte-diff — treat it as a finding, not a nuisance.

Acceptance: `artifact-gate` byte-identical after each commit; `emit_alloc_byte_list`
exists once; `scripts/test-accept.sh` green with zero churn.
Commit: —

### Phase 3 — iteration consolidation

- [ ] Add the `emit_list_iteration_*` trio (§4.1), lifted from the existing
      `transform`/`filter`/`reduce` helpers.
- [ ] Convert `lower_for_each`'s List arm (`builder_control.rs:1201`), then the
      remaining ~17 cursor loops, one commit per file.
- [ ] `lower_list_replace` (`builder_strings.rs:306`) is the awkward one — see
      §4.4. Convert **both** of its cursor passes, and its construction in
      Phase 2. Land it last, after the simpler loops have validated the trio.
- [ ] Leave every Map arm inline.

Acceptance: `artifact-gate` byte-identical; a `List OF Integer` `FOR EACH` emits
the same instructions as before; no `COLLECTION_ENTRY_SIZE` stride add remains in
a List path outside the trio.
Commit: —

## Findings (implementation, 2026-07-19)

### The guard had to be built first

This sub-plan's acceptance is `artifact-gate` byte-identity, and the gate was
nearly blind to the layer it covers: of ~1220 goldens, only six carried generated
code, and none exercised list operations. A green gate would have proved nothing.
`tests/rt-behavior/collections/list-ops-codegen-rt` now carries a `.ncode`
golden, verified to catch a semantically-identical `mul` operand swap in
`emit_element_value_offset` that the rest of the gate missed entirely. Do not
proceed with any further conversion without checking that anchor.

### Track 2 (construction): 13 sites, not ~30; 3 folded

Measured rather than estimated: **13** open-coded constructors (a `KIND` write
followed by an entry-fill loop writing `COLLECTION_ENTRY_FLAG_USED`) across 10
files. §2's "~30" was high, the same way plan-57-A's "38 indexed read sites" was.

Three are now one: `emit_alloc_byte_list` was verbatim in `audio/alsa.rs` and
`audio/macos.rs` (moved to `audio/mod.rs`), and `crypto::randomBytes`'s
open-coded constructor was a verbatim copy of
`crypto_ec::emit_build_byte_list`'s body — first 36 emitted instructions
identical — and now calls it.

Both consolidations changed **label names only** in the `-ncode` dump, which the
gate cannot see for these files anyway. Both were verified the right way: build a
probe and compare the **executable's sha256** before and after. Bit-identical in
all three cases (macos audio, linux-aarch64 audio glibc+musl, crypto). That is
the verification pattern for the remaining 10.

A general `emit_alloc_list` — parameterized on `element_type` rather than
hardcoding `payloadSize = 1` — is **still to be built**, and is what plan-57-D
needs (it is one of D's six edit sites, currently missing).

### Track 1 (iteration): `element_type` threaded; `lower_for_each` cannot convert

- `initialize_collection_loop_slots` and `advance_collection_loop` now take
  `element_type` (unused, documented as plan-57-D's branch point).
  `load_collection_loop_item` already had it. All four call sites updated.
  Byte-identical.
- **`lower_for_each` cannot be converted to the trio byte-identically.** It holds
  `cursor` and `remaining` in registers from `allocate_register()` across the
  whole loop, while the trio keeps them in stack slots and uses
  `temporary_vreg()` scratch. The two emit different code by construction, so
  routing `lower_for_each` through the trio is a rewrite, not a consolidation —
  and it would land with no byte-identity signal to check it against.
- Its List and Map arms also **share** their cursor init and advance; only the
  payload load differs. So "convert the List arm only" (§4.1) is not available
  without first splitting the shared init/advance per arm, which is itself a
  behavior-preserving-but-not-byte-identical change.

Recommendation: `lower_for_each` should be converted **in plan-57-D**, where the
codegen changes deliberately and the diff is reviewed as a real change, rather
than here where the whole discipline is non-change. The `.ncode` anchor above
makes that review possible.

## Validation Plan

- Tests: **no new tests.** This sub-plan asserts non-change.
- Runtime proof: not applicable. Nothing changes at runtime; do not claim one.
- Byte-identity: `scripts/artifact-gate.sh` after **every** commit. This is the
  whole validation story, and it is stronger here than the test suite, because
  the register-clobber class this touches passes small tests by construction.
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual`,
  zero churn.
- Side deliverable: bug-365's §Scope worklist fully triaged.

## Open Decisions

- **Does generalizing `emit_alloc_byte_list` over `element_type` stay
  byte-identical for `"Byte"`?** It must, but a `count * 1` multiply may or may
  not fold. Recommend: write the helper with an explicit `payloadSize == 1`
  fast path from the start rather than discovering it via a failed `artifact-gate`.
- ~~**`lower_simd_alloc_list` strides by `ENTRY_SIZE + 8`.**~~ **Resolved — not
  an anomaly, and not a stride.** See §4.5. It is the allocation-size arithmetic
  factored into one multiply; the entry stride is a plain `COLLECTION_ENTRY_SIZE`.
  Convert it normally — in fact convert it early (§4.5).

## Summary

The risk is the same as plan-57-A's — silent divergence during consolidation —
concentrated in the constructors, where every site allocates and therefore every
site has a register-spill discipline that must be preserved exactly. Byte-identity
after every commit is the guard, and Phase 1 is ordered first because it is the
highest site count at the lowest risk, which builds confidence in the process
before Phase 2 touches allocation.

Untouched: maps, the block layout, every constant, and all behavior.
