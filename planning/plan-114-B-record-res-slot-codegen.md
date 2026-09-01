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

- [x] Add `Flatness` + the shared `walk` and the two public predicates in
      `src/codegen/collection/layout/builder_collection_layout.rs` per §4.1,
      with the explicit `Res(_)` / bare-resource / resource-union arms. The two
      divergent leaf arms are commented DIVERGENCE 1 and DIVERGENCE 2 in place;
      the resource-union case is DIVERGENCE 3, written down as inherited from
      `record_field_is_pointer` so the next reader does not re-derive it.
- [x] Replace `CodeBuilder::type_is_flat` (`:625`) with the two forwarders and
      delete the old predicate and forwarder. `grep -rn "type_is_flat" src/`
      now returns exactly ONE hit — the historical sentence inside `Flatness`'s
      own doc comment explaining what was split and why. No callable named
      `type_is_flat` remains, so no site can accrete on the ambiguous name.
      Three stale doc-comment references were retargeted in the same commit
      (`builder_collection_layout.rs:406`, `:2658`, `marshal/record.rs:15`) —
      a comment naming a deleted function is a dangling citation.
- [x] Reassign all 8 call sites per the §2 table (line numbers corrected in
      Corrections C2). Touch nothing else in those functions.
- [x] Tests: unit tests in the same module asserting the divergence explicitly —
      `Res(File)`, `List OF RES File`, `Map OF String TO RES File`, and a record
      with a `RES` field are memcpy-copyable **and not** arena-transferable; and
      that `String`, `List OF Integer`, a plain record, a data union, and a flat
      `Result` are **both**, i.e. unchanged. Landed as `flatness_split_tests`
      (4 tests). Added beyond the plan: a bare resource nominal, asserted
      separately because it reaches a different arm (`is_resource_type`, not
      `ParameterType::Res`); and a cycle-guard test in both modes.
- [x] `cargo check --all-targets` (the only thing that sees test-target warnings) — clean, no warnings.

Acceptance: `cargo test --no-fail-fast` green, and
`scripts/artifact-gate.sh target/release/mfb all` reports `diffs=0`. If it does
not, the diff localizes a mis-assigned site or the §2 UNVERIFIED reachability —
objdump the flagged fixture, fix the cause, re-run to `diffs=0`.

**MET, and the "if it does not" branch is what actually happened.** The first run
reported **10 DIFFs** (`tcp`/`udp` `.ncodesum`, 5 targets each). Both causes were
mis-assigned arms, localized by dumping one fixture (Corrections C6, C7, C8), and
after fixing them:

```
artifact-gate [all]: 1310 tests, 1472 build(s), 1809 golden(s) checked, 0 diff(s)
GATE_EXIT=0
```

Commit: 714475afa (Phase 1), 06d14ab33 (the gate-driven fixes)

### Phase 2 — Lay out, build, copy, size and drop a `RES` record field

- [x] Relax the record half of `validate_resource_rules`
      (`src/target/shared/validate/mod.rs:144`) to permit a resource-owning record
      field; leave the union half unchanged. Update the doc comment on
      `check_type_declarations` (`src/ir/verify/types.rs:8-12`) to stop asserting
      the layout/drop lowering would be misled — that is no longer true. Both
      done; the record arm is deleted outright rather than weakened, and the
      union arm is untouched and pinned by a new test.
- [x] Read `emit_build_inlined_record` and confirm a non-inlined resource-typed
      field is written as its record pointer at `8*index`; record any deviation in
      Corrections and fix it here. **Confirmed, no deviation** — read
      `src/codegen/memory/marshal/record.rs:163-168`, the `else` (non-inlined)
      branch of pass 2:
      `load %v9 <- [sp + field_slots[index]]` ; `load %v10 <- [sp + result]` ;
      `store %v9 -> [%v10 + 8*index]`. It writes the field's lowered value word at
      `8*index`, which for a resource-typed field is its record pointer. Nothing
      to fix.
- [x] Add resource-field arms to the doc comments on `record_field_is_pointer`
      (`:2639`) and `record_field_is_inlined` (`:2745`) per §4.2.
- [x] Tests (codegen unit tests, `NirModule` built directly — the source ban is
      still up, so no fixture can reach this): a record
      `Holder { name AS String, handle AS RES fs.File }` that (a) builds with the
      handle pointer at its slot, (b) `copy_flat_block`s to a second block whose
      handle word **equals** the source's, (c) sizes to `8*2 + inlined-String-bytes`,
      (d) at scope-drop emits exactly one `arena_free` for the record block and
      none for the resource record. Landed as `res_field_record_layout_tests`
      (4 tests), each asserting the property **at the point codegen decides it**
      and naming the emitter that consumes the answer, so a change to either side
      breaks a test. Plus two NIR-backstop tests
      (`a_record_field_may_own_a_resource_at_nir_level`,
      `a_union_still_may_not_mix_data_and_resource_variants`) — the second is the
      guard that the relaxation did not over-reach.
- [x] Added task: writing the union test surfaced a live gap worth recording —
      `type_owns_resource` has **no `Res(_)` arm**, so a `RES`-marked field is
      invisible to the NIR backstop entirely. See Corrections C5.


Acceptance: the four assertions above pass, and
`scripts/artifact-gate.sh target/release/mfb all` still reports `diffs=0` (no
source program reaches the relaxed backstop). **MET** — the four
`res_field_record_layout_tests` pass, the two NIR-backstop tests pass, and the
gate run quoted in Phase 1 covers this phase's changes too (both landed before
it).
Commit: c3c093940

### Phase 3 — Tree-wide proof

- [x] Full `scripts/artifact-gate.sh target/release/mfb all` (~15–20 min; run it
      alone — two concurrent gates get one killed, and exit 144 / 0-byte output is
      an infrastructure kill, not a diff). **`1310 tests, 1472 build(s), 1809
      golden(s) checked, 0 diff(s)`, `GATE_EXIT=0`.** Run uncontended for the
      artifact-gate lock (the other active session held only `test-accept`, which
      guards separately and, per bug-470, is harmless cross-worktree).
- [ ] `scripts/test-accept.sh target/release/mfb /tmp/plan114b-scratch` (full).
      Never pass a real directory as the second argument; it is `rm -rf`'d.
- [x] `cargo test --no-fail-fast` redirected to a file; check cargo's own exit
      status, not a piped `tail`'s. Pre-merge: `CARGO_EXIT=101`, 83 suites ok,
      **2 failed — both bug-483's TLS fixtures**, which that run predated the fix
      for. Both pass on the merged tree with no change from this letter, so the
      letter's own result is green.
- [x] `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`
      — `cargo fmt --all -- --check` exits 0.

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

**C1 (pre-execution, carried in from a peer session) — `is_pointer_string_record`
is name-keyed on the BARE leaf and is a live miscompile on main (bug-483).**
`type_is_flat_inner` calls `is_pointer_string_record(type_)`
(`builder_collection_layout.rs:2713`), whose body is a literal match on
`"Address" | "Datagram" | "DatagramText" | "AudioDevice"` (`:2632-2635`).
bug-480 Phase 4b package-qualified the builtin value types (`Address` became
`net.Address`), and this predicate was not updated — so those records silently
switched to the inlined-String record layout while their runtime helpers still
write absolute pointers, and reading `.host`/`.name` off one SIGSEGVs. Reported
by the session working bug-483; it hits `net::lookup`,
`tcp`/`udp`/`tls::localAddress`/`remoteAddress`, `udp::receive` and
`audio::devices` on current main.

Two consequences for this letter, neither of which changes its design:

1. **Do not write a new literal name match anywhere in the split.** bug-483's fix
   adds `ParameterType::is_builtin_named(package, leaf)` in `src/types.rs`, which
   accepts both spellings. Reuse it. If bug-483 has landed by the time this letter
   runs, take its `is_pointer_string_record` verbatim and rebase; this letter
   changes only the *callers* of that function, never its body, so the conflict
   is confined to one function.
2. **It is the precedent for why §4.1's resource arms must be explicit.** bug-483
   is the same failure mode one level down: a predicate that answers by *falling
   through to a default* rather than by a stated arm silently gives the wrong
   answer for a value that arrived by the other route. §2 already records that
   `type_is_flat(Res(File))` is `true` today "incidentally rather than by
   decision" — bug-483 is what that costs when it happens to be the wrong
   default. Both predicates get a written `Res(_)` arm and a written
   bare-resource arm for this reason, not for tidiness.

**C2 (Phase 1) — the §2 classification table is CORRECT; all 8 sites verified by
reading, and the two drifted line numbers are corrected.**
Phase 1 exists to falsify the table cheaply before anything depends on it. It
survived: every site wants the meaning §2 assigns it. Two line numbers had drifted
since the plan was authored (`grep -rn "type_is_flat" src/ --include='*.rs' | wc -l`
still gives `20`, so the population is unchanged):

| Site (corrected) | Plan said | Function | Why that meaning |
|---|---|---|---|
| `memory/arena/builder_arena_transfer.rs:396` | `:386` | `copy_value_to_current_arena` dispatch | routes to `copy_flat_block` **for a thread arena transfer** → arena-transferable |
| `memory/arena/builder_arena_transfer.rs:1188` | `:945` | `collection_payload_needs_transfer_fix` | decides whether a **transferred** payload needs the deep-copy fix → arena-transferable |
| `cleanup/thread/builder_thread_cleanup.rs:198` | same | `size_computable`, failed-send pending-free | sizes the **thread-send copy** block → arena-transferable |
| `collection/layout/builder_collection_layout.rs:58` | same | `is_pointer_collection_payload_type` | in-thread slot layout → memcpy-copyable |
| `collection/layout/builder_collection_layout.rs:105` | same | `list_element_padding_alignment` | in-thread element alignment → memcpy-copyable |
| `collection/layout/builder_collection_layout.rs:2760` | same | `record_field_is_inlined` | in-thread record layout → memcpy-copyable |
| `engine/value/builder_values.rs:334` | same | `is_freeable_flat_value` | scope-drop `arena_free` **within one thread** → memcpy-copyable |
| `collection/layout/builder_collection_layout.rs:626` | `:625` | `CodeBuilder::type_is_flat` forwarder | split into two forwarders |

The rule that decides every row: **does this site's answer travel across an arena
boundary?** The three arena-transferable sites are all on the thread
transfer/send path; the four memcpy-copyable sites are all in-thread layout or
scope-drop.

**C3 (Phase 1) — §2's UNVERIFIED reachability question, answered by hand-evaluating
the predicate. SUPERSEDED BY C7 — the hand-evaluation below is WRONG in its
middle step (the collection payload is `RES`-stripped before the walk sees it),
though its conclusion happens to hold. Left in place rather than deleted so the
error and its correction are both visible.**
§2 asks whether any *currently reachable* program reaches
`builder_arena_transfer.rs:396` with a `Res`-carrying type, and says the artifact
gate is the measurement. Traced it directly instead, for `List OF RES fs.File`:

- `type_is_flat_inner` → `typed_is_collection_type` → every payload flat?
- payload is `Res(fs.File)` → no `Res` arm → falls to `else => !record_field_is_pointer(..)`
- `record_field_is_pointer(Res(fs.File))`: not a collection, not in `record_fields`,
  `base_resource_type` leaves `Res(fs.File)` unchanged so `union_names` misses, not
  `ResultOf`, not `Error` → **false**
- so `!false` → **`type_is_flat(List OF RES fs.File)` is `true` today.**

Site `:198` (`size_computable`) would therefore call such a message "sizeable" and
site `:396` would route it to `copy_flat_block`, aliasing the sender's arena. It is
**unreachable**, and letter A is now what makes that true at the front end: a
`RES`-marked collection on any thread data plane is refused with
`2-203-0138 TYPE_THREAD_RESOURCE_PLANE_REQUIRED` before codegen sees it
(`tests/syntax/threads/thread-res-collection-plane-invalid`). So this letter's
split changes these two sites' answers only for programs that cannot be written,
which is exactly why `artifact-gate.sh all` → `diffs=0` is the right gate and a
legitimate one.

**C4 — 30 `.ncodesum` goldens on main encode the BROKEN layout. Rebase onto
bug-483 BEFORE regenerating anything in this letter.**
Reported by the session working bug-483, from its own `artifact-gate.sh all` run.
The goldens for the affected records were regenerated by the bug-480 series
itself (`a243ee742`, `8b27c8a11`) **after** the regression landed and before
anyone ran the affected programs — so main's committed sums are a snapshot of the
miscompile. Restoring the correct pointer layout moves them back:

- **30 sums, 6 packages x 5 targets**: `audio`, `http`, `net`, `tcp`, `tls`, `udp`.
  (`http` because its endpoints carry `net::Address`; `audio` because
  `audio::devices()` returns `AudioDevice`, whose `name` String had the same
  broken layout — garbage device names were a measured symptom.)
- `.ir` / `.ast` are untouched: the fix changes a layout predicate, not a type
  name or a line number.

**The operational rule for this letter:** if the predicate split changes who lands
on `is_pointer_string_record`, it will move these same 30 sums again. Rebase onto
the bug-483 fix and its golden commit **first**, then regenerate. Regenerating
before rebasing would regenerate their fix away and the resulting diff would look
like this letter's work. This is the `abi-function-migration-drifts-ncodesum`
discipline — gate to 0 diffs before landing so new diffs are provably yours —
with the extra wrinkle that the baseline itself is currently wrong.

Note also that a `no_other_declared_record_is_pointer_string` unit test now walks
the registry on that branch; it will report immediately if this letter's split
changes the membership of that list.

**C5 (Phase 2) — `type_owns_resource` has no `Res(_)` arm, so the NIR backstop
cannot see a `RES`-marked field at all.**
Found by writing the union guard test: a `NirVariant` whose field is
`RES fs.File` **validated cleanly** where it should have been refused as a mixed
union. Root cause is a documented deliberate omission at
`src/target/shared/validate/mod.rs:128-131`:

```rust
// NB: no `Res(inner)` arm — the string form never stripped the `RES `
// marker (`is_resource_type("RES File")` is false), so a `Res`-wrapped
// element falls to the name check below exactly as the string walk did.
other => crate::codegen::builtins::is_resource_type(&other),
```

So `type_owns_resource(Res(fs.File))` is `false`, and by the same path
`type_owns_resource(List OF RES fs.File)` is `false` too.

Two consequences, and neither is acted on in this letter:

1. **It does not weaken anything this letter did.** The record half is being
   *removed* here, so a field the backstop cannot see is moot for records. The
   union half is what remains, and its primary enforcer is the front-end
   `TYPE_MIXED_RESOURCE_UNION` (`2-203-0087`), which letter D's §1 explicitly
   leaves untouched. The backstop is a guard against malformed NIR, not the rule.
2. **It changes how the union guard test had to be written.** Written with the
   `RES` spelling the test would have passed for the wrong reason — the union
   would validate because the field was invisible, not because the union half was
   preserved. It uses a bare `fs.File` variant field instead, with the reason in a
   comment so nobody "simplifies" it back.

Deliberately **not** widened here. Adding a `Res(_)` arm would make the backstop
newly reject NIR it currently accepts, which is a behavior change outside this
letter's "no change to which programs compile" non-goal, and it belongs with the
union rules rather than with record layout. Recorded so letter D's union work has
it in hand.

**C6 — §4.1's divergence table is WRONG for the bare-resource row, and the
artifact gate is what proved it.**
The plan says a bare resource nominal is `true` for MemcpyCopyable and `false`
for ArenaTransferable, by analogy with the `RES`-marked element. Implemented as
written, `artifact-gate.sh all` reported **10 DIFFs** — `tcp` and `udp`
`.ncodesum` on all 5 targets.

Root-caused by dumping one fixture rather than theorising, per
`.ai/testing-gates.md`. `tests/byte-identity/tcp` has **no thread in it at all**,
which is what ruled out the arena sites immediately. The `.ncode` diff was:

```
$ diff base.ncode mine.ncode
408c408
<     "frame": { "stackSize": 8032, ... }
>     "frame": { "stackSize": 8080, ... }
1086c1086
<     { "name": "ready_674", ... }
>     { "name": "pending_temp_674", "type": "pending_temp", "offset": 5440 }
```

A **new `pending_temp` slot**, i.e. a new pending free. `is_freeable_flat_value`
is `memcpy_copyable(t) && (String | collection | ResultOf | record | data union)`.
The tcp fixture's `Result OF tcp.Socket` used to fail the first term; with the
bare-resource row `true` it passed **and** matched `ResultOf`, so the value was
newly claimed as a freeable flat block and a free was emitted for something that
is not a block. The row does not stay local to resources — it propagates through
every structural arm.

The two positions genuinely differ, and the original code was right:

- `Res(inner)` is an **element/field marker**: the enclosing block owns an 8-byte
  slot holding the handle, so the enclosing block is still one pointer-free run
  of bytes and a `memcpy` of it is correct.
- A **bare resource nominal** is the value's OWN type. "Flat" would assert the
  resource *record* is a copyable block that `arena_free` reclaims as a unit. It
  is not: separately allocated, own lifetime, own close op.

Corrected to `false` for both modes — identical to `type_is_flat`. Byte-identity
restored and verified per-fixture before re-running the gate:

```
$ shasum -a 256 <tcp .ncode built from clean main 213803f96>
56c452e3aef5519d7cda7de28da8d3107aff380eb4fd731ac14d68b9b3d32c85   # == golden
$ shasum -a 256 <tcp .ncode with the fix>
56c452e3aef5519d7cda7de28da8d3107aff380eb4fd731ac14d68b9b3d32c85   # == golden
```

The baseline build (`git archive 213803f96` → `cargo build --release`) matching
the golden is what made the diff *provably* this letter's rather than
pre-existing bug-483 noise.

> **The sha above expires.** bug-483's golden commit moves the `tcp` and `udp`
> `.ncodesum`s — all 5 targets each, part of its 30 — because main's committed
> sums encode the broken `net::Address` layout (see C4). Once that lands, a build
> from `213803f96` will **no longer** match the committed tcp/udp goldens, and the
> mismatch will be **bug-483's, not this letter's**. It is the same 10 files this
> letter's gate flagged, which makes it a real trap rather than a theoretical one:
> the identical file list, for an unrelated second reason.
>
> So do **not** carry `56c452e3…` across a rebase. After rebasing onto bug-483,
> re-derive the baseline from the new merge-base and re-run the comparison; this
> letter's fix should show 0 diffs on tcp/udp against the *corrected* goldens, and
> if it does not, that is real and is this letter's.
>
> **Done — and it holds.** bug-483 landed on main as `6e56e1264` and was merged
> into this branch (clean, no conflict, despite both sides editing
> `builder_collection_layout.rs`: their change is the *body* of
> `is_pointer_string_record`, mine is its *callers*). Re-run against the
> corrected sums:
>
> ```
> artifact-gate [all]: 1311 tests, 1473 build(s), 1809 golden(s) checked, 0 diff(s)
> GATE_EXIT=0
> ```
>
> So the split is neutral against both the old and the new goldens, which is a
> stronger statement than either run alone. The superseded sha `56c452e3…` is
> retained above only as the record of how the C6 defect was originally
> localized; **do not re-use it as a baseline.**

**C7 — §2's "`type_is_flat(Res(File))` is `true` today" is right about the type
and WRONG about the collection, and my own Correction C3 inherited the error.**
Both the plan and C3 reasoned that `type_is_flat(List OF RES fs.File)` is `true`
because the payload `Res(fs.File)` has no arm and falls through to
`!record_field_is_pointer(..)`. **The payload is never `Res(fs.File)`.**
`typed_list_element_type` and `typed_map_type_parts` **strip the `RES` marker**
(`src/codegen/engine/types/type_utils.rs:345`, `:352` — both call
`typed_strip_res_marker`), so `collection_payload_types` yields a **bare**
`fs.File` and the walk has always taken the bare-resource arm → `false`.

Consequences, all of which make this letter *smaller* than the plan thought:

1. `type_is_flat(List OF RES fs.File)` is and always was **`false`**, so §2's
   "`builder_arena_transfer.rs:386` would route a `List OF RES fs::File` to
   `copy_flat_block`, aliasing the sender's arena" **never happened**. There was
   no latent aliasing bug to fix there.
2. The `Res(_)` arm is reachable **only where a type is stored unstripped — a
   record field**, which is exactly the new shape this letter exists for. So the
   divergence the split introduces has a blast radius of precisely the new
   feature, and touches no existing program. That is a far stronger neutrality
   argument than the plan's, and it is measured rather than assumed.
3. §2's UNVERIFIED reachability row is answered **"no"** — but for a different
   reason than C3 gave. Not "letter A refuses it at the front end", but "the
   predicate never saw a `Res` payload in the first place".

C3's hand-evaluation is superseded by this. It reached the right conclusion
(`diffs=0` for existing programs) by the wrong route, which is why the gate still
found a real defect the reasoning had missed — a good argument for running the
gate even when the analysis says it is unnecessary.

**C8 — `copy_value_to_current_arena` takes MemcpyCopyable, not
ArenaTransferable (§2 table corrected).**
The §2 table assigns `builder_arena_transfer.rs:396` "arena-transferable". That
conflates two questions, and the function's own name gives it away: it copies
into the **current** arena, and most of its callers are in-arena — the `Result`
wrap at `:145` is reached by any `TRAP` in a thread-free program. Whether the
*source* came from another thread's arena is the caller's question, not this
value's shape, and it is answered where the cross-thread decision is actually
made: `collection_payload_needs_transfer_fix` (`:1188`) and the thread-send
`size_computable` (`builder_thread_cleanup.rs:198`), which keep
arena-transferability. Those two are the genuine cross-arena guards and the only
consumers of that predicate.

<!-- Further corrections filled in during execution. -->

## Summary

The risk here is a mis-assigned call site quietly changing codegen for a type that
already exists, and the artifact gate is a genuinely decisive check for it because
this letter is provably neutral for every reachable program. The `RES` record field
layout itself needs almost no new code — the existing classifications already
produce an 8-byte handle slot; the work is stating that intent and testing it.
Untouched: ownership, the front-end ban, the resource-record header, and every
format.
