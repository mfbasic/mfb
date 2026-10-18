# bug-1096: a program that calls `math::seed` but never `math::rand` still draws OS entropy at startup and links the unused `_mfb_rng_next`

Last updated: 2026-10-17
Effort: small (<1h)
Severity: LOW
Class: Dead-code

Status: Open
Regression Test: tests/codegen/codegen_seed_only_no_entry_entropy.rs (to be added)

A program that calls `math::seed` and never calls `math::rand` gets the full startup
seeding sequence anyway: the entry draws 8 bytes from the OS entropy source and calls
`_mfb_rng_seed_at`. It also links `runtime.rng_next`. Nothing reads that seed. Only
`math::rand` reads the generator's stream, and when `thread::start` seeds a worker
from the parent's stream, the worker cannot observe it either without `math::rand`.
The cost is one OS call at every start, one entropy import (a libc import on Linux and
macOS, `BCryptGenRandom` on Windows) and a dead function. The import also counts
against a program that otherwise needs no OS entropy at all: bare-metal targets, or a
static ELF.

**The single correct behavior a fix produces:** the entry seeds the generator from OS
entropy only when the program calls `math::rand`. `runtime.rng_next` is emitted only
when it is reachable. `math::seed` alone emits `runtime.rng_seed_at` (its own
lowering needs it) and no startup entropy draw or import. Every `math::rand`
sequence, seeded or not, is unchanged.

References:

- `./mfb spec memory program-startup` (`src/docs/spec/memory/08_program-startup.md`,
  "RNG Seeding and Start Time"), `./mfb spec stdlib math-rng`
  (`src/docs/spec/stdlib/07_math-rng.md`, "Main thread — OS entropy"), and
  `src/docs/spec/linker/03_import-selection.md` (the `math::rand`/`math::seed`
  bullet) all state the current behavior ("when the program uses
  `math::rand`/`math::seed`"). They change with the fix.
- Found while reviewing what a bare-metal target must remove (scratch notes
  `tmp/bm.md`, not committed). Sibling: bug-1097, the unused x86-64 `getentropy` import
  on the same path.

## Failing Reproduction

```
mkdir -p /tmp/b1096/src
cat > /tmp/b1096/project.json <<'EOF'
{ "name": "s", "version": "0.1.0", "mfb": "1.0", "kind": "executable",
  "sources": [ { "root": "src", "role": "main", "include": ["**/*.mfb"] } ],
  "entry": "main", "targets": ["native"] }
EOF
printf 'IMPORT math\n\nFUNC main AS Integer\n  math::seed(42)\n  RETURN 0\nEND FUNC\n' > /tmp/b1096/src/main.mfb
for t in linux-x86_64 linux-aarch64 linux-riscv64 macos-aarch64 windows-x86_64; do
  target/release/mfb build -q --ncode --target $t /tmp/b1096 >/dev/null
  python3 -I -c '
import json,sys
d=json.load(open(sys.argv[1]))
e=[f for f in d["functions"] if f["name"]=="program.entry"][0]
print(sys.argv[2],
  "seed_at calls in entry:", sum(1 for i in e["instructions"] if i.get("target")=="_mfb_rng_seed_at"),
  "| entropy imports:", [x["symbol"] for x in d["imports"] if "ntropy" in x["symbol"] or "GenRandom" in x["symbol"]],
  "| rng fns:", [f["name"] for f in d["functions"] if "rng" in f["name"]])' /tmp/b1096/s.ncode $t
done
```

- Observed (measured 2026-10-17, release build of main at 2a29186d0):
  - every target: `seed_at calls in entry: 1`; rng fns `['runtime.rng_next', 'runtime.rng_seed_at']`
  - entropy imports: linux-x86_64/aarch64/riscv64 `['getentropy']`, macos-aarch64
    `['_getentropy']`, windows-x86_64 `['BCryptGenRandom']`
  - the entry calls `getentropy` (aarch64, riscv64), `_getentropy` (macOS) or
    `BCryptGenRandom` (Windows), or makes the raw `getrandom` (nr 318) syscall (x86-64),
    right before `_mfb_rng_seed_at`
- Expected: `seed_at calls in entry: 0`, no entropy import, rng fns
  `['runtime.rng_seed_at']`.

Contrast cases (must not change):

- A program that calls `math::rand` (with or without `math::seed`) keeps the entry
  seed, its import and both rng functions.
- A program with neither call has no seed block, no import and no rng function today.

| Target | Result |
| --- | --- |
| linux-x86_64 | fails ✗ (raw `getrandom` + seed; also see bug-1097) |
| linux-aarch64 | fails ✗ |
| linux-riscv64 | fails ✗ |
| macos-aarch64 | fails ✗ |
| windows-x86_64 | fails ✗ |

## Root Cause

`src/codegen/engine/builder/mod.rs` computes one flag for two different needs:

```
let uses_rng = module_uses_call(module, "math.rand") || module_uses_call(module, "math.seed");
```

`uses_rng` is passed as the entry's `seed_rng` (the `seed_rng:` fields in the same
file). That makes `lower_program_entry` (`src/codegen/engine/function/entry.rs`, the
`if seed_rng` block) emit `platform.emit_random_bytes` + `_mfb_rng_seed_at`. The same
flag gates `code_functions.push(lower_rng_next()); code_functions.push(lower_rng_seed_at());`
in `builder/mod.rs`. `math::seed` needs `rng_seed_at` for its own lowering
(`src/codegen/builtins/math/func_seed.rs:lower_math_seed` → `lower_math_call("seed")`)
but needs neither the startup seed nor `rng_next`.

The per-target import tables copy the same conjunction: `"math.rand" | "math.seed"` in
`src/target/linux_common/plan.rs:LinuxPlan::native_call_imports`,
`src/target/macos_aarch64/plan.rs` (`native_call_imports`, "both `math::rand` and
`math::seed` keep the entry seed random"), and `src/target/win_x86_64/plan.rs`
(`native_call_imports`). The conjunction dates from the commit that introduced
`math::rand` (`git log -S'module_uses_call(module, "math.seed")'` → 04c0ff5fa). That
commit gives no reason why a program with only `math::seed` needs a random start
seed.

`module_uses_call` (`src/codegen/engine/analysis/module_analysis.rs`) scans every
function of the single NIR module a native build lowers, so "no `math.rand` call op" is
whole-program.

Hypothesis to eliminate in Phase 1 (unverified): some path reaches the PCG stream
without a `math.rand` call op, e.g. `math::rand` taken as a function value, or a
built-in package written in MFBASIC that draws from the stream. Phase 1 confirms or
eliminates it by grepping every `_mfb_rng_next` / `RNG_NEXT` reference and every
`math::rand` use in embedded package sources. If such a path exists, the predicate
must include it.

## Goal

- The repro prints `seed_at calls in entry: 0`, no entropy import and
  `['runtime.rng_seed_at']` on all five targets.
- `math::seed(42)` followed by `math::rand` still replays the same sequence (existing
  seeded-reproducibility tests stay green).

### Non-goals (must NOT change)

- The PCG64 algorithm, the arena RNG state words (88/96), `ENTRY_SEED_SCRATCH_OFFSET`, and the
  seeded or unseeded sequence of any program that calls `math::rand`.
- Thread seeding (`thread::start` draws the child seed from the parent stream) for
  programs that call `math::rand`.
- Do not "fix" this by dropping the seed for `math::rand` programs, or by making
  `math::seed` imply anything about global initializers. An unseeded `math::rand` must
  stay seeded before any user code, global initializers included.

## Blast Radius

Search: `grep -rn '"math.rand" | "math.seed"\|math.seed")' src`.

- `src/codegen/engine/builder/mod.rs` `uses_rng` — fixed by this bug (split into
  "needs entry seed / `rng_next`" = `math.rand`, and "needs `rng_seed_at`" =
  `math.rand || math.seed`).
- `src/codegen/engine/builder/mod.rs` `AbiCtx`/`thread.start` reading `uses_rng`
  (comment "`thread.start` reads `AbiCtx::arena_global_slots`/`uses_rng`") — must take
  the `math.rand` flag. Verify in Phase 1 that the child-seed draw is gated on the
  same predicate as `rng_next`.
- `src/target/linux_common/plan.rs:LinuxPlan::native_call_imports` — fixed: entropy
  import only for `math.rand`. (bug-1097 further drops it on x86-64.)
- `src/target/macos_aarch64/plan.rs:native_call_imports` — fixed, same change.
- `src/target/win_x86_64/plan.rs:native_call_imports` (`BCryptGenRandom`) — fixed, same change.
- Spec topics listed under References — updated in the same change (Hard Completion
  Gate, `.ai/specifications.md`).
- `crypto.randomBytes`, `fs.createTempFile` entropy imports — unaffected (separate
  call sites with their own imports).

Fixtures: `grep -rl 'math::seed' tests examples` lists 6 files, and all 6 also call
`math::rand` (counted with `grep -c`, 2026-10-17). So no byte-identity or golden
output is expected to change. Any golden diff is a bug-hunt trigger.

## Fix Design

Split the flag in `builder/mod.rs`:

- `seeds_at_entry = module_uses_call(module, "math.rand")` gates the entry `seed_rng`,
  `lower_rng_next`, and the thread child-seed draw.
- `uses_rng_seed_at = seeds_at_entry || module_uses_call(module, "math.seed")` gates
  `lower_rng_seed_at`.

Change the three `native_call_imports` arms from `"math.rand" | "math.seed"` to
`"math.rand"`.

Rejected: keeping the entry seed for `math::seed` "just in case". No reader exists
(pending the Phase 1 hypothesis check), and the import is exactly what bare-metal and
static targets need to lose.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add `tests/codegen/codegen_seed_only_no_entry_entropy.rs` (pattern:
      `tests/codegen/codegen_win64_entry_seed_scratch.rs`, `common::build_ncode`). It
      builds the repro program per target and asserts: no `_mfb_rng_seed_at` in
      `program.entry`, no entropy import, no `runtime.rng_next`. Add a control program
      using `math::rand` that asserts all three are present. Confirm the seed-only case
      fails on all five targets.
- [ ] Eliminate or confirm the Root Cause hypothesis (function value / embedded MFBASIC
      readers of the stream). Record the verdict here.

Acceptance: the new test fails for the documented reason; the hypothesis has a recorded
verdict.
Commit: —

### Phase 2 — the fix

- [ ] Split `uses_rng` in `src/codegen/engine/builder/mod.rs` as designed. Thread the
      `math.rand` flag to `seed_rng` and to the thread child-seed path.
- [ ] Narrow the three `native_call_imports` arms to `"math.rand"`.
- [ ] Update the three spec topics (and their `[[...]]` citations) to say the entry
      seed is drawn when the program calls `math::rand`.

Acceptance: the Phase 1 test passes; the control case still passes; the seeded-replay
tests stay green.
Commit: —

### Phase 3 — validation

- [ ] `scripts/artifact-gate.sh <mfb> math` (and `thread`): expect no `.ncodesum` change
      (no fixture is seed-only). Investigate any diff.
- [ ] Landing tier `bash scripts/test-fast.sh` + the new test by name.
- [ ] Re-run the repro on all five targets.

Acceptance: gates green with zero golden delta; the repro prints the expected output on all five targets.
Commit: —

## Validation Plan

- Regression test: `tests/codegen/codegen_seed_only_no_entry_entropy.rs`.
- Runtime proof: the repro above, plus an existing seeded-`math::rand` runtime test
  (`tests/rt-behavior/math/bug137_rand_unbiased_bounds`) still passing.
- Doc sync: the three spec topics under References.
- Landing gate: `bash scripts/test-fast.sh`, the new test, `scripts/artifact-gate.sh <mfb> math`
  and `thread`.
- Full tier: the scheduled background run.

## Open Decisions

- None expected. If Phase 1 finds a stream reader other than `math.rand`, widen the predicate
  to include it rather than reverting to `math.seed`.

## Summary

Low risk: one flag split and three import arms. The work that matters is the Phase 1
audit proving nothing except `math::rand` reads the stream.
