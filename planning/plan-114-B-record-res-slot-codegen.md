# plan-114-B: Split flatness into "memcpy-copyable" and "arena-transferable", and lay out a `RES` record field

Last updated: 2026-08-30
Effort: medium (1h–2h)
Depends on: plan-114-A

`type_is_flat` (`src/codegen/collection/layout/builder_collection_layout.rs:2684`)
currently answers two different questions with one predicate: *"does a `memcpy`
deep-copy this value?"* and *"is this block safe to relocate into another thread's
arena?"* For every type that exists today the answers coincide, so the conflation
is invisible. A record holding a resource pointer is the first type where they
diverge: within a thread a `memcpy` is exactly right (it copies the handle
pointer, aliasing the one resource, §15.6), but across an arena boundary the same
`memcpy` produces a pointer into the sender's arena.

This letter separates the two predicates and gives a `RES` record field its
layout, entirely behind the still-standing front-end ban — so no source program
can reach any of it and codegen output must not move.

Behavioral outcome: `type_is_memcpy_copyable` and `type_is_arena_transferable`
exist, every current call site takes the one it means, a NIR record carrying a
`RES` field builds/copies/sizes/drops correctly under unit test, and
`scripts/artifact-gate.sh target/release/mfb all` reports `diffs=0`.

References:

- `.ai/codegen-invariants.md` — record layout, vreg-alloc order.
- `.ai/collections.md` — List/Map codegen, in-place mutation, memory management.
- `.ai/resources-packages.md` — canonical resource-record header, "Scope-drop
  frees owned flat values", "Thread transfer move-flag is success-gated".
- `.ai/testing-gates.md` — artifact-gate scope and the `.ncodesum` sentinel.
- `src/docs/spec/memory/04_arenas.md` — the per-thread arena.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-114-A complete and landed | `ls planning/completed/plan-114-A-*` → one match | NOT MET |
| Working tree clean; release `mfb` built | `git status --porcelain` → empty | MET (2026-08-30) |
| No other artifact-gate / test-accept running | `pgrep -f '[a]rtifact-gate\|[t]est-accept'` → no output | MET (2026-08-30) |

If plan-114-A is not complete, this letter cannot start, full stop. Everything
below is written against the world where these hold.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. If you stop, report the status of *all* prerequisites.

## 1. Goal

- `type_is_flat` is replaced by two named predicates whose meanings are stated in
  their doc comments, and each of the 8 call sites takes the one it actually needs.
- `ParameterType::Res(_)` and a bare resource nominal have an **explicit arm** in
  both predicates: `true` for memcpy-copyable (an aliasable 8-byte handle),
  `false` for arena-transferable.
- A NIR record type with a `RES` field lays out that field as a plain 8-byte slot
  at `8*index`, builds from a resource-typed value, is copied by `copy_flat_block`
  with the handle pointer aliased (not deep-copied), sizes correctly, and is freed
  at scope-drop without touching the resource record.
- `scripts/artifact-gate.sh target/release/mfb all` → `diffs=0`.

### Non-goals (explicit constraints)

- **The front-end ban stays up.** `TYPE_RESOURCE_FIELD_FORBIDDEN` still fires; no
  MFBASIC source can produce a record with a resource field after this letter.
  Lifting it is letter D.
- **No ownership decisions.** Whether a resource reached through a record field
  floats, and who closes it, is letter C. This letter lays out bytes only.
- **No `.mfp`, `.ir` or `.nir` format change.** Record fields already carry a
  rendered type string; `RES fs.File` is a valid rendering of an existing
  `ParameterType` variant (`src/types.rs:64`, `:720`).
- **No change to the resource-record header.** `RESOURCE_OFFSET_TAG/HANDLE/CLOSED/STATE`
  and `RESOURCE_RECORD_SIZE = 96` are untouched.
- No change to which programs compile.

## 2. Current State

### The one predicate, and its 8 consumers

`type_is_flat(model, type_)` (`builder_collection_layout.rs:2684`, impl
`type_is_flat_inner` `:2689`) is documented as "fully flat — a single pointer-free
block a `memcpy` deep-copies". Its arms, read at `:2698-2740`:

- `String` → true; `ResultOf(p)` → by `p`; a collection → by every payload; a
  record → `!is_pointer_string_record` and every field flat; a data union → by
  every variant.
- `is_resource_type(type_)` → **false**, commented "a resource is a move-only
  handle to its single instance, never a copyable flat block" (`:2729`).
- else → `!record_field_is_pointer(model, type_)`.

There is **no `Res(_)` arm.** A `RES`-marked element therefore falls to the `else`
arm — `record_field_is_pointer` has no `Res` arm either (`:2639`) — so
`type_is_flat(Res(File))` is `true` today, incidentally rather than by decision.
The mirror predicate in the NIR validator states this explicitly and deliberately
(`src/target/shared/validate/mod.rs`, `type_owns_resource`: *"NB: no `Res(inner)`
arm — the string form never stripped the `RES ` marker"*).

The 8 real call sites (the rest of the 20 grep hits are the definition, the
`CodeBuilder` forwarder, and comments), and which meaning each needs:

| Site | Function | Meaning it needs |
|---|---|---|
| `memory/arena/builder_arena_transfer.rs:386` | `copy_value_to_current_arena` dispatch → `copy_flat_block` | **arena-transferable** |
| `memory/arena/builder_arena_transfer.rs:945` | `collection_payload_needs_transfer_fix` | **arena-transferable** |
| `cleanup/thread/builder_thread_cleanup.rs:198` | `size_computable` for the failed-send pending-free size | **arena-transferable** |
| `collection/layout/builder_collection_layout.rs:58` | `is_pointer_collection_payload_type` | **memcpy-copyable** |
| `collection/layout/builder_collection_layout.rs:105` | `list_element_padding_alignment` | **memcpy-copyable** |
| `collection/layout/builder_collection_layout.rs:2760` | `record_field_is_inlined` | **memcpy-copyable** |
| `engine/value/builder_values.rs:334` | `is_freeable_flat_value` | **memcpy-copyable** |
| `collection/layout/builder_collection_layout.rs:626` | `CodeBuilder::type_is_flat` forwarder | both (split into two forwarders) |

### The record-field layout the design needs already exists

- `record_field_is_pointer` (`:2639`) returns **false** for a concrete resource
  nominal — read the arms: collection no, `record_fields.contains_key` no
  (resources are registered in `resource_names`, not `record_fields`, see
  `src/codegen/engine/validation/validation.rs:264` where only `"type" | "record"`
  kinds populate `record_fields`), `union_names` no for a concrete resource,
  `ResultOf` no, `Error` no. So a resource field is classified as a plain
  8-byte inline scalar slot — which is exactly the wanted layout.
- `record_field_is_inlined` (`:2745`) returns false for it too (not composite), so
  it is a value slot, not an inlined sub-block.
- `emit_record_block_size_to_slot` (`:794`) starts at `8 * fields.len()` (`:808`)
  and `continue`s past every non-inlined field (`:816`), so a `RES` field
  contributes exactly its 8 bytes with no change.
- The collection layer already states the intended rule verbatim, for elements:
  `is_pointer_collection_payload_type` (`:49-62`) — *"A resource handle is a single
  8-byte pointer to its record; a collection slot stores a copy of that pointer
  exactly like any other pointer payload (§15.6)."* This letter carries that
  sentence to the record layer.

### Measured populations

| What | Count | Command |
|---|---|---|
| `type_is_flat` grep hits (incl. def/forwarder/comments) | 20 | `grep -rn "type_is_flat" src/ --include='*.rs' \| wc -l` → `20` |
| — of which real call sites needing classification | 8 | the table above, from the same grep |
| `record_field_is_pointer` grep hits | 9 | `grep -rn "record_field_is_pointer" src/ --include='*.rs' \| wc -l` → `9` |
| `.ncodesum` byte-identity goldens (5 targets) | 127 | `ls tests/byte-identity/*/golden/*.ncodesum \| wc -l` → `127` |
| LOC of the file holding the predicate | 3041 | `wc -l src/codegen/collection/layout/builder_collection_layout.rs` → `3041` |

### Verified properties

- **A `RES` record field needs no new size or offset code.** Read
  `emit_record_block_size_to_slot:794-840` and `record_field_is_inlined:2745-2761`
  together: a non-inlined field contributes the fixed `8*len` term and is skipped
  by the walk. Verified by reading, not by running — Phase 2's unit test is what
  proves it.
- **`type_is_flat(Res(File))` is `true` today.** Traced by reading `type_is_flat_inner`
  (`:2689-2742`) and `record_field_is_pointer` (`:2639-2653`): neither has a `Res`
  arm, so `Res(File)` reaches `else => !record_field_is_pointer(..)` → `true`.
  Consequence: `builder_arena_transfer.rs:386` would route a `List OF RES fs::File`
  to `copy_flat_block` on a thread transfer, aliasing the sender's arena. Nothing
  reachable gets there — plan-114-A's front-end rule refuses the plane first — but
  the codegen predicate should say no on its own.
- **UNVERIFIED:** whether any *currently reachable* program reaches
  `builder_arena_transfer.rs:386` with a `Res`-carrying type. Phase 3's artifact
  gate is the measurement: `diffs=0` means no, and a diff localizes to the fixture
  that does. Do not assume either answer.

## 3. Design Overview

Two predicates, one shared walk skeleton, explicit resource arms in both:

```rust
/// True when a `memcpy` of this value's block is a correct COPY within one
/// thread. A resource handle qualifies: the 8-byte slot is a pointer to the one
/// resource record, and copying it aliases that resource rather than duplicating
/// it (§15.6) — the same rule `is_pointer_collection_payload_type` already
/// applies to a collection slot.
pub(crate) fn type_is_memcpy_copyable(model: &TypeModel, type_: &ParameterType) -> bool

/// True when this value's block may be RELOCATED into another thread's arena.
/// Strictly stronger than `type_is_memcpy_copyable`: arenas are per-thread, so a
/// resource handle anywhere inside the block would arrive pointing into the
/// sender's arena. A resource — bare, `RES`-marked, or nested in a field or
/// payload — makes this false.
pub(crate) fn type_is_arena_transferable(model: &TypeModel, type_: &ParameterType) -> bool
```

The design uncertainty is not in the predicates; it is in whether the 8-site
classification in §2 is right. That is cheap to falsify and is scheduled first
(Phase 1), before any layout work.

The correctness risk is that a mis-assigned site silently changes codegen for an
existing type. **Byte-identity IS this letter's gate**, and legitimately so: the
front-end ban means no source program has a resource record field, and every
non-resource type's answer is unchanged by construction. `artifact-gate.sh
target/release/mfb all` must report `diffs=0`. A diff is not a falsified premise —
it is either a mis-assigned site or the UNVERIFIED reachability above; objdump one
fixture to localize (`.ai/testing-gates.md`), fix the cause, and the gate passes.

Rejected alternatives:

- *Keep one predicate and special-case the transfer path at its three call sites.*
  Rejected: it puts the arena rule in three places that must stay in step, which is
  the same class of bug as the three bind-type-keyed close-op sites in
  `.ai/resources-packages.md`.
- *Make `type_is_flat` false for a record with a `RES` field and add a bespoke
  record-copy path.* Rejected: `memcpy` is already the correct copy for that record;
  a bespoke path would be strictly more code doing the same thing, and it would
  drop the record out of `is_freeable_flat_value` so its block would leak.

## 4. Detailed Design

### 4.1 The two predicates

Both are implemented as one private `walk(model, type_, mode, visited)` with a
`mode: Flatness { MemcpyCopyable | ArenaTransferable }` parameter, so the
structural arms (collection payloads, record fields, union variants, `ResultOf`,
the cycle guard) exist once and cannot drift. The arms differ in exactly three
places:

| Arm | MemcpyCopyable | ArenaTransferable |
|---|---|---|
| `ParameterType::Res(inner)` | `true` (aliasable handle slot) | `false` |
| `is_resource_type(type_)` (bare nominal, incl. a stateful spelling) | `true` | `false` |
| `union_is_data(type_) == false` and `union_names.contains(base)` (resource union) | `false` — a `{tag, record-ptr}` block is not a plain slot | `false` |

Everything else is identical to today's `type_is_flat_inner`. Note the resource-union
row keeps today's answer (`record_field_is_pointer:2648` already routes a resource
union to the pointer-composite path); it is written down so the next reader does not
have to re-derive it.

`CodeBuilder::type_is_flat` (`:625`) is replaced by two forwarders,
`type_is_memcpy_copyable` and `type_is_arena_transferable`. Delete `type_is_flat`
and its forwarder — do not leave an alias, or sites will keep accreting on the
ambiguous name.

### 4.2 Record layout for a `RES` field

No new code; the existing classifications already produce the right layout (§2
"Verified properties"). What this letter adds is:

1. Doc comments on `record_field_is_pointer` and `record_field_is_inlined` stating
   the resource-field case explicitly, mirroring the sentence at `:49-62`.
2. Relaxing the NIR backstop `validate_resource_rules`
   (`src/target/shared/validate/mod.rs:144`) so a record field may own a resource
   at the NIR level. Keep the **union** half of that function unchanged — a union
   still may not mix data and resource variants.
3. Unit tests that build a `NirModule` with such a record and assert the emitted
   layout, copy, size, and drop.

The `WITH`-update rebuild path (`emit_build_inlined_record`) writes each
non-inlined field as a value word at `8*index`; a resource-typed value lowers to
its record pointer, so the write is the pointer. Verify this by reading
`emit_build_inlined_record` before writing the test, and record what you find in
Corrections if it differs.

### 4.3 Scope-drop of a record holding a handle

`is_freeable_flat_value` (`builder_values.rs:334`) gates on memcpy-copyability plus
"is a record/String/collection/data-union/Result", so a record with a `RES` field
stays freeable: scope-drop `arena_free`s the **record block**. It must not touch
the resource record — that block is reclaimed by the resource's own
`ActiveCleanup::Resource` / `emit_resource_block_reclaim`
(`src/codegen/resource/cleanup/builder_resource_cleanup.rs`), which letter C
routes. Assert this in the Phase 2 test by counting `arena_free` sites.

## Compatibility / Format Impact

- No externally observable change. No `.mfp`, `.ir`, `.nir`, `.ncode`, ABI, or
  diagnostic change. `RESOURCE_RECORD_SIZE`, the record block layout, and the
  collection entry table are all unchanged.
- Internal API: `type_is_flat` is deleted in favour of two named predicates. It is
  `pub(crate)`, so the blast radius is the 8 sites in the §2 table.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work it describes; `- [~]` for partial with one line on what
> remains; `- [x] ~~text~~ — moot: <evidence>` for a dropped task. An unticked box
> means NOT DONE.

### Phase 1 — Split the predicate (no layout work, no behavior change)

Falsifies the §2 classification table cheaply, before anything depends on it.

- [ ] Add `Flatness` + the shared `walk` and the two public predicates in
      `src/codegen/collection/layout/builder_collection_layout.rs` per §4.1,
      with the explicit `Res(_)` / bare-resource / resource-union arms.
- [ ] Replace `CodeBuilder::type_is_flat` (`:625`) with the two forwarders and
      delete the old predicate and forwarder.
- [ ] Reassign all 8 call sites per the §2 table. Touch nothing else in those
      functions.
- [ ] Tests: unit tests in the same module asserting the divergence explicitly —
      `Res(File)`, `List OF RES File`, `Map OF String TO RES File`, and a record
      with a `RES` field are memcpy-copyable **and not** arena-transferable; and
      that `String`, `List OF Integer`, a plain record, a data union, and a flat
      `Result` are **both**, i.e. unchanged.
- [ ] `cargo check --all-targets` (the only thing that sees test-target warnings).

Acceptance: `cargo test --no-fail-fast` green, and
`scripts/artifact-gate.sh target/release/mfb all` reports `diffs=0`. If it does
not, the diff localizes a mis-assigned site or the §2 UNVERIFIED reachability —
objdump the flagged fixture, fix the cause, re-run to `diffs=0`.
Commit: —

### Phase 2 — Lay out, build, copy, size and drop a `RES` record field

- [ ] Relax the record half of `validate_resource_rules`
      (`src/target/shared/validate/mod.rs:144`) to permit a resource-owning record
      field; leave the union half unchanged. Update the doc comment on
      `check_type_declarations` (`src/ir/verify/types.rs:8-12`) to stop asserting
      the layout/drop lowering would be misled — that is no longer true.
- [ ] Read `emit_build_inlined_record` and confirm a non-inlined resource-typed
      field is written as its record pointer at `8*index`; record any deviation in
      Corrections and fix it here.
- [ ] Add resource-field arms to the doc comments on `record_field_is_pointer`
      (`:2639`) and `record_field_is_inlined` (`:2745`) per §4.2.
- [ ] Tests (codegen unit tests, `NirModule` built directly — the source ban is
      still up, so no fixture can reach this): a record `Holder { name AS String,
      handle AS RES fs.File }` that (a) builds with the handle pointer at its slot,
      (b) `copy_flat_block`s to a second block whose handle word **equals** the
      source's, (c) sizes to `8*2 + inlined-String-bytes`, (d) at scope-drop emits
      exactly one `arena_free` for the record block and none for the resource
      record.

Acceptance: the four assertions above pass, and
`scripts/artifact-gate.sh target/release/mfb all` still reports `diffs=0` (no
source program reaches the relaxed backstop).
Commit: —

### Phase 3 — Tree-wide proof

- [ ] Full `scripts/artifact-gate.sh target/release/mfb all` (~15–20 min; run it
      alone — two concurrent gates get one killed, and exit 144 / 0-byte output is
      an infrastructure kill, not a diff).
- [ ] `scripts/test-accept.sh target/release/mfb /tmp/plan114b-scratch` (full).
      Never pass a real directory as the second argument; it is `rm -rf`'d.
- [ ] `cargo test --no-fail-fast` redirected to a file; check cargo's own exit
      status, not a piped `tail`'s.
- [ ] `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`

Acceptance: `diffs=0` tree-wide, acceptance harness green, `cargo test` green.
Commit: —

## Validation Plan

- Tests: predicate divergence unit tests (Phase 1) and NIR record layout/copy/size/drop
  unit tests (Phase 2), both in-tree `#[cfg(test)]` modules next to the code.
- Coverage check: measure with `--bin mfb` per `.ai/build-tooling.md`; confirm the
  new `Res(_)` and bare-resource arms in both modes are in the denominator. A green
  gate means "nothing *covered* changed", so an uncovered new arm is not proven.
- Runtime proof: **none is possible in this letter** — the front-end ban means no
  program can construct such a record. The runtime proof lands in letter D and is
  named there. Do not claim this letter is runtime-verified.
- Doc sync: `.ai/codegen-invariants.md` gains the two-predicate distinction (the
  arena-transferable rule is exactly the class of invariant that doc exists for).
- Acceptance: `cargo test --no-fail-fast`; `cargo check --all-targets`;
  `scripts/artifact-gate.sh target/release/mfb all` → `diffs=0`;
  `scripts/test-accept.sh target/release/mfb /tmp/plan114b-scratch`; `cargo fmt`.

## Open Decisions

- **Name of the second predicate.** `type_is_arena_transferable` (recommended —
  says the constraint) vs. `type_is_thread_sendable_block` (collides with the
  front-end `is_thread_sendable`, which is a *type* rule, not a *block* rule).
  Take the first.
- **Delete `type_is_flat` or keep it as an alias for the memcpy meaning?** Delete
  (recommended): an ambiguous name is what produced the conflation, and 8 sites is
  a small enough blast radius to do it once.

## Corrections

<!-- Filled in during execution. -->

## Summary

The risk here is a mis-assigned call site quietly changing codegen for a type that
already exists, and the artifact gate is a genuinely decisive check for it because
this letter is provably neutral for every reachable program. The `RES` record field
layout itself needs almost no new code — the existing classifications already
produce an 8-byte handle slot; the work is stating that intent and testing it.
Untouched: ownership, the front-end ban, the resource-record header, and every
format.
