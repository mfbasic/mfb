# bug-387: the "neutral" MIR stream carries AArch64 physical register names, forcing every non-AArch64 backend to un-AArch64 the stream before encoding

Last updated: 2026-07-25
Effort: x-large (1d–3d)
Severity: LOW
Class: Footgun / Other (drifted abstraction)

Status: Open
Regression Test: the three encoder suites
(`src/arch/{aarch64,x86_64,riscv64}/encode/tests.rs`) plus
`scripts/artifact-gate.sh` at `diffs=0` and one full `scripts/test-accept.sh`
per target — the fix must be **byte-identical** everywhere.

The MIR stream that the shared lowering hands to each backend is documented as
architecture-neutral, but it is not: it still spells physical registers with
AArch64 names — `xN`, `wN`, `dN`, `sN`, `sp`, `lr`, `xzr`. Every non-AArch64
backend therefore carries a hand-tuned layer whose entire job is to *un-AArch64*
the stream before it can encode. The abstraction leak is not hypothetical — one
of those layers reconstructs, with a forward fixpoint dataflow analysis, the
call/syscall/return ABI-role context that the "neutral" stream discarded
upstream. An abstraction that needs a downstream fixpoint to recover information
it threw away upstream is leaky by definition.

There is **no observable runtime miscompile** here today — both remap layers are
correct and every golden is byte-identical. The defect is structural: a growing
per-ISA translation burden, two divergent implementations of the same concept,
and a latent-miscompile hazard class (a stray AArch64 spelling silently realizes
as the wrong physical register on another ISA — see the RISC-V `x31`→`t6`
comment cited below). The single correct outcome of a fix is that the stream
handed to backends names ABI/scratch registers with **ISA-neutral role tokens**,
each backend realizes those tokens to its own spellings at selection time, and
**no AArch64 register spelling survives in the stream** — with every emitted
byte unchanged.

This is the D5 item carved out of **bug-341** (`src/arch/` cleanup cluster),
which explicitly declined to fix it: *"The fix is a real design change … which
is precisely what plan-34-B Phase 4 attempted and bug-85 reverted (D2). It needs
its own plan, not a cleanup bug."* bug-341's Non-goals list "the neutral-stream
design itself (D5)" as out of scope. This document is that plan. **bug-341 is
unchanged by this file.**

References:

- `bugs/bug-341-arch-encoder-cleanup.md` — item **D5** (context) and **D2** (the
  `select_aarch64` comment that promises the Phase-4 deletion this bug either
  delivers or the comment stays permanent).
- `bugs/completed-bugs/bug-85-x86-entry-runtime-arg-staging-tokens.md` — the
  prior attempt: plan-34-B Phase 4 realized role tokens directly, broke every
  x86-64 program, and was reverted. Its follow-up is left explicitly OPEN. **Any
  fix here must succeed where that one failed.**
- `planning/old-plans/plan-34-B-role-named-registers.md` — still reads
  "STATUS: COMPLETE (2026-07-10)" with Phase 4 listed as landed, which
  contradicts the reverted reality in the code (bug-341 D2).
- `src/docs/spec/architecture/` — the spec pages describing the neutral MIR /
  register-role vocabulary a fix must keep in sync.

## Failing Reproduction

There is no program that mis-runs today; the "reproduction" is the drift itself,
directly observable in the source at HEAD. Two symptoms, both verified against
the current worktree (line numbers are HEAD; bug-341 D5's numbers predate later
edits and have drifted).

**Symptom 1 — the same conceptual operation has two divergent signatures.**
`map_scratch_register` maps a neutral scratch index to a physical register name,
and each backend spells it differently for no reason but drift:

```
src/arch/x86_64/select.rs:36   fn map_scratch_register(n: usize) -> &'static str
src/arch/riscv64/select.rs:624 fn map_scratch_register(n: usize) -> String
```

Neighboring per-ISA "un-AArch64" helpers, all present at HEAD:

| Concern | x86_64 (`select.rs`) | riscv64 (`select.rs`) |
| --- | --- | --- |
| scratch map | `map_scratch_register` `:36` | `map_scratch_register` `:624` |
| ABI-register map | `map_abi_register` `:123` | `map_fp_register` `:645` |
| whole-stream remap | `remap_x86_abi` `:162` | `remap_riscv_abi` `:664` |
| register-string rewrite | `abi_boundary_of` `:23` | `remap_register` `:682` |

**Symptom 2 — one backend runs a fixpoint dataflow analysis to recover ABI-role
context the "neutral" stream discarded.** `remap_x86_abi`
(`src/arch/x86_64/select.rs:162`) contains, at `:270–303`, a forward fixpoint
over the instruction list:

```rust
let mut boundary_before: Vec<Option<AbiBoundary>> = vec![None; count];
let mut changed = true;
while changed {
    changed = false;
    for i in 0..count {
        …
        if new_val != boundary_before[i] { boundary_before[i] = new_val; changed = true; }
    }
}
```

Its sole purpose is to reconstruct which instructions sit at a call / syscall /
return boundary — information the shared lowering had and the neutral stream
dropped, forcing x86 to re-derive it by hand.

Both remap layers operate on **literal AArch64 register strings**, e.g.
`src/arch/riscv64/select.rs:682` (`remap_register`):

```rust
"xzr" => return Some(ZERO.to_string()),
"lr"  => return Some("ra".to_string()),
// … `xN`/`wN` parsed below
```

with a load-bearing comment that names the exact hazard this design keeps live:
on RISC-V `x31` is a *real* register (`t6`), so a stray AArch64 `"x31"` in the
"neutral" stream would silently realize as `t6` instead of `zero` — a
miscompile the narrowing there deliberately forecloses. That comment is the
proof the leak is a footgun, not just cosmetics.

- Observed: the stream handed to backends carries AArch64 physical spellings;
  x86 and riscv64 each carry a bespoke remap layer, one of which runs a fixpoint
  to rebuild discarded ABI context, and the two layers have drifted (divergent
  signatures, one string-rewrite convention each).
- Expected: the stream names registers with ISA-neutral role/scratch tokens;
  each backend realizes them to its own spellings at selection; no AArch64
  spelling and no downstream role-recovery analysis is needed.

Contrast case (why this is hard, not just tedious): the AArch64 backend is
*correct today* precisely because the stream already speaks its dialect —
`src/arch/aarch64/select.rs:87–91` "realizes the plan-34-B role tokens … to
their AArch64 register spellings … Phase 4 deletes this and realizes tokens
directly." Phase 4 did exactly that and broke x86-64 (bug-85). Any fix must
keep all three backends byte-identical, not just make the code prettier.

## Root Cause

The shared lowering emits physical AArch64 register names into the MIR stream
instead of neutral role tokens (`%arg`/`%ret`/`%sysnr`/scratch indices/invariant
tokens). AArch64 is the reference backend, so its selection consumes the stream
directly and looks clean; every other backend must first translate the AArch64
dialect back into neutral roles and then into its own dialect. Because the role
context is *not* encoded in the tokens, x86 cannot do a local rewrite — it must
recover call/ret/syscall boundaries with the `:270–303` fixpoint before it can
choose the right physical register. riscv64 gets away with a mostly-local
`remap_register` string match but still hard-codes the AArch64 spellings it must
recognize.

The design change that removes the root cause — role tokens realized per-ISA at
selection, no AArch64 spelling in the stream — is what plan-34-B Phase 4
implemented and bug-85 reverted after it broke every x86-64 program. So the root
cause is understood *and* has one confirmed-dangerous fix path; the engineering
problem is doing it without repeating bug-85.

## Goal

- The MIR stream handed to backend selection contains **zero** AArch64 physical
  register spellings (`xN`/`wN`/`dN`/`sN`/`sp`/`lr`/`xzr`) used as neutral
  tokens; register roles are carried as ISA-neutral tokens.
- `remap_x86_abi`'s fixpoint (`src/arch/x86_64/select.rs:270–303`) is **deleted**
  because the ABI-role context is no longer discarded and needs no recovery.
- The per-ISA "un-AArch64" helpers converge: no two backends carry divergent
  signatures for the same conceptual map (`map_scratch_register` et al.), or the
  layer is removed entirely where the tokens make it unnecessary.
- The RISC-V `x31`→`t6` hazard is structurally impossible: no AArch64 spelling
  can reach `remap_register` because none is emitted.
- **Every emitted byte is byte-identical** to today's committed goldens on all
  three ISAs; `scripts/artifact-gate.sh` stays at `diffs=0`.

### Non-goals (must NOT change)

- **Any emitted byte.** This is a representation change upstream of encoding; a
  single changed instruction encoding on any ISA is a failed change. This is the
  same bar bug-341 sets and the exact bar bug-85's Phase 4 attempt failed.
- Instruction selection *decisions* — which `CodeOp` is chosen for a given MIR
  op. Only how registers are *named* in the stream changes.
- The `EncodedImage` field set, relocation `kind`/`binding` values, and the
  linker's view of them.
- The AArch64 backend's observable behavior. It currently reads the stream
  directly; after the change it realizes the same neutral tokens to the same
  spellings — same bytes out.
- **The tempting wrong fix, named and forbidden:** re-landing plan-34-B Phase 4
  as-was. It is reverted *because it broke x86-64* (bug-85); repeating it without
  first proving each backend byte-identical is the exact failure this plan
  exists to avoid. Do not resurrect commit `c098504f`'s approach unqualified.
- Do **not** paper over the leak by merely unifying the two `map_scratch_register`
  signatures while leaving AArch64 spellings in the stream — that removes a
  cosmetic divergence, not the root cause, and leaves the fixpoint and the
  `x31`→`t6` hazard in place.

## Blast Radius

Found by searching `src/arch/**/select.rs` and the shared lowering; classify
before implementing (this list is the audit *seed*, to be completed in Phase 1):

- `src/arch/x86_64/select.rs` — `remap_x86_abi` (`:162`), the fixpoint
  (`:270–303`), `map_scratch_register` (`:36`), `map_abi_register` (`:123`),
  `abi_boundary_of` (`:23`) — **fixed by this bug** (the primary remap layer).
- `src/arch/riscv64/select.rs` — `remap_riscv_abi` (`:664`),
  `map_scratch_register` (`:624`), `map_fp_register` (`:645`), `remap_register`
  (`:682`) — **fixed by this bug** (the second remap layer).
- `src/arch/aarch64/select.rs:87–91` — the token-realization seam and its
  bug-341-D2 comment — **fixed by this bug**: it becomes one of three symmetric
  per-ISA realizers, and the "Phase 4 deletes this" comment is resolved (either
  the deletion lands, or if scoped out, D2's rewording stands).
- The shared lowering emitting the AArch64 spellings — **the actual source**;
  must be located in Phase 1 (grep the emitters of `xN`/`sp`/`lr`/`xzr` into the
  neutral stream) and is where the token emission moves to.
- `planning/old-plans/plan-34-B-role-named-registers.md` — its "COMPLETE / Phase
  4 landed" status is already wrong (bug-341 D2); this bug either makes it true
  again or leaves D2 to correct it. **Doc only.**
- `bugs/completed-bugs/bug-85-…md` — its OPEN follow-up is closed by this bug
  landing successfully. **Doc only.**

## Fix Design

The shape is plan-34-B Phase 4's *intent* — neutral role tokens realized
per-ISA — executed with the byte-identity discipline that attempt lacked. The
risk is concentrated entirely in one place: the token→spelling realization must
produce, for AArch64, exactly the strings the encoder sees today, and for x86
and riscv64 exactly the strings their remap layers produce today, so that no
byte moves. That is why this needs a phased plan with a per-backend
byte-identity gate at every step, not a single cutover.

Sequencing rationale (mirrors why bug-341 puts the encoder-test guard first):
the AArch64 path is the reference and must be converted *last* or behind the
strongest guard, because it is the one where the stream and the consumer are
currently in lockstep and a mistake is silent. Convert the stream to emit
neutral tokens while keeping all three backends' realizers producing today's
exact spellings; only once all three are proven byte-identical can the fixpoint
and the divergent helpers be simplified away.

Rejected alternative: unify the two `map_scratch_register` signatures and stop
(the cosmetic fix). Rejected — it leaves AArch64 spellings in the stream, the
fixpoint intact, and the `x31`→`t6` hazard live; it fixes the *tell*, not the
*leak*.

Rejected alternative: re-apply commit `c098504f` (plan-34-B Phase 4) directly.
Rejected — it is the reverted regression (bug-85); it must be re-derived under
the byte-identity gate, not cherry-picked.

Expected output shift: **none**, on every ISA. Any diff in emitted bytes is a
defect in the change.

## Phases

### Phase 1 — locate the source + full audit (no behavior change)

- [ ] Grep the shared lowering for every site that emits an AArch64 physical
      spelling (`xN`/`wN`/`dN`/`sN`/`sp`/`lr`/`xzr`) into the neutral stream;
      record each with a verdict. This is the real root-cause site the two remap
      layers exist to compensate for.
- [ ] Complete the Blast Radius audit above: for each remap helper, record what
      neutral token would replace the spelling it currently rewrites, and the
      exact spelling each backend must still emit (the byte-identity contract).
- [ ] Record a clean `scripts/artifact-gate.sh <exe>` baseline at `diffs=0` on
      each target as the reference for every later phase.

Acceptance: the emission sites are enumerated; each remap helper has a
token-mapping table; a `diffs=0` baseline exists per target.
Commit: —

### Phase 2 — introduce neutral tokens behind identical realizers

- [ ] Extend the token vocabulary (per plan-34-B's `%arg`/`%ret`/`%sysnr`/
      scratch/invariant tokens) and have the shared lowering emit tokens instead
      of AArch64 spellings.
- [ ] Give each backend a realizer that turns tokens into **exactly today's
      spellings** — AArch64 into `xN`/…, x86 into what `remap_x86_abi` produces,
      riscv64 into what `remap_register`/`map_*` produce. No simplification yet.
- [ ] Gate: `scripts/artifact-gate.sh` at `diffs=0` after **each** backend is
      converted; byte-identical or revert that backend.

Acceptance: all three backends consume tokens, not AArch64 spellings; every
golden byte-identical; `cargo build` warning-free.
Commit: —

### Phase 3 — delete the compensation layers the tokens made dead

- [ ] Delete `remap_x86_abi`'s fixpoint (`:270–303`) and any role-recovery it
      fed, now that boundaries arrive as tokens.
- [ ] Converge/remove the divergent `map_scratch_register` /
      `map_abi_register` / `map_fp_register` / `remap_register` helpers.
- [ ] Resolve the `src/arch/aarch64/select.rs:87–91` comment (delete the seam or
      restate it) and reconcile `planning/old-plans/plan-34-B-…md` and the
      bug-85 follow-up with reality.
- [ ] Gate: `artifact-gate.sh` at `diffs=0` after each deletion.

Acceptance: no fixpoint, no divergent per-ISA remap helpers, no AArch64 spelling
reachable by `remap_register`; goldens byte-identical.
Commit: —

### Phase 4 — full validation

- [ ] `scripts/test-accept.sh` full run on macOS and Linux, both arches (this
      layer feeds machine-code emission, so running the produced binaries is not
      optional).
- [ ] `cargo test`, `cargo clippy`, `cargo fmt --check`.
- [ ] Confirm every committed binary golden byte-identical; `git status` shows
      zero modified files under any `tests/**/golden/`.

Acceptance: full suite green on every target; no golden moved.
Commit: —

## Validation Plan

- Regression tests: the three encoder suites are the primary structural guard.
  Note the dependency on **bug-341 C1** — the x86 encoder suite has assertion-
  free calls that cannot detect an encoding regression until C1 lands; ideally
  bug-341's Phase 1 (C1) lands before this bug's Phase 2 converts x86, so the
  byte-identity gate on x86 is real and not nominal.
- Runtime proof: `scripts/artifact-gate.sh` at `diffs=0` on every commit is the
  proof for an output-preserving change, plus one full `scripts/test-accept.sh`
  per target before merge — bug-85 proves that a green type-check is *not*
  sufficient here; the produced binaries must run.
- Byte-identity guard: zero modified files under any `tests/**/golden/`; if a
  golden moves, the change is wrong and is reverted, not re-baselined.
- Doc sync: `src/docs/spec/architecture/` register-role vocabulary;
  `planning/old-plans/plan-34-B-role-named-registers.md`; the bug-85 follow-up.
- Full suite: `cargo test`, `scripts/test-accept.sh` (all targets),
  `cargo clippy`.

## Open Decisions

- **Sequencing vs. bug-341 C1.** Recommended: land bug-341 C1 (real x86 byte
  assertions) before converting x86 here, so the byte-identity gate on x86 is
  trustworthy. Alternative: proceed on `artifact-gate.sh` alone, accepting that
  the x86 encoder unit suite is a weak guard until C1.
- **AArch64 conversion order.** Recommended: convert AArch64 *last / behind the
  strongest guard* since it is the reference dialect and a mistake there is
  silent. Alternative: convert it first to flush latent assumptions early,
  accepting higher risk.
- **plan-34-B / bug-85 ownership.** Recommended: this bug closes bug-85's OPEN
  follow-up and re-annotates plan-34-B on landing. Alternative: leave bug-341 D2
  to reword the comment and keep plan-34-B annotated as reverted if this design
  change is deferred.

## Summary

The MIR stream billed as architecture-neutral still speaks AArch64 —
`xN`/`dN`/`sp`/`lr`/`xzr` — so x86-64 and riscv64 each carry a bespoke layer to
un-AArch64 it before encoding, and one of those layers runs a forward fixpoint
dataflow analysis to rebuild the ABI-role context the "neutral" stream threw
away. The two layers have already drifted (divergent `map_scratch_register`
signatures; one string-rewrite convention each), and the design keeps a real
latent-miscompile hazard live (a stray AArch64 `x31` would realize as RISC-V
`t6`, not `zero`). There is no runtime failure today; the defect is structural.

The fix is the design change plan-34-B Phase 4 attempted — neutral role tokens
realized per-ISA, no AArch64 spelling in the stream — but that attempt broke
every x86-64 program and was reverted (bug-85), so the whole engineering problem
is doing it under a per-backend byte-identity gate this time. The real risk is
concentrated in the token→spelling realizers reproducing today's exact bytes on
all three ISAs; everything downstream (deleting the fixpoint, converging the
divergent helpers) is mechanical once the stream is proven byte-identical.

This is bug-341's D5, carved out per its own instruction that it "needs its own
plan, not a cleanup bug." **bug-341 is left unchanged.**
