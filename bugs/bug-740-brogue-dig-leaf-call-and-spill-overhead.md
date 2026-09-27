# bug-740: brogue `dig` is still ~17.6x the oracle — tiny leaf helpers are called, not inlined, and every temporary round-trips through the stack

Last updated: 2026-09-26
Effort: huge (>3d)
Severity: MEDIUM
Class: Performance

Status: Open
Regression Test: tools/brogue-perf/bench.sh (existing; its budget is the gate)

bug-737 removed the copying that made `examples/brogue`'s `dig` 120x slower
than the C oracle (a whole-list operand snapshot on every `layerAt`-style read
of a module global; 6.1 GB → 32 MB per `dig 1 5`). What is left is not
allocation: `dig 1 5` now makes 148,095 arena allocations totalling 32 MB, and
the remaining time is spent executing ordinary integer code. On a quiet
machine the harness measures **17.6x** (see Failing
Reproduction), against bug-707's proposed goal of 10x.

The single correct behavior a fix produces: the same program, compiled at the
default optimization level, runs its call-dense integer loops without paying a
full call frame for a one-expression helper and without storing and reloading
every intermediate through a stack slot — measured as `bench.sh` at or below
its goal ratio, output still byte-identical to the oracle.

References:

- `bugs/completed/bug-737-brogue-level-generation-still-120x-slower-after-bug-707.md`
  — the copy mechanism fixed there, and this remainder's first profile.
- `bugs/completed/bug-707-brogue-level-generation-is-280x-slower-than-oracle.md`
  — the original 10x goal.
- `tools/brogue-perf/bench.sh` — the harness.

## Failing Reproduction

```
$ cargo build --release
$ mkdir /tmp/mfb-heavy.lock          # quiet machine
$ tools/brogue-perf/bench.sh --runs 7 --budget 10
seed=1 depth=5  port(median)=0.165s  oracle(median)=0.012s
seed=1 depth=10  port(median)=0.353s  oracle(median)=0.018s
seed=1 depth=20  port(median)=0.370s  oracle(median)=0.019s
seed=2 depth=5  port(median)=0.148s  oracle(median)=0.010s
seed=3 depth=15  port(median)=0.248s  oracle(median)=0.014s
port total user=1.2840s  oracle total user=0.0730s  ratio=17.6x  budget=10x  (seed 1 depth 5 alloc_calls=148095)
FAIL: ratio 17.6x exceeds budget 10x
$ rm -rf /tmp/mfb-heavy.lock
```

- Observed: ratio 17.6x (the medians above are from the bug-737 run of
  2026-09-26, taken with the default budget and the lock held; only the
  budget comparison line differs).
- Expected: at or below the goal (bug-707 proposed 10x).

### How to measure (exact recipe)

1. **Ratio (the gate).** Hold the heavy lock, then run
   `tools/brogue-perf/bench.sh --runs 7`. The default budget is 25. The current
   value is **17.6x** (port 1.284 s total user time, oracle 0.073 s, measured
   2026-09-26 at the bug-737 tip). Pass `--budget 10` to test against the goal.
2. **Allocation check (confirms it is not memory).** Run
   `target/release/mfb build --debug examples/brogue && examples/brogue/build/brogue.out dig 1 5 >/dev/null 2>err`
   and then `grep -E '^arena.0.(alloc_calls|alloc_bytes) ' err`. The expected
   output is 148,095 calls and 32,033,136 bytes.
3. **Profile.** Copy the example to `/tmp`. In the copy's `src/main.mfb`, wrap
   the `dumpDig(...)` call in `main` in `FOR rep = 1 TO 60 … NEXT`. Then run
   `mfb build -q <copy>`, start `<copy>/build/brogue.out dig 1 20 >/dev/null &`,
   and run `sample <pid> 10 1 -file prof.txt`. Take the "Sort by top of stack"
   offsets (add `0x100000000`) and bucket them by function against
   `objdump -d <copy>/build/brogue.out`. A function starts at each
   `sub sp, sp, …` that follows a `ret`/`b`. The binary is stripped, so name
   each function by the source line numbers its error paths load into `x1`
   (`mov x1, #line` followed by `mov x2, #col`), or by matching its body
   against `mfb build --ncode`.
4. **Correctness.** `examples/brogue/check/check-terrain.sh 8 1` must report
   all 320 pairs matching.

## Root Cause (measured profile, not yet fixed)

A sampling profile of the default (`-O1`) build, `dig 1 20` repeated 60 times
in one process (`sample <pid> 10 1`, 6,261 top-of-stack samples, attributed to
functions by splitting the stripped `__text` at each `sub sp, sp` prologue that
follows a `ret` and identifying each function by the source line numbers its
error sites load into `x1`):

| function (source) | share of samples |
| --- | --- |
| `cellularAutomataRound` (`src/grid.mfb`) body | 29.8% |
| `coordinatesAreInMap` (`src/grid.mfb`, one `AND` chain of four compares) | 19.4% |
| `gi` (`src/grid.mfb`, `x * DROWS + y`) | 13.0% |
| `nbDx` (`src/grid.mfb`, one `collections::get` of a module table) | 7.8% |
| `nbDy` | 6.9% |
| everything else | 23.1% |

So ~47% of all time is inside four one-expression helpers, and ~77% inside
`cellularAutomataRound` plus those helpers, which the C oracle
(`Grid.c cellularAutomataRound`, `-O1`) inlines to a handful of instructions
per neighbour. The emitted code for each helper shows three compounding costs
(`mfb build --ncode`, functions `gi`, `coordinatesAreInMap`, `nbDx`):

1. **No inlining of user functions.** There is no inliner in
   `src/optimizer/` (no pass in `opt1/` or `opt2/` inlines a module function);
   `gi` is a 96-byte frame with a `lr` save/restore and a `bl` at every one of
   its ~millions of call sites.
2. **Every temporary is spilled.** `gi`'s body stores `x0`, `x1`, the loaded
   `DROWS`, the product and the sum to stack slots and reloads each before use
   (`str x8, [sp, #32]` / `ldr x8, [sp, #32]` pairs); `coordinatesAreInMap`
   materializes each compare as a 0/1 in a register, stores it, reloads it and
   branches on it, four times.
3. **Module `LET` constants are memory loads.** `DROWS`/`DCOLS`
   (`PUBLIC LET … AS Integer = 29` in `src/tables.mfb`) are read from the
   globals block (`add x8, x19, #0xf38; ldr x9, [x8]`) on every use instead of
   folding to an immediate.

`-O3` helps but does not close it (same machine, `dig 1 20` best-of-5 user
time: O1 0.43 s, O3 0.32 s, oracle 0.02 s — noisy, lock not held).

What is NOT the cause (measured in bug-737): allocation volume (32 MB per
`dig 1 5`), list copying, or self-update decline.

## Goal

- `tools/brogue-perf/bench.sh` at or below the owner-chosen goal (bug-707
  proposed 10x) at the default optimization level, output byte-identical to
  the oracle (`examples/brogue/check/check-terrain.sh 8 1`).

### Non-goals (must NOT change)

- Do not rewrite the example to hand-inline its helpers: calling a small
  helper is ordinary MFBASIC and the compiler should make it cheap. (bug-737
  measured a hand-inlined `cellularAutomataRound` at only ~11% faster on `dig 1 20` (0.44 s → 0.39 s best-of-5, lock not held),
  because the inlined body keeps costs 2 and 3.)
- Error locations (`ErrorLoc` file/line/column) of an inlined helper's checks
  must still name the helper's source line.

## Blast Radius

Every MFBASIC program with small helper functions in hot loops. Not audited
beyond `examples/brogue`; the fix is a compiler optimization with byte-identity
golden churn tree-wide.

## Fix Design

Not designed. Candidates, in the order the profile ranks them: (a) fold
module-level `LET` scalars of literal value to immediates; (b) keep
single-use temporaries in registers at `-O1`; (c) inline leaf module
functions below a size threshold. Each needs its own plan; (c) interacts with
error-location reporting and with the plan-147 `$own<mask>` variants.

## Phases

### Phase 1 — pick the first mechanism and measure it alone

- [ ] Re-profile at HEAD, choose (a), (b) or (c), and write its plan.

Acceptance: a plan with a measured prediction of its share.
Commit: —

## Validation Plan

- Regression test: `tools/brogue-perf/bench.sh` with its budget tightened to
  each new floor.
- Runtime proof: `examples/brogue/check/check-terrain.sh 8 1`.
- Full suite: `cargo test` + `scripts/artifact-gate.sh target/release/mfb all`.

## Open Decisions

- **Scope (owner decision, 2026-09-26): do the full push to the goal**, meaning
  get to within 10x of the oracle, as long as language semantics do not change
  or break. Observable behavior, error locations and the plan-147 variant ABI
  all stay as they are.
- Which of (a)/(b)/(c) first — recommended (a), the cheapest and a prerequisite
  for (c) to pay off.

## Summary

The remaining gap is general code quality on call-dense integer loops, not a
memory mechanism; it needs optimizer work, not an example change.

## Progress (2026-09-27)

- On `worktree-B-740`: bug-747 (fixed, archived); plan-161-A (`2d170c339`),
  plan-161-B (`8b36c06e3`, `d34fdae20`, `71c76232a`), plan-161-C (`65781f6b9`);
  main merged (`f284af34d`, which brings bug-746's riscv64 fix).
- Measured without the lock (median of 5, debug-built compiler — codegen is
  identical to release): base 16.3x → A+B+C 5.6x.
- brogue `dig` vs oracle, seeds 1–2 × depths 1–40: linux-x86_64 80/80 (Docker
  `linux/amd64` emulation — boxes 2227/2228 were down all session),
  linux-aarch64 80/80 (box 2226), linux-riscv64 80/80 (box 2229),
  windows-x86_64 80/80 (the real Windows box 206); macOS host 320/320.
- Remaining: goldens, full gates, bench under the lock; budget commits.
