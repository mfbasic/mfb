# bug-800: `fs::isWithin` man examples time out (60 s) in the man-examples gate under load

Last updated: 2026-10-03
Effort: unknown (needs a reproduction)
Severity: MEDIUM
Class: Correctness (possible hang) / test reliability

Status: Open — intermittent, not reproduced on demand
Regression Test: `scripts/man-examples-gate.sh` (`fs::isWithin#1`, `#3`)

## Observation

`bash scripts/man-examples-gate.sh` reported `fs::isWithin` examples as
`TIMED OUT after 60s — an example must terminate` in two full-suite runs on
2026-10-03:

| run | tree | failures |
|---|---|---|
| baseline | main `285f146e9` | `fs::isWithin#3(run)` |
| batch-1 gate | `bug-staging` (bug-787/723/750/793/794) | `fs::isWithin#1(run)`, `fs::isWithin#3(run)` |

Every other example in `fs` ran, including heavier filesystem examples, in the
same runs. Both runs happened under heavy machine load: two concurrent full
suites, release builds, and `syspolicyd` at ~90% CPU assessing each freshly
linked binary.

## Not reproduced

- Each example built and run alone: instant, exit 0 (`/tmp/iswithin`,
  `/tmp/iw2`, examples 2 then 3 in one directory).
- `SCRATCH=/tmp/iwfs bash scripts/man-run-examples.sh fs --run isWithin`, twice:
  3/3 ran.
- `SCRATCH=/tmp/iwfs bash scripts/man-run-examples.sh fs --run` (all 98 `fs`
  examples in one scratch directory, so every earlier example's state is
  present): 98/98 ran.

## What is known about the code

`src/codegen/builtins/fs/gen_canonical.rs:lower_fs_is_within_helper` makes two
`arena_alloc` + host `realpath` calls and has only bounded loops: two byte copies
counting down a known length, and a prefix compare. Reading it found no path that
can spin.

## Hypotheses to test

1. First-launch assessment of a newly linked binary under load exceeds 60 s. This
   is not specific to `isWithin`, but its examples may land at an unlucky point
   in the run. Test: record each example's wall time in the gate; check whether
   the stall precedes `main`.
2. `realpath` on the host blocks: a path component resolving through a slow or
   contended mount. Test: `sample`/`spindump` the process when the gate's timeout
   is about to fire.

## Fix

Capture a stack (`sample <pid>`) from a timed-out run before changing anything.
If it is (1), the gate's timeout policy is the thing to fix. If it is a real
hang, it needs a RED test.
