# bug-860: compile memory grows ~cubically with inline TRAP handlers in one function — 458 handlers use 77 GB RSS and the compiler is SIGKILLed

Last updated: 2026-10-05
Effort: large (3h–1d)
Severity: HIGH
Class: Other (compiler scalability / robustness)

Status: Open
Regression Test: tests/perf or a compile-budget test: one FUNC with 150 inline-TRAP LETs whose handler reads `e.code` must build under a fixed RSS bound (to add)

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
