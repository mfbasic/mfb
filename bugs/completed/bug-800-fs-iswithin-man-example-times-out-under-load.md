# bug-800: `fs::isWithin` man examples time out (60 s) in the man-examples gate under load

Last updated: 2026-10-03
Effort: small
Severity: MEDIUM
Class: Correctness (possible hang) / test reliability

Status: Fixed (2026-10-03, bug-staging-2)
Regression Test: `tests/runtime/rt_fs_realpath_posix_symbol.rs`; `scripts/man-examples-gate.sh` (`fs::canonicalPath`, `fs::isWithin`)

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

## Root cause (2026-10-03)

The batch-2 gate timed out five examples, all `realpath` users:
`fs::canonicalPath#1`, `#2` and `fs::isWithin#1`–`#3`.

- Run with `SCRATCH` under `/tmp`: all five in 1.5 s. Run under `$TMPDIR`
  (`/var/folders/…/T/…`, where the gate's `mktemp -d` puts it): 11 s, 3.5 s of
  it system time.
- One `fs::canonicalPath(".")` program with its cwd under `$TMPDIR` cost
  0.43 s (0.17 s system, 3 313 voluntary context switches, 1.4 G instructions);
  the same binary with its cwd in `/tmp` cost 0.00 s (11 M instructions). 50 calls in
  a loop: 8.36 s.
- `ls $TMPDIR | wc -l` → 353 832 entries (`/tmp`: 4 514).
- A C program calling `realpath(".")` 20 times under `$TMPDIR` took 0.00 s. The C
  compiler binds `realpath` to `_realpath$DARWIN_EXTSN`. The macOS backend imported
  the bare legacy `_realpath`. Called through the legacy symbol from C
  (`__asm("_realpath")`), 10 calls took 1.72 s (1.39 s system).

The legacy entry point resolves a relative path through a `getcwd` that reads
every parent directory, and `$TMPDIR` grows with every test run. That is why the
timeout was intermittent, got worse over time, and hit only the `realpath` users.

## Fix

`emit_realpath` and the `fs.canonicalPath`/`fs.isWithin`/`fs.openWithin` imports
use `DARWIN_REALPATH_SYMBOL = "_realpath$DARWIN_EXTSN"`
(`src/target/macos_aarch64/code.rs`). With a caller-supplied buffer the two entry
points agree. Compared from C for `nonexist`, `/tmp/nonexist`, `/tmp/nope/deeper`,
`""` and `/etc/hosts/x`, both return the same NULL and `errno` (2/2/2/2/20) and leave
the same partial path. A probe that references all 46 libSystem imports
(`nm -u`) shows `realpath` is the only one with a suffixed variant on arm64.

Verified: 50 calls under `$TMPDIR` 8.36 s → 0.00 s. Every `rt_fs_*` suite passes,
the `canonicalPath`/`isWithin`/`openWithin` man examples run 8/8, and
`rt_fs_realpath_posix_symbol` fails on the old binary (`["_realpath"]`) and passes
with the fix. Spec: `architecture/06_native.md`.
