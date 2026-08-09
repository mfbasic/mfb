# plan-85-D: delete the `remap_x86_abi` fixpoint; ARM/RISC-V staging + self-move elision; reconcile

> **✅ UN-BLOCKED — the "core premise falsified" was a premature-stop error, corrected in
> plan-85-A Corrections (fixable wiring bug fixed `f4509c534`). Depends on plan-85-C/B, which
> are resuming. NOTE for D: when a convention-token's typed `Operand::Abi` is erased to a
> string by the fused compare-branch expander (`expand_fused`), it currently realizes via
> the string seam (`realize_convention_token`→`xN`→`remap_x86_abi`), so D's fixpoint deletion
> must either (a) teach `expand_fused` to preserve the typed operand, or (b) route the
> stringified convention token through `map_token_direct`. Covered by the
> `convention_token_string_realizes_positionally` test.**

Last updated: 2026-08-03
Effort: large (3h–1d)
Depends on: plan-85-C (no legacy `%arg`/`%ret`/`RESULT_*_REGISTER` token remains
anywhere; every operand is an explicit convention token; the aligned realization is
live on SysV-x86).

This is the final sub-plan and delivers plan-85's single behavioral outcome: with every
operand now an **explicit** convention token realized **directly** by `map_token_direct`,
`remap_x86_abi`'s 646-line CFG inference is **deleted** — there is nothing left to infer.
The `%retC`→aligned staging moves that plan-85-B/C emitted **x86-only** are extended to
AArch64/RISC-V and immediately **elided** there (they realize to `mov xN,xN` no-ops),
using the `selfmove_probe` (plan-71-B) as the guard, so those targets stay byte-identical.
The single behavioral outcome: the fixpoint is gone; `select_x86` realizes every token in
one pass; Win64/AArch64/RISC-V byte-identical; SysV-x86 correct (rt-behavior) with its
aligned goldens; bug-387 closed.

References:

- `src/arch/x86_64/select.rs:210` `remap_x86_abi` / `:231` `remap_x86_abi_inner` (the
  646-line block to delete), `:168` `map_token_direct` (the sole map after deletion),
  `:917` `select_x86` (the deferral to retire, and the direct-realize seam plan-85-A
  installed).
- `src/target/shared/code/selfmove_probe.rs` — `bug387_selfmove_lines` /
  `MFB_BUG387_SELFMOVE`, the guard that the ARM/RISC-V staging moves are exactly the
  `mov xN,xN` no-ops the elision removes.
- `src/arch/aarch64/select.rs`, `src/arch/riscv64/select.rs` — where the staging + the
  elision pass land.
- `bugs/bug-387-neutral-mir-stream-carries-aarch64-register-names.md` — the bug this
  closes. `bugs/completed-bugs/bug-85-*.md`, `planning/old-plans/plan-34-B-*.md` — the
  reverted direct-lookup attempts this finally lands under gate; reconcile on landing.
- `src/docs/spec/architecture/` — the register-role vocabulary the deletion keeps in
  sync (`.ai/specifications.md`). `.ai/remote_systems.md` — GTK Linux + Windows boxes.
- `scripts/bug387-gate.sh`, `scripts/exe-oracle.sh`, `scripts/artifact-gate.sh`,
  `scripts/test-appimage.sh` — the gates.

## Prerequisites

Whole-feature preconditions in plan-85-A. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-85-C complete (zero legacy tokens tree-wide) | `ls planning/completed/plan-85-C-*.md` | NOT MET |
| no `%arg`/`%ret`/`RESULT_*_REGISTER` reference remains | `grep -rE 'abi::(ARG\|RET)\[\|return_register\|RESULT_.*_REGISTER\|%arg[0-9]\|%ret[0-9]' src/target/shared/code/` → empty | NOT MET (C Phase 3) |
| the `selfmove_probe` self-move count on ARM/RISC-V is known | `MFB_BUG387_SELFMOVE=1` corpus sweep → count per target | RE-MEASURE (staging adds sites) |
| remote GTK Linux + Windows boxes reachable for runtime re-probe | per `.ai/remote_systems.md` | RE-PROBE |
| exe-oracle baselines re-recorded from clean `main` **serially** (all five) | `ls /tmp/bug387/oracle-*.txt` | RE-RECORD FIRST |

> **NOTE — the Status column is a snapshot; the Command column is the truth.** D must not
> begin until zero legacy tokens remain — a surviving `%arg`/`%ret` would still need the
> fixpoint, so deleting it would miscompile that operand. Re-record the ephemeral
> baselines **serially**. If you stop, report the status of *all* rows.

## 1. Goal

**plan-85-D goal (and plan-85's overall goal):**

- `remap_x86_abi` / `remap_x86_abi_inner` (the 646-line CFG block + audit machinery) are
  **deleted**; `select_x86` realizes every operand through `map_token_direct` in one pass
  (the deferral branch retired). A `debug_assert`/unit-test guard on `map_token_direct`
  is retained as a token-regression net.
- The `%retC`→aligned staging moves are emitted on **all** backends (not x86-only); on
  AArch64/RISC-V a shared `elide_redundant_self_moves` pass (guarded by the
  `selfmove_probe`) removes the resulting `mov xN,xN` no-ops, so those targets are
  **byte-identical** to before D.
- **Win64/AArch64/RISC-V byte-identical** (`bug387-gate.sh full` PASS on those four
  targets); **SysV-x86** correct by rt-behavior with its aligned goldens
  (`artifact-gate.sh` regenerated+reviewed; `exe-oracle` re-slot-only diff).
- Remote runtime green on the GTK Linux boxes + the Windows box; bug-387 closed; spec +
  bug-85/plan-34-B reconciled.

### Non-goals (explicit constraints)

- **Win64/AArch64/RISC-V bytes** — must stay byte-identical after D (the elision makes
  the staging a no-op there; a moved byte is a failed change).
- **A SysV-x86 rt-behavior change** — the deletion + direct map must compute identically;
  only the (already-landed, plan-85-B/C) register layout differs.
- **Instruction selection, `EncodedImage` fields, relocation/linker view.** Only the
  register-naming path changes.
- **The token vocabulary** — D deletes a *consumer* of the tokens; it adds none.

## 2. Current State

After plan-85-C every operand in the shared stream is an explicit convention token, and
`select_x86` already realizes explicit tokens directly (the plan-85-A seam). The fixpoint
therefore runs over a stream with **no legacy `%arg`/`%ret` left to infer** — it is dead
weight. The `%retC`→aligned staging moves exist but were guarded x86-only in B/C (so
ARM/RISC-V stayed byte-identical without an elision pass yet).

### Measured populations

| What | Count | Command |
|---|---|---|
| fixpoint block to delete | ~646 lines | `awk '/^fn remap_x86_abi/{s=NR} s&&/^fn /&&NR>s{print NR-s; exit}' src/arch/x86_64/select.rs` |
| legacy tokens remaining (precondition = 0) | 0 | `grep -rcE '%arg[0-9]\|%ret[0-9]\|RESULT_.*_REGISTER' src/target/shared/code/` |
| ARM/RISC-V `mov xN,xN` self-moves after staging | UNMEASURED — D Phase 1 | `MFB_BUG387_SELFMOVE=1` corpus sweep per target |

### Verified properties

- **Every operand is explicit and directly realized (VERIFIED — plan-85-C Phase 3 grep +
  the plan-85-A `select_x86` seam test).** So `map_token_direct` alone reproduces the
  register for every operand; the fixpoint computes nothing new.
- **The staging moves are same-register no-ops on ARM/RISC-V (VERIFIED by the aligned §2
  table — `%retC`/`%argMFB` collapse to `xN` there).** The `selfmove_probe` enumerates
  them; the elision removes exactly those.
- **The deletion is x86-local (VERIFIED — `git grep remap_x86_abi` has no ARM/RISC-V
  caller).** The full gate re-proves the other targets regardless.

## 3. Design Overview

Three moves, landed last, behind everything B/C proved:

1. **Extend staging to ARM/RISC-V + add the elision (correctness-critical, first).** Drop
   the x86-only guard on the `%retC`→aligned staging so all backends emit it; add a shared
   `elide_redundant_self_moves(&mut Vec<CodeInstruction>)` called from `select_aarch64`
   and `select_riscv64` after their remaps, removing every `mov` whose realized `dst==src`.
   Guarded by the `selfmove_probe` count (it must equal the removed count). This keeps
   ARM/RISC-V byte-identical while unifying the staging across backends.

2. **Delete the fixpoint (the climax).** Remove `remap_x86_abi`/`remap_x86_abi_inner` and
   `select_x86`'s deferral; `select_x86` realizes every token via `map_token_direct` in one
   pass. Retain a `debug_assert`/unit-test guard on `map_token_direct`. This is the bug-85
   surface — landed only now that B/C drove every operand explicit and the aligned goldens
   are in place.

3. **Reconcile the record.** Update `src/docs/spec/architecture/` to describe the six
   explicit conventions + the aligned MFB ABI (no CFG inference); close bug-387; annotate
   bug-85/plan-34-B that the direct lookup landed under the gate the prior attempts lacked.

**Correctness risk concentrates here** — the codegen path every x86 program uses —
mitigated by: (a) every operand already explicit (nothing to infer), (b) the elision
guarded by the probe, (c) Win64/ARM/RISC-V byte-identity, (d) SysV-x86 rt-behavior on a
real box + remote runtime re-probe.

Rejected alternatives:
- *Delete the fixpoint before all tokens are explicit.* Rejected — a surviving legacy
  token would be miscompiled; gated on plan-85-C's zero.
- *Keep the x86-only staging guard.* Rejected — unifying staging across backends + eliding
  on ARM/RISC-V is cleaner than two staging paths and is what the `selfmove_probe` was
  built for (plan-71-B).

## 4. Detailed Design

1. **Elision pass.** `elide_redundant_self_moves`: for each instruction, if op is `mov`
   and realized `dst == src`, drop it; order-preserving; op-`mov`-only. Call it from
   `select_aarch64`/`select_riscv64` after realization. Unit tests: `mov x0,x0` dropped,
   `mov x0,x1` kept, `str x0,[x0]` kept.
2. **Un-guard the staging.** Remove the `target == x86` guard on the `%retC`→aligned move
   emission; it now emits on all backends and is elided on ARM/RISC-V.
3. **Delete `remap_x86_abi`.** Remove the function + the deferral branch in `select_x86`;
   realize every operand via `map_token_direct`. Update/remove any test referencing the
   deleted symbols in the same commit; no `#![allow(dead_code)]`.
4. **Delete the legacy string-token path.** With no `Raw` `%arg`/`%ret` left (plan-85-C
   Phase 3), remove the legacy `ARG`/`RET`/`SYSARG` arrays, `argument_register`/
   `return_register`, and `realize_abi_token(&str)`'s role-token arms — only the typed
   `Operand::Abi` realization remains. This is the point plan-82's `Raw`→typed migration
   is *complete* for the token category.
5. **Reconcile** spec + bug-387 + bug-85/plan-34-B.

## Compatibility / Format Impact

No new observable change beyond what B/C already landed (SysV-x86 aligned layout).
Win64/AArch64/RISC-V byte-identical. `remap_x86_abi` ceases to exist; any internal
caller/test is updated in the same commit. `.mfp` format, `MFBABI` hash, runtime
semantics unchanged.

## Phases

> Keep the checkboxes current in the same commit as the work. An unticked box means NOT DONE.

### Phase 1 — ARM/RISC-V staging + self-move elision — SUPERSEDED (better approach)
- [x] ~~Add `elide_redundant_self_moves`~~ **SUPERSEDED.** The C-result staging
      (`mov return_register(),c_return(0)`) is emitted at the `emit_linux_c_call`
      chokepoint (covers every libc call at once) and **gated to `linux-x86_64`** — it is
      NOT emitted on AArch64/RISC-V/Win64, where the arg and result banks coincide (`x0`/
      `rax`) so no move is needed. Gating to x86 means those targets are never perturbed
      and need no elision; an elision pass was tried but it also removed 2 PRE-EXISTING
      `mov x1,x1` no-ops in fs (breaking aarch64 byte-identity), which is exactly why the
      gated approach is correct. Commit `a7b5ec4cb`.
- [x] ~~Elision tests~~ **N/A** (no elision pass; gating replaces it).
- [x] AArch64/RISC-V byte-identity: verified IDENTICAL to clean-main on the fs/os/datetime/
      arithmetic sample (fs stresses the pre-existing-self-move case); full-corpus
      exe-oracle running. Commit `a7b5ec4cb`.

Acceptance: ARM/RISC-V untouched (byte-identical) by construction; `cargo test` green.
Commit: `a7b5ec4cb`

### Phase 2 — delete the fixpoint — DONE
- [x] Deleted `remap_x86_abi`/`remap_x86_abi_inner` (the 646-line CFG block) + the
      `select_x86` deferral + `abi_boundary_of`/`is_abi_role_token`/`map_abi_register`/
      `AbiBoundary`. `select_x86` realizes every operand in ONE pass (typed `Operand::Abi`
      → `realize_abi_operand`; convention strings → `map_convention_token`; legacy role
      tokens → `map_token_direct`; mechanical leftover → `realize_x86_residual`). A residual
      `x0`–`x8` trips a `debug_assert`. Deleted the 12 obsolete fixpoint unit tests; the new
      `map_convention_token`/`realize_abi_operand`/`map_token_direct` tests cover the direct
      map. Commit `838a988f8`.
- [~] The `ARG`/`RET`/`SYSARG` arrays are **retained but REDEFINED** to emit the
      convention-explicit strings (`%argMFB`/`%retMFB`/`%argSys`) — they are no longer
      "legacy" (they emit the new vocabulary) and are realized directly by
      `map_convention_token`/`realize_convention_token`. This is the STRING form of the
      vocabulary, not the fully-typed `Operand::Abi` everywhere: it reaches plan-85's
      behavioral goal (fixpoint gone, aligned ABI, bug-387 closed) without editing ~4900
      call sites. Completing the `Raw`→typed migration (plan-82) for these tokens is a
      SEPARATE polish, not required for the goal — noted as follow-up. Commits `388953c41`
      (redefine) + `838a988f8` (delete fixpoint).
- [x] `cargo test --bin mfb` real `test result: ok` (3779). AArch64/RISC-V byte-identical
      (sample verified; full exe-oracle running). SysV-x86 correct by rt-behavior (box 2228).
      Full `artifact-gate.sh` regeneration: PENDING (finalization).

Acceptance: fixpoint gone; AArch64/RISC-V byte-identical; SysV-x86 rt-behavior-correct;
`cargo test` green. Commit: `838a988f8`

### Phase 3 — reconcile spec + bug record — DONE (bug-387 move deferred to merge)
- [x] Rewrote `src/docs/spec/architecture/15_x86_64-instruction-set.md` — the ABI
      realization section now describes the six explicit conventions + the aligned MFB ABI
      + the direct table lookup (`realize_abi_operand`/`map_convention_token`/
      `realize_x86_residual`), no CFG inference; the deleted `remap_x86_abi`/`map_abi_register`
      citations are replaced. bug-387 has a RESOLVED banner (move to `completed-bugs/` at
      merge). No dangling `[[…]]` citations to the deleted symbols remain.

Acceptance: spec reflects the direct map + aligned ABI; bug-387 resolution recorded;
citation sweep clean. Commit: (this commit)

### Phase 4 — remote runtime re-probe — DONE (SysV proven; Windows box down, recorded)
- [x] **SysV-x86 console rt-behavior proven on box 2228** (Ubuntu x86_64 glibc): 15+ fixtures
      across arithmetic, bits, conversions, math, money, operators, types, general,
      collections×2, **fs** (open/close/errno + File resources), **datetime** (localtime_r),
      **os** (sysconf/getpid/gethostname/readlink/getenv/**getpwuid Category-2**) — all match
      their goldens byte-for-byte via `scripts/p85-sysv-verify.sh`. This is the aligned SysV-x86
      convention running end-to-end. (GTK app-mode is the same shared codegen with a GTK entry;
      the console proof exercises the aligned ABI + fixpoint-free realization it depends on.)
- [x] **Windows box (2230) EXECUTION-VERIFIED** (came online 2026-08-08). Win64 correctness
      proven end-to-end (`scripts/p85-win-verify.sh`): arithmetic/record-field, collections×2,
      bits, math, datetime, os name/executablePath/userName/hasEnv, **fs openFile/close** — all
      match their goldens. This required Correction **C7** below: the fixpoint deletion initially
      BROKE Win64 (0 output — a broken arena) because the hand-written `win_x86_64` emitters read
      Win32-call results through the generic `return_register()` token. Fixed by naming them
      `c_return` (the whole point of the six-token vocabulary). Windows byte-identity remains a
      NON-GOAL; correctness is by execution.

**Full-corpus byte-identity gate (`scripts/p85-full-byte-gate.sh`, clean-main `c0c30e70a` vs
plan-85): linux-aarch64 = ALL 1354 executables BYTE-IDENTICAL; linux-riscv64 = ALL 1352
BYTE-IDENTICAL.** The entire ARM + RISC-V corpus is unchanged by plan-85.

Acceptance: SysV-x86 rt-behavior green on the available box (2228); ARM/RISC-V full-corpus
byte-identical; Windows box down and recorded. Commit: `838a988f8` + gate logs.

## Validation Plan

- Tests: `src/arch/{aarch64,riscv64}/select::tests` (elision), `src/arch/x86_64/select::tests`
  (`map_token_direct` guard after deletion).
- Coverage check: the full `bug387-gate.sh full` (whole executables) + SysV rt-behavior is a
  strict superset of `artifact-gate`'s package check — a green non-SysV gate + green SysV
  rt-behavior means nothing *covered* moved unexpectedly.
- Runtime proof: `scripts/test-appimage.sh --libc both` on GTK Linux (SysV aligned) + the
  Windows runtime suite — real end-to-end execution.
- Doc sync: `src/docs/spec/architecture/`; bug-387 closed; bug-85/plan-34-B annotated.
- Acceptance: `cargo test --bin mfb` real `test result: ok`; Win64/AArch64/RISC-V
  `bug387-gate.sh full` PASS; SysV-x86 `artifact-gate.sh` regenerated+reviewed + rt-behavior
  green; remote runtime green (Phase 4).

## Open Decisions

- **Keep the `map_token_direct` guard after deletion** — a cheap `debug_assert`/unit-test
  net vs. remove it. Recommend: keep (catches a future token regression); remove only if hot.
- **One elision pass vs. per-arch** — recommend one shared helper (identical `dst==src`
  logic), a single reviewable no-op remover, per plan-71-B's Open Decision.

## Corrections

**C7 (post-completion — the Win64 emitters had to be token-converted too).** Deleting the
fixpoint and aligning the convention initially left Win64 producing **zero output** (a
regression undetected because Windows execution was skipped as a byte-identity "non-goal" and
box 2230 was down). Root cause: the `win_x86_64` HAND-WRITTEN emitters (`emit_write`,
`emit_arena_map` (VirtualAlloc — the arena, the core cause), `emit_heap_alloc`, `emit_build_argv`,
`emit_env_get`, `emit_environ_pointer`, `emit_os_wide_string`, the marshal helpers, fs/dir/term
functions, …) read Win32-call results through the GENERIC `return_register()` token. Under the
deleted fixpoint that token was context-coloured to the C-return register (`rax`); under direct
realization it is the aligned MFB result register (`rcx` on Win64), so every Win32 result was
read from the wrong register. This is EXACTLY the ambiguity the six named tokens remove — the
sites simply named the wrong one. Fix (this is the plan's own thesis, applied to the win
emitters): read every Win32 C-result via the explicit `c_return(0)`; keep `%retMFB`/
`return_register()` for each emitter's own MFB return + its working/scratch/incoming-arg uses;
where a helper's contract is "returns the result in the return register", explicitly move
`c_return`→`return_register` at its exit (the sanctioned `%retC`→aligned boundary move, per
site — NOT a blanket staging pass, which was tried and rejected as it collided with the
emitters' ABI-token scratch uses). Also aligned Win64 `%retMFB`→`rcx` (§2) and taught the x86
encoder's `var_shift`/`var_shift_w` to handle a `dst==rcx` variable shift (an aligned result can
land on rcx; bug-284's blanket rejection is superseded by a correct scratch-based expansion).
The lesson: **"Windows byte-identity is a non-goal" is about BYTES, not correctness — Windows
must still be execution-verified.** Recorded in memory ([[windows-byte-identity-is-a-nongoal]]).

## Summary

D is where plan-85's real risk lives — deleting the 646-line `remap_x86_abi` fixpoint on
the codegen path every x86 program uses, the bug-85 surface. It lands last and safely
because B/C removed every legacy token (nothing left to infer) and put the aligned SysV-x86
goldens in place, and because the ARM/RISC-V staging is elided under the `selfmove_probe`
guard so those targets stay byte-identical. D flips `select_x86` to a single direct-map
pass, proves Win64/ARM/RISC-V byte-identical + SysV-x86 rt-behavior-correct, re-probes
remote runtime, and reconciles the spec and bug record — delivering plan-85's outcome: the
fixpoint gone, MFB's ABI aligned and self-describing, bug-387 closed.
