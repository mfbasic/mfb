# bug-860: compile memory grows ~cubically with inline TRAP handlers in one function — 458 handlers use 77 GB RSS and the compiler is SIGKILLed

Last updated: 2026-10-10
Effort: large (3h–1d)
Severity: HIGH
Class: Other (compiler scalability / robustness)

Status: Fixed (batch 20, 2026-10-10) — the fourth-power memory term. The Goal line "n=150 under 1 GB" is not met (1.14 GB); by owner decision (2026-10-10) that remainder is bug-1035.
Regression Test: `tests/codegen/codegen_stack_slot_promotion.rs::many_inline_trap_handlers_build_in_bounded_memory` (the 100-handler program must build under a 1.5 GB peak resident set, read from `wait4`), and `optimizer::opt2::promote::tests::the_per_slot_search_agrees_with_the_dense_dataflow` / `the_search_visits_only_the_blocks_a_slot_is_live_into` (`cargo test --bin mfb -- optimizer::opt2::promote::`)

A function that contains many `LET x = <fallible call> TRAP(e) … END TRAP` statements whose
handler *reads the error binding* (`toString(e.code)`) makes native codegen's register
allocation use memory super-linearly in the handler count. A straight-line test `main`
with 458 such cases (an ordinary table-driven test shape, ~3200 lines) reached
**76.9 GB max RSS in 96 s, and the OS killed the compiler (exit 137) with no
diagnostic.** Splitting the same cases one per function builds in 0.3 GB, so this is
not a code-size limit. Users get a hard crash on a plausible program.

Correct behavior: compile memory and time stay roughly linear in function size, so 458
handlers in one function build like 458 handlers spread over 458 functions.

References:

- AGENTS.md / `.ai/compiler.md` (production-ready: no crash on valid input)
- Found while exercising src/docs/spec/stdlib/08_encoding.md with a generated test program.

## Failing Reproduction

Generator (`n` handlers, each reading `e.code`):

```
python3 - <<'PY' 150
import sys
n=int(sys.argv[1])
L=["IMPORT io","IMPORT encoding","","FUNC main() AS Nothing"]
for i in range(n):
    L.append(f'''  LET r{i} AS String = encoding::percentDecode("a%20b") TRAP(e)
    io::print(toString(e.code))
    RECOVER ""
  END TRAP''')
L.append("END FUNC")
open('/tmp/hardening/B04/scal/src/main.mfb','w').write("\n".join(L)+"\n")
PY
/usr/bin/time -l /tmp/hardening/mfb build -q /tmp/hardening/B04/scal
```

(150-handler instance saved as `/tmp/hardening/B04/scal_n150.mfb`.)

Measured (`/usr/bin/time -l`, macOS aarch64, release `/tmp/hardening/mfb`):

| handlers n | wall s | max RSS |
| --- | --- | --- |
| 25 | 0.65 | 82 MB |
| 50 | 0.99 | 188 MB |
| 100 | 2.74 | 1.35 GB |
| 150 | 6.41 | 5.34 GB |
| 100, `-O0` (e.code + use-after variant) | 2.92 | 0.59 GB |
| 150, `-O0` | 10.22 | 0.66 GB |
| 458 (mixed encoding cases, `/tmp/hardening/B04/encx` before splitting) | 96.5 then SIGKILL | 76.9 GB |

From 100 to 150 handlers RSS goes ×4.0, about n^3.4. Variants at n=100: handler that does
*not* read `e` (`io::print("x")`) → 120 MB; no handler body → 115 MB; handler reads `e.code`
→ 1.35 GB; reads `e.code` **and** the bound value is used after the TRAP → 4.04 GB.

- Observed: superlinear RSS; at realistic sizes the process is killed (exit 137, no message).
- Expected: near-linear growth; the 458-case program builds.

## Root Cause

`mfb build -vv` on the n=100 `e.code`+use-after variant (5.1 s total) attributes the time
to codegen → `lower_function` → `register allocation` (3.2 s): **stack slot promotion
2.04 s** and **coloring 0.95 s** for 62 functions, nearly all in `main`. Hypothesis: each
inline-TRAP handler that materializes the error record keeps its error-binding vregs
interfering with every later handler's (they all share the name `e` and lower to fresh
slots that stay live across the following handlers). Something in stack-slot promotion
or the interference build is O(n^2) in pairs, and keeps O(n) state per pair. The `-O0` row
(0.66 GB at n=150) shows the opt-level passes make it much worse but are not the only
term. Confirm with a heap profile of `stack slot promotion` at n=100 vs 150.

## Goal

- n=150 builds in < 1 GB RSS at default opt level; n=458 builds.
- No change in emitted code semantics (goldens for TRAP fixtures unchanged).

### Non-goals (must NOT change)

- Do not cap function size or emit a "function too large" error as the fix.

## Blast Radius

- Any large generated/table-driven function with many inline TRAPs (test suites, codegen'd dispatchers). Latent wherever handlers read the error binding.

## Resolution so far (bs20-agent-a, 2026-10-10)

### Root cause (measured)

`optimizer::opt2::promote::read_before_written` — the check that keeps a frame
slot some path loads before storing out of stack-slot promotion — solved its
backward liveness with three dense bit matrices, `gen`, `kill` and `live_in`, each
`blocks × ceil(candidate slots / 64)` words. In this program both factors grow
with the SQUARE of the handler count, so the matrices grow with its fourth power.

The instruction count of `main` is quadratic because every exit path of every
handler repeats the drop of every `r<i>` bound before it. `mfb build -vv`,
"largest lower_function", this doc's generator:

| handlers n | instructions in `main` | slots promoted (whole build) | executable |
| --- | --- | --- | --- |
| 25 | 59,245 | 11,685 | 0.51 MB |
| 50 | 200,945 | 32,285 | 1.14 MB |
| 100 | 731,845 | 110,985 | 3.60 MB |
| 458 | 14,173,313 | | 67.8 MB |

That is `66 n² + 718 n` (it predicts 14,173,268 at 458). The repeated cleanup is
visible in `mfb build -q -ncode`: `main` calls `_mfb_rt_drop_owned_string` 389,
1,379 and 5,159 times at n = 10, 20 and 40, and `_mfb_arena_free` 150, 500 and
1,800 times. The hypothesis in "Root
Cause" above — error-binding vregs interfering across handlers in the allocator —
is not what the measurement shows: the allocator's own liveness is already sparse
(`regalloc/analysis.rs:analyze`, `U32Set` per block), and its memory is linear in
the instruction count.

### The fix

`read_before_written` now answers one slot at a time (`EntryLiveness`): a
backward worklist from the blocks whose first access of the slot is a load,
through predecessors, stopping at a block that stores the slot, and reporting
whether it reaches the entry block. Two stamp arrays of one word per block are
shared by every search. That is the same least fixed point
(`in[b] = gen[b] ∪ (out[b] − kill[b])`), read at the same place, so the set of
promoted slots is unchanged; the memory is linear in the stream and the work is
the number of (slot, block) pairs with the slot live into the block.

### Measurements (`/usr/bin/time -l mfb build -q`, macOS aarch64, release)

| handlers n | before: wall, max RSS | after: wall, max RSS |
| --- | --- | --- |
| 25 | 0.90 s, 100 MB | 1.13 s, 77 MB |
| 50 | 2.09 s, 421 MB | 1.22 s, 233 MB |
| 100 | 6.04 s, 4.04 GB | 3.51 s, 0.61 GB (0.53 GB on another run) |
| 150 | 17.78 s, 17.98 GB | 7.43 s, 1.14 GB (1.12–1.19 GB over four runs) |
| 458 | (this doc: SIGKILL at 76.9 GB) | 81.6 s, 9.90 GB, builds and runs |

"Before" is this branch's compiler without the fix (the doc's own figures are
from an older compiler, and were lower). `-O0`, where promotion is off, was
0.55 GB at n=100 and 1.20 GB at n=150 before the fix.

Emitted code is unchanged: `bash scripts/artifact-gate.sh target/release/mfb all`
checks 2322 goldens with 0 diffs, and the n=25 and n=100 executables and their
`-ncode -target linux-x86_64` dumps have the same SHA-256 built with and without
the fix. Expected golden changes: none.

### What is not met, and why it needs a decision

**Goal line "n=150 builds in < 1 GB RSS at default opt level" is not met: 1.14 GB.**
The other line (n=458 builds) is met.

What is left is linear in the instruction count, about 700 bytes per instruction
of `main` (0.61 GB / 731,845; 1.14 GB / 1,631,858; 9.90 GB / 14,173,313), and it is
the same with promotion off (`-O0`, 1.20 GB at n=150). The instruction count is the
quadratic term. Two ways to get under 1 GB, neither inside this bug's "no change in
emitted code":

1. Emit each exit path's cleanup once and branch to it, so the code is linear in
   the handler count. This changes emitted code for every function with more than
   one exit path (every golden with a cleanup moves).
2. Cut the compiler's bytes per instruction by a quarter or more (the instruction
   stream itself, `regalloc::analysis::Effect`, the per-instruction and per-vreg
   side tables of `linear_scan::run`). Flattening two of those tables
   (`analyze`'s `vdef`/`vuse`, `sweep_runs`' event lists) was tried and moved the
   n=150 peak by less than its run-to-run spread (1.13–1.19 GB), so it was not
   kept. Peak-RSS probes put the n=150 high-water marks at 679 MB entering the
   integer allocation, 766 MB after the effects, 957 MB after liveness and
   1,130 MB after coloring.

### Remainder (owner decision, 2026-10-10)

Asked which of the two ways above the bug should take, or whether 1.14 GB is acceptable,
the owner chose both, as a new bug: bug-1035. This bug is closed with the fix above.
