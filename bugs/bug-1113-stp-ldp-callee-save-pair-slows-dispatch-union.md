# bug-1113: pairing callee-saved saves into `stp`/`ldp` slows `dispatch/union` by 16 % on Apple M2

Last updated: 2026-10-26
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Other (performance regression)

Status: Open
Regression Test: — (a benchmark row; see Validation Plan)

plan-171-A phase 3 (`dd383c356`) made every AArch64 prologue save adjacent
callee-saved registers with `stp` and every epilogue restore them with `ldp`.
Across the language benchmark that is a net win (452 rows, geomean final/base
0.9757), but one row regresses: `dispatch/union` goes **13.04 → 15.15 ms
(+16 %)** at `-O1` on an Apple M2 Max. The program is correct; it is slower.

**The correct behavior a fix produces:** `dispatch/union` runs no slower than
at the pre-pairing compiler `89a4dce2c` (within run-to-run noise, min of
alternating runs), while the rest of the benchmark keeps the pairing gain
(geomean final/`89a4dce2c` ≤ 0.98) and every program's output is unchanged.

References:

- `planning/plan-171-A-aarch64-load-store-pairs.md` Correction 13 (the measurement
  and the isolation below are quoted from it), Phase 3.
- Owner decision 2026-10-26: merge plan-171-A as is, file this bug, fix it in step 3
  of the optimizations effort (`.claude/worktrees/opts`, `tmp/opt-plan.md`).
- `src/codegen/engine/util/vreg_frame.rs:finalize_frame` (the save/restore emission);
  `src/optimizer/opt2/pairing.rs:pair_adjacent_accesses` (the Level-1 body row — not
  implicated, see Root Cause).

## Failing Reproduction

```
# from a worktree at the current tip; build the benchmark at -O1 with the new and
# the pre-pairing compiler (detached worktree at 89a4dce2c, release build), then
# run them alternately on the macOS host and compare the dispatch/union row:
./benchmark/language/run.sh 10      # README: per-row ms, min of runs
```

- Observed (plan-171-A Correction 13, Apple M2 Max): `dispatch/union` 13.04 ms at
  `89a4dce2c`, 15.15 ms after phase 3/4.
- Expected: ≤ ~13.1 ms (the base, within noise).

Isolation already done (Correction 13):

- One-function program (`evalNode` from `benchmark/language/mfb/src/dispatch.mfb`
  alone): 13.1 → 16.6 ms.
- **Layout-independent:** the base binary patched in place, `str;str` → `stp;nop`
  at the same addresses, reproduces it (13.4 → 17.3 ms). Padding the layout eight
  ways never moves base above 13.5 ms or final below 16.3 ms.
- **One pair carries all of it:** patching only the `x21/x22` save costs nothing;
  patching only the `x23/x24` save costs the whole regression.
- Contrast: a same-shape assembly loop with no value on the critical path shows no
  difference (8.2 ms for `str`/`ldr`, `stp`/`ldp`, and mixed forms). Other
  recursive rows improve: ackermann 2.736 → 2.610 ms, treewalk 14.46 → 13.22 ms,
  binarytrees 25.51 → 24.70 ms; fib is flat (46.96 → 47.14).

| Environment | Result |
| --- | --- |
| macOS, Apple M2 Max, `-O1` | regresses ✗ (+16 %) |
| linux-aarch64 boxes | not measured — Phase 1 measures it |
| x86-64, riscv64 | unaffected ✓ (no pair form emitted; goldens byte-identical) |

## Root Cause

Hypothesis (strong, from the isolation above; Phase 1 confirms or eliminates):
in `evalNode`, `x23` holds the left operand's result across the second recursive
call. The callee's prologue saves `x23` and its epilogue restores it, so that
store→load round trip is on the critical path of the recursion. On this core a
single-register `str`/`ldr` round trip is forwarded fast, but the same data
written by `stp` and read by `ldp` (or `stp` + `ldr`, or `str` + `ldp`) is not —
consistent with the M2 store-to-load bypass applying only to matching
single-register accesses. `finalize_frame` pairs every adjacent save
unconditionally, so it cannot tell a latency-critical save from any other.

Hypotheses to eliminate: (a) code layout — eliminated by the same-address patch;
(b) the Level-1 body pairing row — eliminated, the regression appears at phase 3
(frames only) and the patch touches only the prologue/epilogue.

## Goal

- `dispatch/union` at `-O1` is within noise of `89a4dce2c` on the M2 host.
- The benchmark geomean against `89a4dce2c` stays ≤ 0.98 (the pairing gain is kept).
- Every runtime fixture's output is unchanged; the artifact gate is 0 after the
  intended golden regeneration.

### Non-goals (must NOT change)

- No change to frame **layout** (slot offsets, save-area order, `stack_size`),
  per plan-171-A §1 non-goals.
- Do not revert pairing wholesale, and do not special-case the benchmark or the
  function name `evalNode`.
- Do not "fix" by editing the benchmark program or its baseline.
- x86-64 and riscv64 output stays byte-identical.

## Blast Radius

- Every AArch64 function whose prologue pairs a callee-saved register that carries
  a value across a call in its caller — the pattern is general; `dispatch/union` is
  the one row where it dominates. Phase 1 counts how many benchmark rows move when
  the candidate rule is applied.
- The Level-1 body pairing row (`pair_adjacent_accesses`) — the same forwarding
  hazard applies to a body `stp` followed by a short-distance `ldr` of one half;
  Phase 1 checks whether any benchmark row shows it.

## Fix Design

Candidates, to be measured in Phase 1 before choosing:

1. **Don't pair the restore whose register is read soon after the return** — needs
   the caller's liveness, which `finalize_frame` (callee side) doesn't have.
   Rejected as stated: frame emission is per function.
2. **Pair saves, not restores** (`stp` in the prologue, two `ldr` in the
   epilogue) or the reverse — cheap, uniform; measure whether it removes the
   regression (Correction 13 shows `stp`+`ldr` is also slow, so likely not).
3. **Leave the highest callee-saved pair(s) unpaired in recursive functions**
   (self-call present) — targeted; measure geomean impact.
4. **Order save slots so that the latency-critical register is saved alone** —
   requires knowing which register; the allocator assigns `x19`… in order, so the
   first-assigned long-lived value tends to the lowest registers.

The choice and its measurements go in this document before Phase 2.

## Phases

### Phase 1 — measurement + audit (no behavior change)

- [ ] Re-measure `dispatch/union` and the full benchmark, base `89a4dce2c` vs tip,
      alternating, on the M2 host; record the numbers and commands.
- [ ] Measure on a linux-aarch64 box (`.ai/remote_systems.md`) — is the
      regression M2-specific?
- [ ] Prototype candidates 2–4 and record per-candidate `dispatch/union` and
      geomean.

Acceptance: the regression is reproduced, its scope across cores known, and one
candidate chosen with numbers.
Commit: —

### Phase 2 — the fix

- [ ] Implement the chosen rule in `finalize_frame` (and the body row if Phase 1
      implicates it), with unit tests pinning which saves pair.

Acceptance: `dispatch/union` within noise of base; geomean ≤ 0.98; no output change.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] Classify and regenerate the aarch64 native goldens (`scripts/artifact-gate.sh
      target/release/mfb all` → `scripts/regen-native-goldens.sh` → gate 0).
- [ ] `bash scripts/test-fast.sh`, `cargo test --release --test rt_codegen_forms`,
      full `scripts/test-accept.sh`, and a full `cargo test --workspace --no-fail-fast`.
- [ ] Re-run the benchmark comparison; update plan-171-A Correction 13 and the
      `planning/optimizations.md` "Load/store pair formation" row.

Acceptance: gates green; only the intended aarch64 goldens moved.
Commit: —

## Validation Plan

- Regression test(s): a `rt_codegen_forms` assertion pinning the chosen pairing
  rule on a recursive function; the benchmark row is the runtime proof.
- Runtime proof: `./benchmark/language/run.sh` before/after on the M2 host.
- Doc sync: plan-171-A Correction 13; `planning/optimizations.md`; `mfb spec
  architecture aarch64-instruction-set` if the frame contract text changes.
- Landing gate: `bash scripts/test-fast.sh` + the gates in Phase 3.
- Full tier: required before merge (the opts effort merges only after a green full
  workspace suite).

## Open Decisions

- Which candidate rule — decided by Phase 1's numbers.

## Summary

The risk is choosing a rule that recovers this row while giving back the
pairing gain elsewhere; Phase 1's measurements decide it. Correctness is not at
stake — the paired code is right, only slower on one core's forwarding path.
