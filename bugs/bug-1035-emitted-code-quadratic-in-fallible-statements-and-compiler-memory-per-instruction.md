# bug-1035: a function's emitted code is quadratic in its fallible statements (every exit path repeats the cleanup), and the compiler holds about 700 bytes per instruction — 150 inline TRAPs peak at 1.14 GB, 458 at 9.9 GB

Last updated: 2026-10-10
Effort: large (3h–1d)
Severity: MEDIUM
Class: Other (compiler scalability; code size)

Status: Open
Regression Test: tighten `tests/codegen/codegen_stack_slot_promotion.rs::many_inline_trap_handlers_build_in_bounded_memory` (today: 100 handlers under 1.5 GB) to the Goal below, plus an instruction-count test for item 1

This is what bug-860 left. That bug's fourth-power memory term is fixed; its goal of
"150 handlers build under 1 GB" is not met. **Owner decision (2026-10-10): do both items
below.** Item 1 changes emitted code, which bug-860's own goal forbade, so it is this
new bug.

1. **Share cleanup tails.** Every exit path of every fallible statement repeats the drop
   of every owned value bound before it, so a function with n such statements has an
   instruction count quadratic in n. Emit each cleanup once and branch to it.
2. **Allocator memory diet.** The compiler's peak memory is linear in the instruction
   count at about 700 bytes per instruction, the same with stack-slot promotion off.
   Cut it by a quarter or more with no change in emitted code.

**Correct behavior:** code size and compile memory grow linearly with the number of
fallible statements in a function.

## Failing Reproduction

Generator (n handlers, from bug-860), saved as `gen.py`:

```
import sys
n=int(sys.argv[1]); out=sys.argv[2]
L=["IMPORT io","IMPORT encoding","","FUNC main() AS Nothing"]
for i in range(n):
    L.append(f'''  LET r{i} AS String = encoding::percentDecode("a%20b") TRAP(e)
    io::print(toString(e.code))
    RECOVER ""
  END TRAP''')
L.append("END FUNC")
open(out,'w').write("\n".join(L)+"\n")
```

```
python3 gen.py 150 src/main.mfb
/usr/bin/time -l mfb build -q .
```

Measured by the coordinator with the bug-860 fix in (`/tmp/bs20-a-target/release/mfb`,
macOS aarch64, `/usr/bin/time -l`): n=100 peaks at 525 MB in 3.5 s; n=150 at 1,162 MB in
7.4 s. Expected at n=150: under 1 GB.

Measured by the bug-860 agent, not re-run by the coordinator:

| handlers n | instructions in `main` (`mfb build -vv`) | max RSS | executable |
| --- | --- | --- | --- |
| 25 | 59,245 | 77 MB | 0.51 MB |
| 50 | 200,945 | 233 MB | 1.14 MB |
| 100 | 731,845 | 0.61 GB | 3.60 MB |
| 458 | 14,173,313 | 9.90 GB (81.6 s) | 67.8 MB |

The instruction count is `66 n² + 718 n`. In `mfb build -q -ncode` output, `main` calls
`_mfb_rt_drop_owned_string` 389, 1,379 and 5,159 times at n = 10, 20 and 40.

## Root Cause

Item 1: each exit path of a handler emits its own copy of the scope cleanup (not traced
to a function; start from the `_mfb_rt_drop_owned_string` call sites in the `-ncode`
dump).

Item 2: the bug-860 agent's peak-RSS probes at n=150 put the high-water marks at 679 MB
entering integer allocation, 766 MB after the effects (`regalloc::analysis::Effect`),
957 MB after liveness and 1,130 MB after coloring (`linear_scan::run`). Flattening
`analyze`'s `vdef`/`vuse` and `sweep_runs`' event lists was tried and moved the peak by
less than its run-to-run spread, so it was not kept. Coloring time is also mildly
superlinear: 2.66 s at 1.63M instructions, 33.0 s at 14.2M (`mfb build -vv`,
"coloring").

## Goal

- n=150 builds under 1 GB at the default opt level; the regression test's bound is
  tightened to match.
- The instruction count of the generator's `main` is linear in n.
- Item 2 alone changes no emitted code (`scripts/artifact-gate.sh <mfb> all`: 0 diffs).

### Non-goals (must NOT change)

- Do not cap function size or emit a "function too large" error.
- Item 1 changes code layout only: the order in which values are dropped and resources
  are closed on each exit path stays as it is.

## Blast Radius

- Item 1 moves the native goldens of every fixture whose functions have a cleanup on
  more than one exit path. Each needs the function-level proof before it is regenerated.
  Do it as its own phase step, after the behaviour fixes it would otherwise blur.
