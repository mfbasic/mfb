# plan-84: Collapse the multi-pass instruction-stream rebuild + `fields`-Vec re-clone churn

Last updated: 2026-08-03
Effort: large (3h–1d) — structural, correctness-sensitive
Depends on: nothing representational (builds on merged plan-79/82). Independently
landable, but best sequenced **after plan-83** (so the attribution is clean and the
two efforts don't fight over the same measurement).

Stop rebuilding every function's instruction stream — and re-cloning every
instruction's `fields` Vec — five to six times across the codegen pipeline. The
operand *values* are now cheap to clone (plan-79/82: `VReg`/`Phys`/`Imm` are
heap-free), but the **`Vec<(&'static str, Operand)>` container itself allocates on
every clone**, and each pass allocates a **fresh `Vec<Instruction>`** for the whole
stream. That container/stream churn — not operand boxes — is a top allocation
class.

## Why this plan exists — the *counted* cause

Grounded in the same **measured allocation-cause attribution** as plan-83 (sampling
backtrace allocator, 1-in-2048, over `mfb test tests/acceptance`; it counts the
call sites that *cause* allocations). Result (total ≈595M):

| Est. share | Call site | What allocates |
|---|---|---|
| ~13% | `regalloc::linear_scan::run` | the rebuilt `out: Vec<CodeInstruction>` + per-instruction `scratch_for`/`evictions` maps/vecs + `substitute`'s `instruction.fields.clone()` — **run twice** (Int then Fp class) |
| ~10% | `mir::lower_to_mir` | fresh `Vec<MirInstruction>` + `mir_fields_from_code` = `fields.to_vec()` (clones every instruction's fields Vec) |
| ~10% | `arch::*::select` | fresh `Vec<CodeInstruction>` + `code_fields_from_mir` = `fields.to_vec()` (clones every instruction's fields Vec) |
| ~5% | `code_impl` ← `abi::load_/store_/move_` | building each instruction: the `fields` Vec growth in `.field()` + `&imm.to_string()` immediates |

Each function's stream is: **built** (production) → **lower_to_mir** (new Vec +
to_vec each fields) → **select** (new Vec + to_vec each fields) → **regalloc Int**
(new out Vec + fields.clone each) → **regalloc Fp** (new out Vec + fields.clone
each) → peephole → finalize. That is ~5–6 full-stream allocations and ~5–6
fields-Vec clones per instruction. Combined ≈28–33% of all compile allocations.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| Byte-identity oracle + acceptance harness | `ls scripts/artifact-gate.sh tests/acceptance/project.json` | MET (both present, 2026-08-03) |
| The pipeline seams are as described (lower_to_mir → select → regalloc×2) | `rg -n 'fields.to_vec\(\)' src/target/shared/code/mir.rs` and `rg -n 'instruction.fields.clone\(\)' src/target/shared/code/regalloc/linear_scan.rs` | MET (mir.rs:370,380; linear_scan.rs:457) |
| The `-mir` capture / round-trip identity paths that reuse the pre-select stream | `rg -n 'route_function_through_mir\|capture_function\|lower_to_mir' src/target/shared/code/mir.rs` | MET (lower_to_mir@469, route_function_through_mir@585, capture_function@686; analysed in Phase 1) |

## Non-goals

- **Byte-identity absolute** across all four targets. The stream *contents* and
  emitted bytes must not change — only how the containers are allocated/reused.
- Do not change the MIR op set, fusion, selection, or the two-pass (Int/Fp)
  regalloc *decisions* (`vreg-alloc-order-load-bearing`, `bug387`).
- Do not regress the `-mir` dump or the `route_function_through_mir` identity pass.

## Design — measure first, then move/mutate instead of clone

The churn has two shapes; both are addressable by **moving or mutating instead of
cloning**, but each has a correctness constraint that must be checked first.

1. **`to_vec` clones at the MIR/select boundary (≈20%).** `lower_to_mir` borrows
   `&[CodeInstruction]` and `mir_fields_from_code` clones each fields Vec; `select`
   likewise clones out of `&[MirInstruction]`. If these boundaries **consumed their
   input by value** (`Vec<CodeInstruction>` / `Vec<MirInstruction>`), each fields
   Vec could be **moved** into the next instruction, not cloned. Constraint: the
   `-mir` dump (`capture_function`) and `route_function_through_mir` may need the
   pre-select stream after lowering — resolve by capturing/round-tripping from the
   moved-through value, or cloning only in the (rare, dump-enabled) capture path.

2. **The regalloc stream rebuild + `substitute` clone (≈13%, ×2 passes).**
   `linear_scan::run` builds a fresh `out` Vec and `substitute` clones each
   instruction's fields to rewrite the vreg operands. Two levers: (a) rewrite the
   vreg operands **in place** on a moved-in instruction (only those operands
   change; the rest of the fields Vec is untouched) rather than cloning the whole
   Vec; (b) the second (Fp) pass re-does the whole rebuild over the Int-colored
   stream — see whether the two passes can share one rebuild (one pass that colors
   both classes, or an in-place Fp rewrite) without changing the assignment.
   Constraint: spills/reloads insert instructions, so the stream length changes —
   the pass still produces a new stream, but the *unchanged* instructions can be
   moved through and only the vreg-bearing ones mutated in place.

3. **Immediate/field building (≈5%).** `.field("imm", &n.to_string())` allocates a
   `String` per immediate. A typed `Operand::Imm(n)` at the producer (`plan-78-B`
   started this) avoids it. Opportunistic; smaller.

## Phases

> Uncertainty first (measure each clone's real share), blast radius last. Keep
> boxes current; `artifact-gate … all` after each.

### Phase 1 — Attribute each clone precisely, and confirm the move-safety constraints

- [x] Re-run the attribution probe; record the exact est-alloc for `lower_to_mir`,
      `select`, `linear_scan::run` (and split `substitute`'s clone from the `out`
      Vec if possible with a targeted counter). This ranks the targets.
- [x] Determine whether `lower_to_mir`'s input can be consumed by value: who reads
      `self.instructions` after lowering, and whether the `-mir` capture /
      `route_function_through_mir` identity pass needs the pre-select stream.
      Record the constraint and the chosen move/clone-in-capture-only design.

**Measured target ranking** (deterministic per-site counters, uncommitted
`MFB_ALLOC_STATS` counting allocator + `fetch_add` at each clone site, over
`mfb test tests/acceptance`, 362/362; two runs, per-site counts identical):

| Clone site | fields-Vec clones | Phase |
|---|---|---|
| `substitute` (`linear_scan.rs:457`, run ×2 Int+Fp) | **21,420,760** | 3 |
| `mir_fields_from_code` (`lower_to_mir`) | **21,355,962** | 2 |
| `code_fields_from_mir` (`select`, host = aarch64) | **19,694,107** | 2 |

Combined **62.47M** fields-Vec clones; `total_allocs ≈ 479.6M` (the acceptance
run includes the test harness's in-process compiles; per-site counters are the
deterministic anti-guess signal). Ranking matches the plan's ≈13/10/10% estimate;
`substitute` leads because it clones **every** instruction in **both** regalloc
passes.

**Move-safety decision (recorded):**
- `lower_to_mir`'s input **can be consumed by value.** Every hot caller drops the
  input right after: `builder_registers.rs:141-142` (`self.instructions = select(...)`),
  `route_function_through_mir` (`function.instructions = select(...)`),
  `linux_gtk/mod.rs:662-664` (`*instructions = select(...)`). The **only** double
  read is the `-mir` capture at `builder_registers.rs:137-139`, which lowers a
  second time *before* the real lowering — and capture is rare (`-mir` dump only).
- **Chosen design:** add a by-value `lower_to_mir_owned(Vec<CodeInstruction>)` that
  **moves** each instruction's `fields` into the produced `MirInstruction` in the
  common (non-fused) case; keep the borrowing `lower_to_mir(&[…])` as a
  `.to_vec()` → owned wrapper for the rare capture path and the ~20 test callers
  (clone acceptable there). Fused/`addr_of`/shared-branch cases keep cloning:
  they need `setter.fields` for multiple outputs or slice-borrow it (minority).
- **`select` likewise:** all three backends funnel their common case through the
  shared `code_fields_from_mir(&…)`. To move, the owning loop must own the Vec →
  change `Backend::select` and each `select_<isa>` to take `Vec<MirInstruction>`
  by value; `neutral` is a dropped-after local at every call site, so it moves.
  Non-fused case moves `instruction.fields`; fused case still slices+clones.

Acceptance: a written, measured target ranking + the move-safety decision, in this
file. No code yet. — MET (above). Instrumentation is uncommitted (never staged).
Commit: 836bc00d1 (plan doc only; production code unchanged this phase)

### Phase 2 — Move `fields` through the MIR/select boundary instead of `to_vec`

- [x] Make `lower_to_mir`/`select` consume their input by value (or otherwise move
      each `fields` Vec into the produced instruction), so `mir_fields_from_code`/
      `code_fields_from_mir` move rather than `to_vec`-clone. Preserve the `-mir`
      capture and the round-trip identity (clone only where genuinely reused).

Implemented: added `lower_to_mir_owned(Vec<CodeInstruction>)` that moves each
non-fused instruction's `fields` Vec into the produced `MirInstruction`; the
borrowing `lower_to_mir(&[…])` now rebuilds an owned copy for the rare `-mir`
capture path and the ~20 unit-test callers (`CodeInstruction` is not `Clone`, so
it maps element-by-element). Changed `Backend::select` + `select_aarch64`/
`select_x86`/`select_riscv64` to consume `Vec<MirInstruction>` by value, moving
each non-fused field bag into the `CodeInstruction`. Hot callers move via
`std::mem::take`: `builder_registers.rs`, `route_function_through_mir`,
`linux_gtk/mod.rs`. Fused / `addr_of` / shared-branch / v128 cases keep cloning
(minority; slice-borrow or multi-use). Spec `15_x86_64-instruction-set.md`
signature updated.

**Measured (uncommitted per-site counters, `mfb test tests/acceptance`, 362/362):**

| Bucket | before | after | Δ |
|---|---|---|---|
| `mir_fields_from_code` clones | 21,355,962 | 3,049,653 | −85.7% |
| `code_fields_from_mir` clones | 19,694,107 | 1,387,798 | −93.0% |
| `substitute_fields_clone` (Phase 3) | 21,420,760 | 21,420,760 | unchanged |
| `total_allocs` | ≈479.6M | ≈387.4M | ≈−92M (−19%) |

The two boundary buckets fell from 41.05M → 4.44M combined fields-Vec clones
(the residue is exactly the fused/addr_of minority the design keeps).

Acceptance: `artifact-gate … all` 0 diffs (**1159 tests, 1301 builds, 1569
goldens, 0 diffs** — byte-identical across all 4 targets); `cargo test --bin mfb`
green (**3783 passed; 0 failed**); the `lower_to_mir`+`select` buckets fell (above).
— MET.
Commit: — (hash recorded next commit)

### Phase 3 — Regalloc: rewrite vreg operands in place, don't clone the whole stream

- [ ] In `linear_scan::run`/`substitute`, rewrite the vreg operands of a moved-in
      instruction **in place** (only the colored operands change) instead of
      `instruction.fields.clone()`; move unchanged instructions through. Keep the
      assignment bit-identical (the coloring, order, and spill/reload insertion are
      unchanged — this is a carrier/ownership change, not a decision change).
- [ ] Investigate collapsing the Int+Fp double rebuild (Phase 1's finding);
      implement only if byte-identical.

Acceptance: `artifact-gate … all` 0 diffs (the assignment must not move —
`vreg-alloc-order-load-bearing`); `cargo test`; the `linear_scan::run` bucket falls
(record); the regalloc unit tests (`sweep_equals_naive…`, spill tests) stay green.
Commit: —

### Phase 4 — (Opportunistic) typed immediates at the producer

- [ ] Where a producer builds `.field("imm", &n.to_string())`, emit
      `Operand::Imm(n)` instead (no `String`). Bounded; land only the clean sites.

Acceptance: `artifact-gate … all` 0 diffs; attribution shows the immediate
`to_string` share fall.
Commit: —

### Phase 5 — Measure the realized win

- [ ] Re-run the attribution + total-allocation counter + release/debug acceptance
      wall on `mfb test tests/acceptance`. Record total allocations before → after,
      and the summed stream-rebuild/clone class (was ≈28–33%) → after. This is the
      headline: a large total-allocation drop, verified against the counted cause.

Acceptance: total allocations fell substantially; the stream-rebuild/clone class is
materially reduced; both acceptance walls re-measured; byte-identical (0 diffs);
acceptance 362/362 on release and debug.
Commit: —

## Validation Plan

- Tests: `cargo test --bin mfb` (esp. the regalloc + mir round-trip modules).
- Byte-identity: `artifact-gate … all` 0 diffs after every phase — the assignment
  and every emitted byte must not move; a diff means a move/mutate changed a value.
- Cause verification (anti-guess guard): each phase must show its targeted
  attribution bucket shrink; code that moves but doesn't reduce the measured bucket
  did not fix the cause.
- Runtime proof: `mfb test tests/acceptance` exits 0 on release **and** debug (the
  in-place rewrite must not corrupt the stream).

## Open Decisions

- **Move vs clone-in-capture-only at the MIR boundary** (Phase 1) — governed by
  whether the `-mir` dump / round-trip identity pass needs the pre-select stream.
- **Collapsing the Int+Fp regalloc double-rebuild** (Phase 3) — only if provably
  byte-identical; otherwise leave the two passes and just remove the per-pass
  `fields.clone`.

## Corrections

<Filled in during execution.>

## Summary

plan-84 is the structural half of the real fix, and the larger one: ≈28–33% of
compile allocations are the pipeline rebuilding each function's instruction stream
and re-cloning every `fields` Vec five-to-six times. Moving `fields` through the
MIR/select boundary and rewriting vreg operands in place (instead of cloning) —
byte-identically — removes that churn. Like plan-83, its acceptance is a
*re-measured* drop in the exact counted buckets, and byte-identity is the guardrail
because none of this may change a single emitted byte or a single register
assignment.
