# bug-350: x86-64 reuses the AArch64 caller-saved clobber mask — `xmm8`–`xmm14` are modeled as call-surviving when SysV makes every xmm volatile (LATENT miscompile)

Last updated: 2026-07-18
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness (register allocation; latent miscompile + measured spill cost)

Status: Fixed (2026-07-19)
Regression Test:
- `target::shared::code::regalloc::tests::call_clobber_masks_match_each_targets_abi`
  — the per-ISA table test, asserting the DERIVED mask against each ABI's truth
  spelled out independently, and asserting explicitly that the inherited AArch64
  FP mask leaves bits 8–14 clear while x86's sets them.
- `target::shared::code::regalloc::tests::x86_fp_values_live_across_a_call_avoid_the_volatile_high_xmm`
  — nine call-spanning FP vregs on x86; **verified to FAIL against the pre-fix
  allocator** (it colored one onto `xmm8`) and to pass after.

`call_clobber_mask` selects its caller-saved register masks with
`if is_riscv { RISCV_… } else { … }`. There is no x86 branch, so **x86-64 uses the
AArch64 masks**. The masks are indexed by physical-register *number*, and the two
ISAs number their registers differently, so the AArch64 constants describe the
wrong register set on x86.

The integer half is safe: every SysV caller-saved GPR lands inside the AArch64
mask's bit range, so nothing is under-approximated. The **FP half is not**.
`CALLER_SAVED_FP = 0xffff_00ff` clears bits 8–15 because AArch64's `d8`–`d15` are
callee-saved by the PCS. On x86-64 those bit positions are `xmm8`–`xmm15`, and
SysV has **no callee-saved xmm at all** — every one is destroyed by a `call`. So
the allocator believes `xmm8`–`xmm14` survive a call when they do not, and will
happily color a value live across a call onto one of them. Nothing else rescues
it: `X86_64RegisterModel::is_callee_saved` returns only the integer table
(`src/arch/x86_64/regmodel.rs:92-95`), so the frame does not save these registers
either.

**This is LATENT, not a live miscompile.** A CFG-aware scan of 224 x86-64 code
dumps built from the whole `tests/rt-behavior` suite found **zero** cases where an
`xmm8`–`xmm14` value is live across a call that actually executes on the path
between its definition and its use (§Failing Reproduction). The registers *are*
allocated — `xmm8` appears in 12 of the 224 dumps, `xmm9`–`xmm14` in 7–8 each —
but in every observed case the only intervening calls sit on error tails that
`ret` rather than rejoin. The bug fires the moment a real call lands between the
def and the use of such a value.

The single correct behavior a fix produces: `call_clobber_mask` returns the
target's **actual** caller-saved set — on x86-64, all 16 xmm registers and the
nine SysV volatile GPRs — so no value is colored into a register the callee
destroys, and no value is needlessly spilled out of one the callee preserves.

References:

- `src/arch/x86_64/regmodel.rs:63-64` — the model already states the rule the
  allocator violates: "SysV makes every xmm caller-saved, so a float live across a
  `call` must spill (there is no callee-saved bank)."
- `planning/old-plans/plan-00-H-x86_64-backend.md` (the x86-64 backend plan).
- Found during the cleanup review, agent 11 (arch encoders), INCIDENTAL item;
  filed after triage because the reviewer flagged it "possible live miscompile,
  NEEDS TRIAGE".

## Failing Reproduction

There is no runtime failing reproduction — that is the finding. The evidence is
(a) the mask arithmetic, which is decisive, and (b) a corpus scan showing the
hazardous registers are in active use but never yet in the fatal position.

### (a) The mask is wrong — arithmetic

`CALLER_SAVED_FP = 0xffff_00ff` (`src/target/shared/code/regalloc/analysis.rs:69`)
against the x86 FP index map (`analysis.rs:245`, `fp_physical_index`: `xmm0`–`xmm15`
→ `0..=15`) and `FP_REGS` (`src/arch/x86_64/regmodel.rs:67-70`, allocatable =
`xmm0`–`xmm14`):

| xmm | bit | in mask (modeled clobbered) | SysV truth | verdict |
| --- | --- | --- | --- | --- |
| `xmm0`–`xmm7` | 0–7 | yes | caller-saved | correct |
| `xmm8`–`xmm14` | 8–14 | **no** | **caller-saved** | **UNDER-APPROXIMATED — miscompile** |
| `xmm15` | 15 | no | caller-saved | reserved FP scratch, not allocatable — moot |

The integer side, for contrast (`CALLER_SAVED_INT = 0x3_ffff`, `analysis.rs:58`;
x86 index map at `analysis.rs:212-215`; `INT_ALLOCATABLE` at
`src/arch/x86_64/regmodel.rs:43`):

| reg | bit | in mask | SysV truth | allocatable | verdict |
| --- | --- | --- | --- | --- | --- |
| `rax`,`rcx`,`rdx`,`rsi`,`rdi`,`r8`,`r9` | 0,1,2,6,7,8,9 | yes | caller-saved | no | correct |
| `r10`,`r11` | 10,11 | yes | caller-saved | yes | correct |
| `r12` | 12 | yes | **callee-saved** | yes | over-approximated — wasted spill |
| `r14` | 14 | yes | **callee-saved** | yes | over-approximated — wasted spill |
| `rbx`,`rbp`,`r13`,`r15` | 3,5,13,15 | yes | callee-saved | no | reserved — moot |

No SysV caller-saved GPR falls outside the mask, so **the integer half cannot
miscompile**. It is purely over-conservative — and expensively so: with only four
allocatable GPRs, marking two of them (`r12`, `r14`) clobbered means *every* one of
x86's four integer registers is considered destroyed across any call, forcing a
spill for any integer value live across one.

### (b) The hazardous registers are live, but never yet fatally placed

```
# build the whole rt-behavior suite for x86-64 and dump native code
while read p; do (cd "$p" && mfb build --target linux-x86_64 --ncode .); done < projects.txt
# CFG-aware scan: xmm8-14 defined, then USED after a call reachable on a real path
```

- Observed: 224 dumps scanned, **0 hits**. `xmm8` allocated in 12 dumps,
  `xmm9`–`xmm14` in 7–8 each.
- Expected (once the fix lands): still 0 hits, but for a sound reason — the
  allocator no longer treats those registers as call-safe.

A representative near-miss, from a two-float function with a mid-body call
(`fmul_d dst=xmm8` at index 8; `bl _mfb_make_error_result` at 319; `fmov_x_from_d
src=xmm8` at 422 under label `float_result_finite_1`): the linear interval spans
the call, so the allocator consulted the clobber mask, found bit 8 clear, and
chose `xmm8` **because** it believed the register survives. It is correct here only
because instruction 319 is on an error tail that returns at 323 and never reaches
422. The allocator's reasoning was wrong; the outcome was lucky.

Contrast cases that are correct today:

- AArch64 — the masks are its own; `d8`–`d15` genuinely are callee-saved.
- rv64 — `is_riscv` selects `RISCV_CALLER_SAVED_FP = 0xf003_fcff`, which correctly
  covers `ft0`–`ft7`, `fa0`–`fa7`, `ft8`–`ft11`.
- x86 integer class — over-approximated, so correct (slow, not wrong).
- The `_mfb_*` runtime-helper arm (`analysis.rs:133-141`) uses `all_int`
  (`0x7fff_ffff`), which covers all 16 x86 GPRs — conservative and correct.
  Its FP arm still uses the broken `caller_saved_fp`.

## Root Cause

`src/target/shared/code/regalloc/analysis.rs:90-107` (`call_clobber_mask`) branches
only on `is_riscv`. The `ClassModel` it reads carries `is_riscv`
(`analysis.rs:50-54`) but no x86 discriminator, and `regalloc::mod.rs:263-275`
computes both `is_riscv` and `is_aarch64` from `model.arena_base()` yet passes only
`is_riscv` down. So x86 falls into the `else` arm and inherits AArch64's constants.

The masks are indexed by physical-register number
(`int_concrete_physical_index`/`fp_physical_index`, `analysis.rs:204-227`,
`245-259`), which is exactly why an ISA-specific constant cannot be shared: the
same bit means `d8` on AArch64 and `xmm8` on x86.

The consumer is a hard exclusion, not a hint —
`src/target/shared/code/regalloc/linear_scan.rs:142-147` refuses any physical whose
bit is set in `clobbered`, and takes the first survivor in `allocatable` order.
Because `FP_REGS` is listed in plain `xmm0..xmm14` order
(`src/arch/x86_64/regmodel.rs:67-70`), once bits 0–7 are excluded the *first*
candidate is `xmm8` — so the wrong bits don't merely permit the hazardous choice,
they steer directly into it. (AArch64's `FP_ALLOCATABLE` deliberately orders the
callee-saved `d8`–`d15` **last**, `src/arch/aarch64/regmodel.rs`; x86 has no such
ordering because it has no callee-saved bank to order.)

Why the reviewer's related lead holds, but narrowly: `INT_ALLOCATABLE` really is
only 4 registers (`r10`, `r11`, `r12`, `r14`) versus AArch64's 14 and rv64's 12,
and the "plan-00-H §7 frees a register by moving arena_base to TLS" comment at
`src/arch/x86_64/regmodel.rs:10` and `:33` cites a section that **does not exist** —
`planning/old-plans/plan-00-H-x86_64-backend.md` has sections 1–6 and phases 1–4,
no §7. But the plan does *not* claim the move landed; its Phase 4 status line
records the opposite, explicitly and deliberately: "`arena_base` remains pinned to
r15 (works; the optional TLS move is a perf refinement, not a gap — r15 is a valid
callee-saved home and no test needs it freed)." So this is a **stale comment
pointing at a nonexistent section**, not a completeness claim that was violated.
Fold it in as a doc fix, not a defect. (The same comment's "versus AArch64's 19" is
also wrong — AArch64 has 14.)

## Goal

- `call_clobber_mask` is driven by the target's real caller-saved set, so on
  x86-64 all 16 xmm bits are set and exactly the 9 SysV volatile GPR bits are set.
- No allocatable x86 register that SysV preserves is reported clobbered
  (`r12`, `r14` become usable across a PCS call).
- No allocatable x86 register that SysV destroys is reported surviving
  (`xmm8`–`xmm14` are no longer call-safe homes).

### Non-goals (must NOT change)

- AArch64 and rv64 mask values and the code they produce — this fix must be
  **byte-identical** on those two targets. Any AArch64 golden churn means the fix
  is wrong.
- The `_mfb_*` runtime-helper `all_int` conservatism (`analysis.rs:133-141`).
- The reserved-register set, `INT_ALLOCATABLE` membership, and the `r15`/`r14`/`rbx`/`r13`
  pinning decisions. Freeing `arena_base` to TLS is a separate perf change; do not
  smuggle it in here.
- Do NOT "fix" this by removing `xmm8`–`xmm14` from `FP_REGS`. That hides the
  modeling bug behind a smaller pool and makes x86 FP pressure worse. The mask is
  what is wrong.

## Blast Radius

Search: every consumer of `call_clobber_mask` and every ISA-conditional in the
allocator.

- `analysis.rs:514` (`phys_def` accumulation) and `analysis.rs:597` (per-class
  `call_clobber` list) — the only two callers. Both fixed by this bug.
- `linear_scan.rs:142-147` — the sole consumer of the resulting mask; unchanged by
  the fix, but it is what turns the wrong mask into a wrong assignment.
- `regalloc/mod.rs:263-275` — computes `is_aarch64`/`is_riscv`; must be extended to
  carry the ISA to `ClassModel`. Fixed by this bug.
- `RegisterModel::caller_saved` (`src/arch/shared/regmodel.rs:38` + the three impls)
  — **the authoritative, already-correct statement of this exact ISA fact, and it
  has zero non-test callers.** This is the root structural problem: the fact is
  stated twice and the allocator uses the hand-rolled copy. In scope as the
  preferred fix vehicle (§Fix Design). Also filed separately as cleanup review
  agent-11 items 1 and 2.
- `RISCV_CALLER_SAVED_INT`/`_FP`/`ALL_INT` (`analysis.rs:59-66`) — correct;
  unaffected, but they become redundant once the mask is derived.
- AArch64 `CALLER_SAVED_INT`/`_FP`/`ALL_INT` (`analysis.rs:57-71`) — correct for
  AArch64; unaffected.
- `regalloc/tests.rs` — latent test gap, in scope: the linear-scan per-ISA
  branching has no x86 or rv64 coverage at all (cleanup review agent-05 item 20),
  which is precisely why this survived.

## Fix Design

Derive the mask from `RegisterModel::caller_saved` instead of hand-rolled per-ISA
constants. `caller_saved` already returns the correct register *names* for all
three ISAs; mapping them through the class's `physical_index` yields the correct
mask by construction and cannot drift. This simultaneously kills the dead trait
method and removes the "ISA fact stated twice, authoritative-looking copy unused"
hazard that caused the bug.

Concretely: compute the two masks once per function in `regalloc::allocate`, store
them on `ClassModel` (replacing the `is_riscv` flag), and have `call_clobber_mask`
read them. `all_int` becomes "every bit in the class's physical index space",
derived the same way.

Rejected alternatives:

- **Add an `is_x86` branch and a third pair of constants.** Smallest diff, and it
  fixes the defect — but it adds a fourth hand-maintained copy of an ISA fact that
  is already stated correctly elsewhere, and the next backend repeats this bug. Use
  only if the derivation proves to shift AArch64 output.
- **Order `FP_REGS` to put `xmm8`+ last.** Only hides the steering effect; the mask
  stays wrong and a high-pressure function still reaches them.
- **Drop `xmm8`–`xmm14` from the pool.** Rejected under Non-goals.

Expected output shift: **x86-64 only.** FP allocation should move off `xmm8`–`xmm14`
for call-spanning values (more FP spills, all correct), and integer allocation
should gain `r12`/`r14` across PCS calls (fewer integer spills). AArch64 and rv64
must be byte-identical. The x86 delta is the whole point and must be inspected, not
blanket-regenerated.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add a table test in `regalloc/tests.rs` asserting, for each of the three
      register models, that `call_clobber_mask` for a PCS `bl` equals the mask
      built from `RegisterModel::caller_saved`. Confirm it **fails for x86 FP**
      (bits 8–15 missing) **and x86 int** (bits 12–15 spuriously set), and passes
      for AArch64 and rv64.
- [ ] Add a targeted allocator test: a synthetic x86 stream with 9+ simultaneously
      live FP vregs and a `bl` between a def and a use, asserting no vreg is
      colored onto `xmm8`–`xmm14` across the call. Confirm it fails today.
- [ ] Re-run the 224-dump CFG scan and record the count in this file, so the
      "latent" claim has a dated baseline.

Acceptance: the new tests fail for exactly the documented reason; AArch64/rv64 rows
pass unchanged.
Commit: —

### Phase 2 — the fix

- [ ] Derive the masks from `RegisterModel::caller_saved`; thread them onto
      `ClassModel` in `regalloc/mod.rs`; delete the `is_riscv` flag and the
      now-redundant `RISCV_*` constants.
- [ ] Fix the stale comments at `src/arch/x86_64/regmodel.rs:10` and `:33`: drop the
      nonexistent "plan-00-H §7" reference, state that `arena_base` is pinned to
      `r15` by a recorded deliberate decision, and correct "AArch64's 19" to 14.

Acceptance: Phase 1 tests pass; AArch64 and rv64 output byte-identical.
Commit: —

### Phase 3 — validation

- [ ] `scripts/artifact-gate.sh` — confirm **zero** AArch64/rv64 artifact churn.
- [ ] Rebuild the 224-project x86-64 corpus; diff the dumps. Confirm the only
      changes are FP moves off `xmm8`–`xmm14` across calls and integer values newly
      surviving in `r12`/`r14`. Re-run the CFG scan: still 0 hits.
- [ ] Full acceptance suite on x86-64 hardware (the float/vector/math suites are the
      ones with real FP pressure).

Acceptance: full suite green on all targets; AArch64/rv64 byte-identical; x86 delta
is exactly the intended reallocation.
Commit: —

## Validation Plan

- Regression test(s): the per-ISA `call_clobber_mask` table test and the
  x86 FP-pressure allocator test, both in
  `src/target/shared/code/regalloc/tests.rs`.
- Runtime proof: the x86-64 float/vector acceptance suites on real hardware, plus
  the CFG-aware corpus scan reporting 0 hits with the mask now sound.
- Doc sync: `src/arch/x86_64/regmodel.rs:10,33` comments (nonexistent §7, wrong
  AArch64 count). No spec change — no spec documents the mask.
- Full suite: `scripts/artifact-gate.sh` then `tests/test-accept.sh` per target.

## Open Decisions

- Derive from `RegisterModel::caller_saved` (recommended — kills the duplicate ISA
  fact and the dead trait method in one move) vs. add a third hand-written constant
  pair behind an `is_x86` branch (smaller, safer diff, but re-arms the same trap for
  the next backend). Prefer derivation; fall back if it perturbs AArch64 output.

## Summary

The x86-64 backend inherits AArch64's caller-saved masks because
`call_clobber_mask` branches only on `is_riscv`. The integer half is merely
over-conservative — measurably so, since it strands 2 of x86's 4 allocatable GPRs.
The FP half is a genuine soundness hole: `xmm8`–`xmm14` are modeled as surviving a
call when SysV destroys them, and the allocatable ordering steers straight into
them. It is **latent** — 224 x86 dumps, 0 CFG-reachable instances, because the
intervening calls in today's code all sit on returning error tails — but the
registers are actively allocated, so the margin is luck, not design. The
engineering risk is not the fix (small) but the x86 output delta in Phase 3, which
must be read rather than regenerated; AArch64 and rv64 must not move at all.

## Resolution (2026-07-19)

Fixed by derivation, the recommended option in §Open Decisions:
`ClassModel` now carries a `caller_saved` mask built once per allocation by
`analysis::caller_saved_mask` from the target's own `RegisterModel::caller_saved`
table, mapped through that class's `physical_index`. The `is_riscv` flag, the
three hand-written constant pairs, and `ALL_INT`/`RISCV_ALL_INT` are gone;
`all_int` is now `PhysMask::MAX`, which is provably equivalent (bits above a
target's register-number space name no register, and both consumers index by a
real register index — phantom bits can never match a candidate, and in the
liveness dataflow a def-mask only *removes* bits, so they never reach `live_out`).

`integer_live_out` and `peephole::remove_fp_shuttles` take the register model
instead of an `is_riscv` bool, for the same reason.

Derivation reproduces the AArch64 and rv64 constants exactly — verified by
arithmetic before the change and by the artifact A/B after it.

### Measured outcome

- **AArch64/rv64 byte-identical, as required.** A/B `linux-artifact-baseline`
  capture over 1016 fixtures x 3 targets (11,190 hashes) differs on **2144
  lines, every one of them `linux-x86_64`**. Zero `linux-aarch64`, zero
  `linux-riscv64`. `scripts/artifact-gate.sh` (macos-aarch64): 1193 goldens,
  0 diffs. Runtime proof on real hardware: 2224 aarch64/musl 469 passed /
  0 failed; 2229 riscv64/musl 456 passed / 0 failed (13 `BUILD-FAIL`, each
  reproduced identically with the pre-fix compiler).
- **x86-64 delta is exactly the intended reallocation**: 536 fixtures, `.mir`
  and `.ncode` only — no `.nir`/`.nplan`/`.nobj` change and no fixture changed
  build status. Frames shrink (e.g. `func_fs_setBuffered_valid` drops one spill
  slot, 456 → 440 bytes) as `r12`/`r14` survive PCS calls, and FP values move
  off `xmm8`–`xmm14` across calls.
- **The Phase 1 CFG scan, re-run on the fixed corpus: 0 hits** — and now for the
  sound reason. `xmm8`–`xmm14` are still actively allocated (in 100/69/69/32/22/11
  functions respectively), so the pool was not merely emptied; the allocator just
  never places a call-spanning value there.
- x86-64 runtime proof on 2227 (musl): 460 passed / 9 failed before, 464 passed /
  5 failed after. No regression — the four that flipped are bug-362's, fixed in
  the same session. The remaining five are pre-existing (`os::userName` was
  confirmed by re-running it against the pre-fix compiler).

### Not done here

The stale-comment fix landed (`regmodel.rs` module comment and
`INT_ALLOCATABLE`: the nonexistent "plan-00-H §7" reference replaced with the
plan's actual recorded decision, and "AArch64's 19" corrected to 14).
`RegisterModel::caller_saved` is now a live, load-bearing method rather than a
dead one, which was the structural point of choosing derivation.

`x86_64::caller_saved(RegClass::Fp)` still returns `FP_REGS` (`xmm0`–`xmm14`), so
the derived mask leaves bit 15 clear rather than setting all 16 as §Goal wrote.
That is moot and deliberately left alone: `xmm15` is the reserved SSE scratch,
absent from the allocatable pool, so it is never a coloring candidate and its
bit cannot affect a decision. Setting it would have meant editing two committed
assertions that deliberately pin `caller_saved(Fp) == FP_REGS`.
