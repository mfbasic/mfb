# plan-57-A: funnel every list element read through one helper

Last updated: 2026-07-19
Overall Effort: huge (>3d) — the whole plan-57 feature (A–E)
Effort: medium (1h–2h)
Depends on: nothing

plan-57 gives fixed-width-scalar lists an entry-free representation
(`kind = 2`). The blocker is not the representation — it is that **"where does
element `i` live?" is open-coded at 38 sites and there is no helper to change.**

This sub-plan introduces that helper and routes every *indexed* read through it,
with **byte-identical generated output**. It ships no feature and no behavior
change. It exists so that plan-57-D is a two-function edit instead of a
90-site one.

The single behavioral outcome: nothing changes. `scripts/artifact-gate.sh`
reports byte-identical output for every target, and `scripts/test-accept.sh` is
green with zero golden churn. Any churn is a regression.

References (read first):

- `src/docs/spec/memory/05_collections.md:24-100` — the block layout;
  `:173-198` — Capacity Headroom, including the "always derive the data base from
  `capacity`" rule.
- `src/target/shared/code/error_constants.rs:762-779` — `COLLECTION_HEADER_SIZE`,
  `COLLECTION_ENTRY_SIZE`, `COLLECTION_ENTRY_OFFSET_*`, `COLLECTION_OFFSET_*`.
- `src/target/shared/code/builder_collection_layout.rs:1725-1746`
  (`emit_collection_data_pointer`) — the one existing shared helper, ~40 call
  sites. The model to follow.
- `src/target/shared/code/builder_collection_query.rs:4-73` (`lower_list_get`) —
  the canonical open-coded read; `:33-55` is the six-instruction idiom this
  sub-plan replaces.
- `src/target/shared/code/builder_collection_compare.rs:196, 291, 391` — already
  offset-parameterized; they take `(collection, offset, length)` and need **zero**
  changes. Proof the abstraction boundary is in the right place.
- `bugs/bug-365-linear-data-region-readers-ignore-entry-order.md` — the
  correctness defect this representation work also closes for fixed-width lists.
- `bugs/bug-333-string-collection-builder-duplication.md` — the standing record of
  ~1,400 lines of duplication in these same builders. This sub-plan is the same
  medicine.
- `.ai/compiler.md` — Hard Completion Gate, and the register-lifetime rules.

## 1. Goal

- Two new helpers on `CodeBuilder`:
  - `emit_element_value_offset(dst_off, dst_len, list, index, element_type)` —
    given a list pointer and an element index, produce the payload's offset and
    length.
  - `emit_element_address(dst, list, index, element_type)` — the same, resolved
    to an absolute address via `emit_collection_data_pointer`.
- Every **indexed** list read is routed through them. The six-instruction idiom
  (`entry = list + HEADER + i*ENTRY; load valueOffset; load valueLength`) appears
  exactly once in the tree.
- `element_type` is threaded through even though this sub-plan ignores it. It is
  the parameter plan-57-D branches on; adding it later would mean touching all 38
  sites twice.
- **Byte-identical output.** The helpers emit the same instructions in the same
  order as the code they replace.

### Non-goals (explicit constraints)

- **No behavior change of any kind.** If a site's current instruction sequence
  differs from the helper's, change the *helper* to match, or leave the site alone
  and record it — do not "improve" a sequence while consolidating it.
- **Cursor-stride loops are plan-57-B**, not this sub-plan. ~20 sites hold a raw
  entry pointer and bump it by `COLLECTION_ENTRY_SIZE` per iteration; they need
  rewriting into indexed form first, which is its own risk and its own commit.
- **Map paths are out of scope entirely.** Maps keep the entry table forever.
  ~15 of the 38 reading functions are Map-only and must not be touched.
- No change to `emit_collection_data_pointer`, the block layout, or any constant.
- No new `#[allow(dead_code)]`. Both helpers gain callers in this sub-plan.

## 2. Current State

`lower_list_get` (`builder_collection_query.rs:4-73`) is the canonical shape:

```rust
move_immediate(entry_offset, ENTRY_SIZE);          // :33
multiply_registers(entry_offset, index, entry_offset);
add_immediate(entry, collection, HEADER_SIZE);     // :43
add_registers(entry, entry, entry_offset);
load_u64(value_offset, entry, ENTRY_OFFSET_VALUE_OFFSET);  // :52
load_u64(value_length, entry, ENTRY_OFFSET_VALUE_LENGTH);  // :57
```

then `emit_load_collection_payload(element_type, collection, value_offset,
value_length)`. That block, with different register names, is written out at every
indexed read site.

Measured on this worktree: **177** references to `COLLECTION_ENTRY_SIZE`, **273**
to `COLLECTION_ENTRY_OFFSET_*`, **168** to `COLLECTION_HEADER_SIZE`, across
**24 files** — all under `src/target/shared/code/`, nothing outside codegen knows
the layout.

The indexed List read sites (Map-only functions excluded):

| function | file:line |
|---|---|
| `lower_list_get` | `builder_collection_query.rs:4` |
| `lower_list_get_or` | `builder_collection_query.rs:475` |
| `lower_list_set_in_place` | `builder_collection_mutate.rs:2023` |
| `lower_list_remove_at` | `builder_collection_mutate.rs:3136` |
| `lower_collection_contains` | `builder_collection_queries.rs:78` |
| `lower_collection_sum` | `builder_collection_queries.rs:1325` |
| `lower_list_zip_fixed` | `builder_collection_queries.rs:750` |
| `lower_list_slice_range` | `builder_collection_queries.rs:1017` |
| `lower_list_find_item` | `builder_search.rs:261` |
| `lower_list_find_sublist` | `builder_search.rs:374` |
| `lower_list_mid` | `builder_search.rs:793` |

Already correct and untouched: `builder_collection_compare.rs:196, 291, 391` take
`(collection, offset, length)` as parameters and resolve the address through
`emit_collection_data_pointer`. The abstraction seam already exists one level
down — this sub-plan adds the level above it.

**Why this is the right first move.** The read-side audit put the full change at
~70 functions / ~90 sites / 24 files, in hand-written assembly emission with no
type-level protection. The bug classes already recorded in these files —
`builder_arena_transfer.rs:690-696` (bug-146, capacity-vs-count bound) and
`builder_collection_layout.rs:236-243` (bug-02, a missing region in the size calc
that corrupted the arena free list) — are exactly what a 90-site mechanical
rewrite reintroduces. Collapsing the surface to two functions first turns
plan-57-D from a sweep into an edit.

## 3. Design Overview

```
emit_element_address(dst, list, index, element_type)
    └── emit_element_value_offset(off, len, list, index, element_type)
            └── (today)  entry = list + HEADER + index*ENTRY; load off, len
                (57-D)   fixed-width: off = index * payloadSize, len = payloadSize
    └── emit_collection_data_pointer(base, list)        [unchanged]
```

Two helpers rather than one because the call sites split cleanly: some need the
`(offset, length)` pair to hand to `emit_load_collection_payload` or the compare
helpers, others want a bare address. Both live in
`builder_collection_layout.rs`, beside `emit_collection_data_pointer`.

**Where the risk concentrates:** in accidentally *changing* something. This is a
consolidation of eleven hand-written sequences that are similar but not provably
identical — register allocation order, scratch-vreg numbering, and instruction
order all affect the emitted bytes. The guard is `scripts/artifact-gate.sh`
byte-identity, and it is not advisory: a single differing byte means a site's
sequence was not what the helper emits, and that site must be investigated, not
re-baselined.

**Rejected alternative:** *skip the refactor and do the representation switch
directly in plan-57-D.* Rejected — that is the 90-site sweep, in assembly
emission, with a behavior change riding along, so a byte-diff could no longer be
used as the guard. Separating "move the code" from "change the code" is the only
way either step gets a clean signal.

**Rejected alternative:** *make the helpers take a `CollectionTypeLayout` instead
of an element-type string.* Rejected for now: the surrounding code passes
`element_type: &str` everywhere, and introducing a second currency mid-refactor
would obscure the byte-identity diff. Revisit in plan-57-E's cleanup.

## 4. Detailed Design

```rust
/// The payload offset and length of list element `index`, in registers.
///
/// This is the single authority for "where does element `i` live". Every
/// indexed list read goes through it; `builder_collection_compare.rs`'s
/// offset-parameterized helpers sit one level below and are unaffected.
///
/// `element_type` is unused today — the lookup entry answers for every element
/// type alike. It is threaded through because plan-57-D branches on it to give
/// fixed-width-scalar lists an entry-free representation, and adding the
/// parameter later would mean touching all 38 call sites twice.
pub(super) fn emit_element_value_offset(
    &mut self,
    dst_offset: &str,
    dst_length: &str,
    list: &str,
    index: &str,
    element_type: &str,
) -> Result<(), String>
```

Body is verbatim the sequence from §2, using `self.temporary_vreg()` for the two
scratch registers so it composes with the register allocator the same way the
inlined code did.

```rust
/// The absolute address of list element `index`'s payload.
pub(super) fn emit_element_address(
    &mut self,
    dst: &str,
    list: &str,
    index: &str,
    element_type: &str,
) -> Result<(), String>
```

`emit_element_value_offset` + `emit_collection_data_pointer` + `add_registers`.

### Landing order

Convert **one site first** — `lower_list_get` — and land it alone behind
`artifact-gate`. That single diff proves the helper reproduces the idiom exactly.
Only then convert the remaining ten, in one commit per file so a byte-diff
regression bisects to a small blast radius.

## Compatibility / Format Impact

Nothing changes. No layout change, no format change, no rule change, no spec
change, no diagnostic change. This sub-plan is invisible from outside
`src/target/shared/code/`.

## Phases

### Phase 1 — the helpers, proven on one site

- [x] Add `emit_element_value_offset` and `emit_element_address` to
      `src/target/shared/code/builder_collection_layout.rs`, beside
      `emit_collection_data_pointer` (`:1725`), with the doc comment from §4
      naming plan-57-D as the reason `element_type` exists.
- [x] Convert `lower_list_get` (`builder_collection_query.rs:4-73`) only.
      (`emit_element_address` was **not** added: no converted site wants a bare
      address — both hand the `(offset, length)` pair to
      `emit_load_collection_payload`. Adding it with no caller would be a
      `#[allow(dead_code)]` promise of the kind AGENTS.md bans. plan-57-B/D add
      it when a caller exists.)

Acceptance: `scripts/artifact-gate.sh` reports **byte-identical** output for
every target. If it does not, the helper does not reproduce the idiom — fix the
helper, do not adjust the baseline.
Commit: —

### Phase 2 — convert the remaining indexed read sites

- [x] `builder_collection_query.rs`: `lower_list_get_or` (`:475`).
- [x] `builder_collection_mutate.rs`: `lower_list_set_in_place` (`:2023`),
      `lower_list_remove_at` (`:3136`). — **not convertible**, see Findings.
- [x] `builder_collection_queries.rs`: `lower_collection_contains` (`:78`),
      `lower_collection_sum` (`:1325`), `lower_list_zip_fixed` (`:750`),
      `lower_list_slice_range` (`:1017`). — all **cursor-strided**, not indexed;
      plan-57-B's scope. See Findings.
- [x] `builder_search.rs`: `lower_list_find_item` (`:261`),
      `lower_list_find_sublist` (`:374`), `lower_list_mid` (`:793`).
      — **not convertible**, see Findings.
- [x] One commit per file.
- [x] Record in the plan any site that **cannot** be converted byte-identically,
      with the reason. — see Findings. Do not force it — a site whose sequence genuinely differs
      is information about the codebase, not an obstacle.

Acceptance: `scripts/artifact-gate.sh` byte-identical after each commit;
`scripts/test-accept.sh target/debug/mfb target/accept-actual` green with **zero**
golden churn; the six-instruction idiom appears in exactly one place
(`grep -c ENTRY_OFFSET_VALUE_OFFSET` over List paths drops to the helper plus the
Map-only functions).
Commit: —

## Findings (implementation, 2026-07-19)

**Status: COMPLETE.** Both helpers' worth of surface collapsed to one helper, two
sites converted, four recorded as unconvertible with reasons. `artifact-gate`
byte-identical throughout.

### The indexed-read population is 2, not 11

§2's table lists eleven functions. Scanned mechanically for the exact
six-instruction idiom, only **six** sites match at all, and only **two** convert
byte-identically:

| site | verdict |
|---|---|
| `lower_list_get` (`builder_collection_query.rs`) | **converted** |
| `lower_list_get_or` (`builder_collection_query.rs`) | **converted** |
| `lower_list_remove_at` (`builder_collection_mutate.rs:3576`) | not convertible — keeps `ENTRY_SIZE` live in its own register (`scratch16`) for the entry-table span copies that follow; the helper writes the product over that register. Converting would clobber a live value: a behavior change, not a consolidation. |
| `lower_list_find_item` (`builder_search.rs:326`) | not convertible — the address is computed **before** the loop and the two payload loads happen **inside** it, after a label. It is a cursor loop wearing the idiom's clothes. plan-57-B. |
| `lower_list_find_sublist` (`builder_search.rs:470`) | not convertible — computes entry addresses for **two** lists interleaved, sharing one `ENTRY_SIZE` register across both. |
| `lower_strings_grapheme_at` (`builder_strings_builtins.rs:2657`) | not convertible — emits `mul rhs, lhs` in the opposite operand order. Semantically identical (`mul` is commutative) but a real byte difference; see below. |

The remaining ~36 `COLLECTION_ENTRY_OFFSET_VALUE_OFFSET` references in these files
are cursor-strided or entry *writes*, both of which are plan-57-B's scope. So the
containment surface for plan-57-D is far smaller than §2 estimated on the read
side, and correspondingly larger on plan-57-B's.

### `artifact-gate` has a coverage hole — do not trust a bare 0-diff

`lower_strings_grapheme_at` was converted, and `artifact-gate` reported
**0 diffs across 1211 goldens**. That was a false negative: `strings::graphemeAt`
appears only in `tests/acceptance/`, which has no `golden/` directory, and the
gate skips any test directory without one. The site had **zero** artifact
coverage.

The difference was found only by building a purpose-made probe program with and
without the change and diffing the `-ncode` dumps directly:

```
< { "op": "mul", "dst": "x10", "lhs": "x10", "rhs": "x9" }
> { "op": "mul", "dst": "x10", "lhs": "x9",  "rhs": "x10" }
```

**This matters for plan-57-B and plan-57-D**, which both lean on `artifact-gate`
as their guard. A green gate means "nothing covered changed", not "nothing
changed". Before trusting it for a given site, confirm that site is reachable
from a test that has a `golden/` directory; where it is not, build a probe
project and diff `-ncode` by hand. The four unconvertible sites above should each
get that treatment when plan-57-D reaches them.

## Validation Plan

- Tests: **no new tests.** This sub-plan asserts non-change, and the existing
  suite is the assertion. Adding tests here would be noise.
- Runtime proof: not applicable — nothing changes at runtime. The Hard Completion
  Gate binds plan-57-C and -D, which do change behavior. Do not claim a runtime
  proof for a refactor.
- Byte-identity: `scripts/artifact-gate.sh` is the primary and sufficient guard,
  run after every commit rather than once at the end.
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual`,
  expecting zero churn.

## Open Decisions

- **Should the helpers be `pub(super)` on `CodeBuilder`, or free functions?**
  Recommend `CodeBuilder` methods: ~32 open-coded data-base computations live in
  standalone `CodeFunction` runtime helpers where no `CodeBuilder` exists
  (`os.rs`, `fs_helpers_*`, `net/io.rs`, `tls/*`, `audio/*`, `crypto*`), and those
  are **construction** sites, not indexed reads — they are plan-57-B's problem and
  need a different shape. Do not contort this helper to serve both.
  Decision: `CodeBuilder` methods
- ~~**Convert `lower_list_replace` here or defer?**~~ **Resolved — moved to
  plan-57-B.** It lives at `builder_strings.rs:306` (not `builder_search.rs`, as
  an earlier draft of the table above had it) and is **cursor-strided, twice**: a
  length-measuring pass and a copy pass, each seeding a cursor at
  `collection + COLLECTION_HEADER_SIZE` and advancing by `COLLECTION_ENTRY_SIZE`
  (`:399-404`). It is also a **constructor**, sizing
  `count*ENTRY + HEADER + data_len` and writing a fresh header and entry table
  (`:413-422`). That is two of plan-57-B's three tracks and none of this
  sub-plan's.
- ~~Same check for `lower_list_find_sublist`.~~ **Resolved — it stays here.** It
  derives entry addresses from indices (`multiply_registers` by `ENTRY_SIZE`,
  then add `HEADER_SIZE` — `builder_search.rs:451-464`) for *both* lists in its
  inner compare, so it is the indexed pattern, just twice over.

## Summary

The whole risk is silent divergence: consolidating twelve similar-but-not-identical
sequences into one, in code that emits machine instructions, where a wrong
register or a reordered instruction is invisible in review. `artifact-gate`
byte-identity is what makes that risk manageable, which is why this sub-plan
exists separately from every part of plan-57 that changes behavior.

Untouched: maps, the block layout, every constant, and everything outside
`src/target/shared/code/`.
