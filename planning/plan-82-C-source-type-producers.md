# plan-82-C: Source-type the register producers (1825 sites)

Last updated: 2026-08-02
Effort: large (3h–1d)
Depends on: plan-82-B (the allocator already consumes/produces typed operands, so
producers can emit typed vregs with no string bridge left in the hot path)

Change `allocate_register`, `allocate_fp_register`, and `temporary_vreg`
(`src/target/shared/code/builder_registers.rs:14,65,182`) to return a **typed
register handle** instead of `String`, and migrate all **1825** call sites so a
bare register operand is stored as `Operand::VReg { class, id }` inline — no
`Box<str>` allocated at production. This is the blast-radius phase plan-78-C
called "too large for one plan"; it is 1825 edits, not a reason to defer.

Single behavioral outcome: no register operand is heap-allocated at production;
the acceptance compile's total allocation count drops by the production-time vreg
share, and every emitted byte is identical.

References:

- plan-82-A / plan-82-B — the typed representation and the typed hot path this
  consumes.
- `src/target/shared/code/builder_registers.rs` — the three producers.
- The 44 files containing the 1825 sites (`rg -l
  'allocate_register\|allocate_fp_register\|temporary_vreg' src/`).

## Prerequisites

See plan-82-A §Prerequisites. Additionally: **if plan-82-B is not complete,
plan-82-C cannot start, full stop** — otherwise producers would emit typed vregs
into an allocator that still string-parses, adding a bridge this plan is meant to
delete.

| Must be true | Command | Status |
|---|---|---|
| plan-82-B merged: allocator carries/writes typed operands | `rg -n 'Operand::Phys \{' src/target/shared/code/regalloc/` | NOT MET (B pending) |

## 1. Goal

- The three producers return a typed handle (name TBD in Phase 1 — likely
  `VReg`, or a thin `Register` newtype wrapping `{class, id}`).
- All 1825 call sites compile against the new return type. A bare-register
  `.field(name, handle)` stores `Operand::VReg` inline (via `impl Into<Operand>`
  for the handle). Compound/addressing-mode sites follow plan-82-A Phase 1's
  decision (stay `Raw`: format the handle's rendered token into the bracketed
  string as today — correctness preserved, those specific operands still string,
  and they are the measured minority).

### Non-goals

- Same as plan-82-A §Non-goals; byte-identity absolute.
- Do not change allocation order or vreg numbering — the handle carries the same
  `id` the string `%vN` carried; `next_vreg` sequencing is load-bearing for
  goldens (`vreg-alloc-order-load-bearing`). A pure carrier-type change.
- Do not touch the encoder (D).

## 2. Current State

Producers return `String` (`allocate_register` → `Result<String,String>`,
`temporary_vreg` → `String`). Call sites bind to a local and reuse it in
`.field(...)` and, for addressing modes, in `format!` (verified in
`builder_collection_layout.rs:109-376`). After plan-82-B the allocator no longer
needs a string, so the string return is now pure allocation overhead: 1825 sites
× (millions of invocations across 9.79 M instructions) heap boxes.

### Measured populations

| What | Count | Command |
|---|---|---|
| Producer call sites | 1825 | `rg '\.allocate_register\(\|\.allocate_fp_register\(\|\.temporary_vreg\(' src/ \| wc -l` |
| Files touched | 44 | `rg -l 'allocate_register\|allocate_fp_register\|temporary_vreg' src/ \| wc -l` |
| Bare-`.field` sites vs. `format!`/compound sites | UNMEASURED | first task of Phase 1 (splits the migration into mechanical vs. hand) |

### Verified properties

- **A rename census by grep under-reports.** Wrapped or bare-parent `.field`
  chains are invisible to `rg` on the method name; the reliable census is to
  change the return type and let the compiler enumerate the errors (see
  `rename-census-by-grep-underreports`). Phase 1 does exactly that.

## 3. Design Overview

Two site classes (measured in Phase 1):

1. **Bare register → operand** (the majority): `self.allocate_register()?` whose
   result only flows into `.field(...)` and arithmetic on the register itself.
   Mechanical: the handle's `Into<Operand>` stores `VReg` inline.
2. **Register embedded in a compound string** (`format!("[{r}, #{off}]")`, reg
   lists): the handle renders to the same token via `Display`/`rendered()`; the
   compound operand stays `Raw` per plan-82-A. Hand-checked.

Correctness risk: the 1825 sites (a wrong edit changes an emitted register).
Mitigate with compiler-enforced completeness (change the type, fix every error)
and artifact-gate byte-identity after each file/cluster.

## Phases

> Keep checkboxes current in the same commit as the work. Land in file clusters,
> byte-gating between clusters, so a regression is bisectable.

### Phase 1 — Census by compiler + return-type switch

- [ ] Change the three producers to return the typed handle. Do NOT yet fix call
      sites. Capture the full compiler error list; record the exact site count
      and the bare-vs-compound split in this file's Measured populations table.
- [ ] Add `impl From<handle> for Operand` (→ `VReg`) and `Display`/`rendered`
      parity for the handle so both site classes have a mechanical target.

Acceptance: the producers' new signature compiles in isolation; the error census
is recorded with its true count (replacing UNMEASURED).
Commit: —

### Phase 2 — Migrate the bare-register sites

- [ ] Fix every class-1 (bare) site so the handle flows into `.field` as an inline
      `VReg`. Land in file clusters; run `artifact-gate … all` per cluster.

Acceptance: all class-1 sites compile; `artifact-gate … all` byte-identical after
each cluster and at the end.
Commit: —

### Phase 3 — Migrate the compound/addressing-mode sites

- [ ] Fix every class-2 site: render the handle into the compound `Raw` string
      exactly as the old `String` did (per plan-82-A Phase 1 decision).
- [ ] Tests: extend/keep the code-builder tests; add one asserting a
      representative addressing-mode operand renders byte-identically from the
      typed handle.

Acceptance: whole crate compiles; `cargo test --bin mfb` green; `artifact-gate …
all` byte-identical.
Commit: —

### Phase 4 — Perf checkpoint

- [ ] Re-measure the plan-82-A baseline (debug+release acceptance wall + total
      allocation count). Record here. Expect the production-time vreg `Box<str>`
      share gone.

Acceptance: total allocation count and release acceptance wall both fell vs. the
plan-82-B checkpoint; acceptance suite green.
Commit: —

## Validation Plan

- Tests: `cargo test --bin mfb` (compiler tests are in the bin target); the
  addressing-mode render test above.
- Coverage check: the compiler-error census guarantees every site is in scope; a
  green build means every producer result was migrated, not skipped.
- Runtime proof: `mfb test tests/acceptance` exits 0 on the release binary.
- Doc sync: none.
- Acceptance: `artifact-gate … all` byte-identical; `cargo test`; acceptance
  green; recorded allocation drop.

## Open Decisions

- Handle type name/shape — reuse `VReg`-shaped `{class,id}` handle vs. a distinct
  `Register` newtype. Recommend the smallest thing that gives `Into<Operand>` +
  `Display`. (§Phase 1)

## Corrections

<Filled in during execution.>

## Summary

C is the blast-radius phase plan-78 deferred — 1825 mechanical-to-hand edits made
safe by compiler-enforced completeness and per-cluster byte-identity. It removes
the production-time register `Box<str>`. Untouched: the encoder (D).
