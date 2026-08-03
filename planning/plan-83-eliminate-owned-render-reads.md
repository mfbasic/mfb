# plan-83: Eliminate owned-`render()` operand reads in read-only codegen passes

Last updated: 2026-08-03
Effort: medium (1h–2h)
Depends on: nothing (builds on the typed `Operand` merged with plan-79/82; needs no
new representation). Independently landable.

Stop the read-only codegen passes from allocating a `String` for every operand
they only *look at*. Several late passes read operands through
`CodeInstruction::get()` (which returns `Option<String>` — it **renders, i.e.
allocates**) or a direct `value.render()`, then throw the string away after a
compare/parse. A borrowing read (`CodeInstruction::operand() -> Option<&Operand>`,
already present, or `value.rendered()` which lends a `Raw`/`Phys` its `&str`) does
the same job with **no allocation** for the common operand kinds.

## Why this plan exists — the *counted* cause

This is grounded in a **measured allocation-cause attribution**, not a guess (the
mistake plan-82's spike made). Method: a sampling global allocator that captures a
backtrace on 1-in-2048 allocations and buckets each by its nearest `mfb` frame,
run over `mfb test tests/acceptance` (16 files, the acceptance compile). It counts
*which call sites cause the allocations*, not just where self-time is spent.

Result (total ≈595M allocations): **`Operand::render()` called from read-only
passes is ≈25% of ALL compile allocations**, spread across:

| Est. share | Call site (nearest `mfb` frames) |
|---|---|
| 8.0% | `Operand::render` ← `arch::aarch64::encode::sizing::instruction_size` |
| 7.0% | `Operand::render` ← `codegen_utils::finalize_frame` |
| 4.1% | `Operand::render` ← `peephole::forward_stores_to_loads` |
| 3.2% | `Operand::render` ← `fma_fusion::fuse_scalar_fma` |
| 1.5% | `Operand::render` ← `regalloc::find_physical_operand` |
| 0.9% | `Operand::render` ← `regalloc::analysis::build_cfg` (label reads) |
| 0.6% | `Operand::render` ← `validation` |

These passes do not *produce* code from the string — they read a field to compare
(`== "sp"`, a mnemonic, a label), to parse an index, or to size an instruction,
and discard it. The allocation is pure waste.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| Typed `Operand` with `rendered()` (borrow) + `operand()` accessor exist | `rg -n 'fn rendered' src/target/shared/code/operand.rs` and `rg -n 'fn operand\(' src/target/shared/code/code_impl.rs` | MET |
| Byte-identity oracle + acceptance harness | `ls scripts/artifact-gate.sh tests/acceptance/project.json` | MET |
| The attribution probe is reproducible | see "Why this plan exists" (re-add the sampling allocator to `main.rs`, `MFB_ATTR_ALLOCS=1`) | MET (used to author this plan) |

## Non-goals

- **Byte-identity absolute** (`artifact-gate … all` = 0 diffs). These are read-only
  changes; `rendered()`/`operand()` see the same value `get()`/`render()` did.
- No representation change; no new `Operand` arm.
- Do not touch passes that legitimately need an owned `String` (a stored label, a
  diagnostic message) — only the read-compare-discard sites.

## Current State + the `rendered()` caveat (READ THIS)

`CodeInstruction::get(name) -> Option<String>` renders (owned). `operand(name) ->
Option<&Operand>` borrows. `Operand::rendered() -> Cow` borrows **only for `Raw`
and `Phys`**; for `VReg` and `Imm` it must *format* (`%vN` / the decimal), so it
still allocates. Therefore the fix is **per-site, by which operand kinds reach it**:

- **Post-regalloc passes** (`finalize_frame`, `peephole`, encoder `sizing`,
  `fma_fusion`): operands are `Phys`/`Raw`/`Imm`. Swapping `get()`/`render()` →
  `rendered()` (or `operand()` + a typed match) makes the `Phys`/`Raw` reads
  allocation-free — the bulk. (`Imm` reads still format; rare.) This is where most
  of the 25% lives (sizing 8% + finalize 7% + peephole 4% + fma 3% = 22%).
- **Pre-regalloc passes** (`find_physical_operand`, `validation`, `analysis`
  label reads): operands include `VReg`, whose `rendered()` still allocates. Here
  the win requires **matching the typed operand** (e.g. compare against
  `Operand::VReg{..}` / read the id) instead of rendering to `%vN` and
  string-comparing. Do this only where it stays byte-identical and readable; a
  `VReg` label/name field that must be a string can stay (small residue, noted).

## Design

For each cause site: identify what the string is used for, then pick the
allocation-free read:

1. **Compare to a literal** (`value == "sp"`, `mnemonic == "b.eq"`): `Operand`
   already `impl PartialEq<str>` — compare the `&Operand`/`Cow` directly, no
   render.
2. **Parse an index / physical name**: read via `operand()` and match `Phys{index}`
   / `Raw`, or `rendered()` (borrow) then parse — no owned `String`.
3. **Size an instruction** (`sizing::instruction_size`'s `probe.imports.insert(
   value.render(), …)`): use `value.rendered()` (borrow) — the probe only needs a
   `&str` key; if `imports` needs owned keys, key only the symbol/label fields
   (which are `Raw`), not every operand.
4. **Label/branch target read**: use `rendered()` (labels are `Raw` → borrow).

Correctness rests on `rendered()`/`operand()` yielding the exact value `render()`
did; byte-identity is the guardrail.

## Phases

> Land per pass; run `artifact-gate … all` after each; keep boxes current.

### Phase 1 — The two biggest, cleanest post-regalloc sites

- [ ] `arch::aarch64::encode::sizing::instruction_size` (8%): stop rendering every
      field into the `probe.imports` map — key only the fields that need it, via
      `rendered()`; or size without the owned map.
- [ ] `codegen_utils::finalize_frame` (7%): read operands via `operand()`/
      `rendered()` instead of `get()`/`render()`.

Acceptance: `artifact-gate … all` 0 diffs; `cargo test --bin mfb` green; re-run the
attribution probe and confirm the `sizing`+`finalize_frame` `Operand::render`
buckets are gone/greatly reduced (record before/after est-alloc numbers here).
Commit: —

### Phase 2 — peephole + fma_fusion (post-regalloc)

- [ ] `peephole::forward_stores_to_loads` (4%) and `fma_fusion::fuse_scalar_fma`
      (3%): borrowing reads / typed matches.

Acceptance: `artifact-gate … all` 0 diffs; `cargo test`; attribution shows these
buckets reduced (record numbers).
Commit: —

### Phase 3 — pre-regalloc reads (typed-match where `VReg` appears)

- [ ] `regalloc::find_physical_operand` (1.5%), `regalloc::analysis` label reads
      (0.9%), `validation` (0.6%): where the operand is `VReg`, match the typed
      arm instead of rendering; where it is `Raw`, use `rendered()`. Note any
      `VReg`-string residue that must stay.

Acceptance: `artifact-gate … all` 0 diffs; `cargo test`; attribution numbers
recorded.
Commit: —

### Phase 4 — Measure the realized win

- [ ] Re-run the sampling attribution and the total-allocation counter on
      `mfb test tests/acceptance`. Record: total allocations before (595–577M) →
      after; the summed `Operand::render`-from-read-passes share (was ≈25%) → after.
      Expect a material total-allocation drop and the read-pass render buckets near
      zero (leaving only the genuine `Imm`/label residue).

Acceptance: total allocations fell measurably; the ≈25% `Operand::render`
read-pass class is largely eliminated; release/debug acceptance wall re-measured
and recorded; byte-identical; acceptance 362/362.
Commit: —

## Validation Plan

- Tests: `cargo test --bin mfb`.
- Byte-identity: `artifact-gate … all` 0 diffs after every phase (a diff = a read
  that wasn't actually read-only — investigate, never re-baseline).
- Cause verification (the anti-guess guard): the attribution probe MUST show the
  targeted buckets shrink; a phase that changes code but not the measured bucket
  did not fix the cause.
- Runtime proof: `mfb test tests/acceptance` exits 0 (release).

## Corrections

<Filled in during execution.>

## Summary

plan-83 is the cheap, high-confidence half of the real fix: ≈25% of compile
allocations are `String`s rendered by passes that only read an operand and throw
it away. A borrowing read (`operand()`/`rendered()`) or a typed compare removes
them, byte-identically. Its acceptance is a *re-measured* drop in the exact
buckets — so, unlike plan-82's spike, the fix is verified against the counted
cause, not assumed.
