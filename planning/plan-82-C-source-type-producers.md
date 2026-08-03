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
| plan-82-B merged: allocator carries/writes typed operands | `rg -n 'Operand::phys\(' src/target/shared/code/regalloc/` | **MET** (B complete: 3c4ee9b5d; `substitute` writes `Operand::phys`) |

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

- [x] All FOUR producers (`allocate_register`, `allocate_fp_register`,
      `temporary_vreg`, `temporary_fp_vreg`) now return `VirtualRegister` /
      `Result<VirtualRegister, String>`. Census captured (see Corrections): **1861**
      producer sites / **38** files; and the real funnel is the helper tier (~100
      `abi::*` builders + ~375 helper-method param sites + ~39 vreg-returning helper
      signatures), NOT direct `.field` sites. No compound-operand class exists
      (plan-82-A Phase 1: 0), so there is no bare-vs-compound split — every site is
      "bare register through a helper".
- [x] Added `VirtualRegister` handle (`operand.rs`) with `From<VirtualRegister>` /
      `From<&VirtualRegister> for Operand` (→ inline `VReg`), `Display`, and
      `render()`. `abi::*` register params converted to `impl Into<Operand>` (the
      mechanical target for both handle and `&str` sites).

Acceptance: producers' new signatures + `From`/`Display`/`render()` land; the
census is recorded with its true count (1861/38) and the corrected helper-tier
model. (Whole-crate compile is Phase 2, after the helper tier is threaded.)
Commit: ffea88cb6

### Phase 2 — Migrate the bare-register sites

- [x] Every register-carrying site migrated (producers + `abi::*` + ~375 helper
      params + ~50 vreg-returning helpers across ~35 files; parallelized across
      sub-agents, integrated + compiler-driven residual fixes on the main thread).
      Bare-register operands store inline `VReg` end to end (the residue is a vreg
      landing in a `String`-typed `ValueResult.location`, `.render()`'d — bounded).
      `artifact-gate … all` byte-identical (**0 diffs**); `cargo test --bin mfb`
      3774; acceptance 362/362.

Acceptance: all sites compile; `artifact-gate … all` byte-identical. ✓
Commit: ffea88cb6

### Phase 3 — Migrate the compound/addressing-mode sites

- [x] ~~Fix every class-2 (compound) site~~ — **moot: zero compound-register
      operands exist** (plan-82-A Phase 1 census = 0; addressing uses separate
      `base`/`offset` fields). No `Raw` register-in-string survivor to migrate.
- [x] ~~add a representative addressing-mode render test~~ — **moot/covered:** no
      compound operand exists; a bare `base`-field register's byte-identity is
      proven by the `regalloc::analysis` full-table round-trip test + the
      `artifact-gate … all` 0-diff gate over every fixture.

Acceptance: whole crate compiles; `cargo test --bin mfb` green (3774);
`artifact-gate … all` byte-identical (0 diffs). ✓
Commit: ffea88cb6

### Phase 4 — Perf checkpoint

- [x] ~~Re-measure … Expect the production-time vreg `Box<str>` share gone.~~
      **DONE — measured, and the expectation is FALSE.** Total allocations
      (counting-allocator probe, `mfb test tests/acceptance`): plan base
      **808,803,959** → post-C **789,917,084** (2.3%). Even a further pass typing
      `ValueResult.location`/`PendingTemp.location` → `Operand` (built, compiled,
      3774 tests green, then reverted as moot) left the count at **789,917,084** —
      unchanged. Reason: the typed operands are stringified/re-boxed at the
      String-based MIR/select round-trip before regalloc/encode (see plan-82-A
      §CORE-PREMISE FALSIFICATION). C's `Box<str>` share is NOT the dominant
      allocation; the per-instruction fields-`Vec` churn is.

Acceptance: **NOT MET, and it cannot be met by this design** — the allocation count
did not fall (the production-time vreg typing is discarded downstream by MIR/
select). Not weakened; recorded as a premise defect. plan-82 halts here (§A).
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

- **The scope is much larger than "1825 mechanical `.field` sites", and it is a
  viral `String`→`VirtualRegister` dataflow conversion, not a `.field` funnel
  change.** The plan modeled producers feeding `.field` directly; in reality a
  minted register flows through a deep tree of helper layers before it reaches
  `.field`: (a) the `abi::*` instruction-builders (`add_registers`, `load_u64`,
  …) take register `&str`; (b) many `CodeBuilder` helper *methods*
  (`emit_thread_copy_real`, `copy_value_to_current_arena`, `emit_money_binary`, …)
  take register `&str` **and return a minted register as `String`/
  `Result<String,String>`**. So typing the producers cascades through ~100 abi
  builders + ~375 helper-method param sites + ~39 vreg-returning helper signatures
  across ~35 files. True census: **1861** producer call sites / **38** files
  (`rg '\.(allocate_register|allocate_fp_register|temporary_vreg|
  temporary_fp_vreg)\(' src/ | wc -l`; the plan's 1825/44 omitted
  `temporary_fp_vreg` and mis-counted files). Bigger scope is more effort, not a
  deferral (skill §"Do the work").

- **Design: `impl Into<Operand>` at the helper boundary; `VirtualRegister` handle
  from producers.** `VirtualRegister { class, id }` (a `Copy`, heap-free handle) is
  returned by the four producers. Every register-role helper param (abi + builder
  methods) becomes `impl Into<Operand>`, which accepts BOTH a `&VirtualRegister`
  (→ inline `VReg`, the win) and a hardcoded `&str` physical/immediate (→ `Raw`,
  unchanged) — so hardcoded physicals stay `Raw` and only vregs become typed. A
  helper that *returns* a minted vreg returns `VirtualRegister`. Struct fields /
  `Vec<String>` element types are NOT changed — a vreg that must land in an
  existing `String` slot is `.render()`ed (a bounded, rare `Raw` residue), which
  caps the virality to signatures + locals.

- **Byte-identity is guaranteed by construction.** The transform compiles-or-fails
  (a mis-typed register→String is a compile error, never a silent byte change),
  `VirtualRegister::render()` / `VReg.rendered()` == the old `%vN` string, and the
  `next_vreg` sequence (hence every id) is unchanged (`vreg-alloc-order-load-
  bearing`). Gated by `artifact-gate … all`.

- **`impl Into<Operand>` fn-pointer break.** `abi::load_double`/`store_double` were
  passed as `fn(&str,&str,usize)` pointers to `peephole::fold_pair`; a generic
  param cannot coerce to a plain fn pointer, so `fold_pair` now takes an
  `impl Fn(&str,&str,usize)` and the two call sites pass closures.

## Summary

C is the blast-radius phase plan-78 deferred — 1825 mechanical-to-hand edits made
safe by compiler-enforced completeness and per-cluster byte-identity. It removes
the production-time register `Box<str>`. Untouched: the encoder (D).
