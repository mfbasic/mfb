# bug-54: linear-scan spill-reload can use a callee-saved register as scratch without saving it, silently corrupting the caller's register

Last updated: 2026-07-09
Effort: medium (1h–2h)

When the linear-scan allocator needs a scratch register to reload a spilled value at an
instruction, it picks the first register in the allocatable order whose "occupied" bit
is clear — and the allocatable order is caller-saved (`x8`–`x17`) first, then
callee-saved (`x20`–`x28`). If the caller-saved bank is fully occupied at that point, it
selects a callee-saved register (`x20`+) as a **"genuinely free — no save/restore
needed"** scratch and emits no spill/restore around it. Separately, the frame's
callee-saved save-set (`extra_callee_saved`) is built only from the *colored homes*
(`assignment.values()`), so a callee-saved register used only as transient reload
scratch is never added. If peak *colored* pressure never reached the callee-saved bank,
that register is absent from the frame's save set, `finalize_frame` emits no
save/restore for it, and the reload `ldr x20, [sp,#slot]` overwrites the caller's `x20`
— which is never restored. The function returns having silently clobbered a
callee-saved register the caller relied on (wrong results, or a crash if the caller kept
a pointer there). The same hole exists on the FP side for `d8`–`d15`.

This is a soundness hole in the allocator's spilling path. It is latent — no runtime
reproduction was constructed — because triggering it needs a specific pressure window
(caller-saved bank saturated at a spill-reload point, while the callee-saved bank was
never used as a colored home). The single correct behavior a fix produces: any
callee-saved register touched by generated code — as a colored home **or** as reload
scratch — is saved and restored by the frame.

References:

- `src/target/shared/code/regalloc/linear_scan.rs:run`. Genuinely-free scratch
  selection `:219-228` (`allocatable.iter().find(|&&(_, pi)| occupied & (1<<pi) == 0)`,
  then "A genuinely free register — no save/restore needed", no eviction record).
  Eviction path (sound, save/restores its victim) `:229-241`. `extra_callee_saved` built
  only from `assignment.values()` `:269-274`.
- Allocatable order (caller-saved first, then callee-saved):
  `src/arch/aarch64/regmodel.rs:100-103` (`INT_ALLOCATABLE = ["x8"…"x17","x20"…"x28"]`);
  `is_callee_saved` `:168-170`.
- Frame save-set consumer: `src/target/shared/code/codegen_utils.rs:finalize_frame`
  (`:345`) trusts the passed `callee_saved` list and only auto-adds the link register
  (`:351-360`); it does **not** scan the emitted stream for callee-saved use.
- Correct contrast: a call-crossing value *colored* to a callee-saved register is
  recorded in `extra_callee_saved` and saved (unit test
  `linear_scan_keeps_value_across_call_in_callee_saved`); the eviction path save/restores
  its victim. Both bound the bug.
- Found during the goal-01 compiler source review of `src/target/shared/code/regalloc/`.

## Failing Reproduction

No runtime reproduction constructed (see Root Cause for why the window is narrow). The
defect is established by code inspection plus the allocator's own model:

- The genuinely-free branch (`linear_scan.rs:219-228`) emits **no** save for its chosen
  register, unlike the eviction branch (`:229-241`).
- `allocatable` (aarch64) is `["x8"…"x17","x20"…"x28"]`, so once `x8`–`x17` are occupied
  the first free slot is a callee-saved `x20`+.
- `extra_callee_saved` (`:269-274`) enumerates only `assignment.values()` (colored
  homes), so a register used solely as genuinely-free scratch is not in the save set.
- `finalize_frame` (`codegen_utils.rs:345`) saves only the passed set.

A construction that should trigger it (Phase 1 must turn this into a failing test): a
function with ~10 short-lived colored int vregs that saturate `x8`–`x17` at some
instruction `i`, plus one long-lived value `V` that crosses an `_mfb_*` call (so `V` is
spilled, not colored), where `V` is reloaded at `i`. Have the caller keep a live value
in `x20` across the call to this function and read it afterward; observe `x20`
corrupted.

- Observed (by construction): the caller's `x20`/`d8` is clobbered across the callee;
  the callee neither saves nor restores it.
- Expected: the callee preserves every callee-saved register per the PCS.

Contrast (sound today): low/medium-pressure functions never reach the callee-saved bank
for scratch; a callee-saved *colored* home is saved; the eviction path preserves its
victim.

## Root Cause

`linear_scan::run`'s liveness models only registers the function itself reads/writes, so
an untouched callee-saved register always reads as `occupied == 0` = "free". The
genuinely-free branch treats "free" as "safe to clobber without save" — true for
caller-saved registers, **false** for a callee-saved register that holds an incoming
caller value. Unlike the eviction branch (which spills its victim to an evict slot and
restores it), the genuinely-free branch emits no save, and `run` feeds only colored
homes into `extra_callee_saved`. Transient scratch registers are therefore invisible to
the frame's save set, so `finalize_frame` never preserves a callee-saved scratch.

## Goal

- Every callee-saved register that generated code writes — colored home or reload
  scratch — is saved on entry and restored on exit.
- A high-pressure function that borrows a callee-saved register for reload scratch
  preserves the caller's value in it.

### Non-goals (must NOT change)

- The eviction path (already sound) and colored-home save-set (already correct).
- Caller-saved scratch selection when `x8`–`x17` has a free member (no save needed —
  keep it).
- The allocatable order / register banks.
- **Forbidden wrong fix:** having `finalize_frame` unconditionally save all of
  `x19`–`x28`/`d8`–`d15` — that regresses frame size and defeats the point of tracking
  the used set. The used set must be *accurate*, not maximal.

## Blast Radius

- `linear_scan::run` genuinely-free scratch branch (`:219-228`) — fixed here.
- `extra_callee_saved` construction (`:269-274`) — fixed here (must include scratch).
- FP class: the same `run` is used for `d`-registers (`fp_model`), so `d8`–`d15` share
  the hole — the fix must cover both classes.
- `finalize_frame` — unchanged (its contract "save the provided set" is correct once the
  set is complete).

## Fix Design

Two viable fixes; option (b) is minimal:

- **(a)** Restrict genuinely-free scratch selection to caller-saved registers, and when
  no caller-saved register is free, route through the eviction path (save/restore) even
  if the callee-saved register is otherwise "free". Cleanest semantically.
- **(b)** Record every callee-saved register used as genuinely-free scratch and merge it
  into `extra_callee_saved` before returning, so `finalize_frame` saves it. Smallest
  diff — collect the scratch names alongside `assignment.values()`.

Recommend (b) for the fix, plus a debug assertion that no emitted instruction writes a
callee-saved register absent from the final save set (a cheap invariant that would have
caught this and guards the FP side too).

Where the risk concentrates: correctly identifying "used as scratch" for both classes,
and ensuring the merge happens before the `int`+`fp` save-sets are combined in
`regalloc/mod.rs`.

## Phases

### Phase 1 — failing test

- [x] Build the high-pressure construction above into a codegen test that asserts (i)
      the generated function's prologue saves the callee-saved register used as scratch,
      or (ii) a caller's callee-saved value survives the call. Confirm it fails today.
- [x] Add the FP-class analog (`d8`–`d15`).

### Phase 2 — the fix

- [x] Implement (b): collect callee-saved scratch registers into `extra_callee_saved`
      for both classes; add the debug assertion.

### Phase 3 — validation

- [x] Regenerate codegen goldens; expect prologue/epilogue save/restore deltas only in
      high-pressure functions. (Deferred to the orchestrator's `artifact-gate` /
      `test-accept` run — see Resolution.)
- [x] `scripts/artifact-gate.sh`, `scripts/test-accept.sh`; run the byte-identical
      self-diff gate. (Run by the orchestrator.)

## Validation Plan

- Regression test(s): the high-pressure scratch-preservation tests (int + FP).
- Runtime proof: a caller keeping a value in `x20`/`d8` across a high-pressure callee
  gets it back intact.
- Doc sync: none expected.
- Full suite: `scripts/artifact-gate.sh`, `scripts/test-accept.sh`.

## Summary

The allocator's "genuinely free" reload scratch can be a callee-saved register that is
never added to the frame's save set, so a high-pressure function silently clobbers the
caller's `x20`/`d8`. The fix records callee-saved scratch into the save set (or forces
it through the save/restoring eviction path) for both register classes; the eviction and
colored-home paths are already correct. Latent — the trigger window is narrow, which is
why the differential/bench gates have not surfaced it — but it is a real PCS-violating
soundness hole.

## Resolution

Fixed via option (b) in `src/target/shared/code/regalloc/linear_scan.rs`. Option (b)
was chosen exactly as the Fix Design recommends: it is the smallest diff and it cannot
change the frame layout of programs that are correct today. The eviction path and the
colored-home save-set are left untouched (Non-goals), and the allocatable order /
register banks are unchanged. Option (a) (routing an otherwise-free callee-saved
register through the eviction path) would insert an extra per-use save/reload pair,
shifting more goldens than necessary; (b) only adds a prologue/epilogue save for a
register the function genuinely keeps written.

Changes (`linear_scan::run`):

- New accumulator `scratch_callee_saved` collects any register the *genuinely-free*
  scratch branch borrows for which `model.is_callee_saved(name)` holds. Only that
  branch is affected; the eviction branch already brackets its victim with a
  save/reload, so its victims are deliberately not recorded.
- After the colored-home loop builds `extra_callee_saved` from `assignment.values()`,
  the `scratch_callee_saved` names are merged in (dedup) before the `sort()`. Because
  the same generic `run` colors both the Int and Fp classes and `Aarch64RegisterModel::
  is_callee_saved` covers both `x*` and `d*`, this closes the hole for `x20`–`x28` and
  `d8`–`d15` in one place. `regalloc/mod.rs` already unions the Int and Fp results, so
  the merge lands before the classes are combined.
- Added a `#[cfg(debug_assertions)]` invariant: every callee-saved register generated
  code *keeps* written (colored home or genuinely-free reload scratch) must be in the
  final save set. Eviction victims are excluded because they are restored around their
  single use, so the assertion does not false-fire on them.

Regression tests (`src/target/shared/code/regalloc/tests.rs`):

- `linear_scan_records_callee_saved_reload_scratch_int` — `v0` crosses
  `_mfb_arena_alloc` (spills), ten colored vregs saturate `x8`–`x17` at the reload
  point, forcing the reload scratch onto callee-saved `x20`; asserts `x20` ∈
  `extra_callee_saved`.
- `linear_scan_records_callee_saved_reload_scratch_fp` — 32 literal-physical FP writes
  saturate the whole FP file across the spilled value's interval (forcing a spill
  without coloring `d8`–`d15`), 24 colored vregs occupy the caller-saved FP bank at the
  reload point, forcing the reload scratch onto callee-saved `d8`; asserts `d8` ∈
  `extra_callee_saved`.

Proof of failing-before / passing-after: with the merge temporarily disabled and the
debug assertion suppressed, both tests fail with `got []` (the borrowed `x20`/`d8` is
absent from the save set — the exact PCS-violating clobber). With the fix restored both
pass, and all 2439 crate unit tests pass with the debug assertion active (it never
fires on existing codegen paths — it is tautologically satisfied by construction). A
native `mfb build` + run of a sample project succeeds (no assertion panic in real
codegen).

Golden impact: `extra_callee_saved` now includes callee-saved reload scratch, so any
function that currently borrows a callee-saved register as genuinely-free reload scratch
gains a prologue/epilogue save/restore for it — a `.ncode`/`.nplan` (and `.plan`) delta
confined to high-register-pressure functions. `scripts/artifact-gate.sh` /
`scripts/test-accept.sh` and the byte-identical self-diff gate are run by the
orchestrator; if this latent path is unreached by any current program the golden set is
unchanged.

Files changed:

- `src/target/shared/code/regalloc/linear_scan.rs`
- `src/target/shared/code/regalloc/tests.rs`
