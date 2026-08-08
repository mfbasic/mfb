# plan-86-I — regex compiled handle + replace slice-build (capped floor)

Sub-plan **I** of [plan-86](plan-86-benchmark-perf.md). Open. **Structural floor — do not gate on parity.**

**Covers (2 P1, capped):** regexbench replace (25.1), alternation (19.1).

## Root cause
`src/builtins/regex_package.mfb`: `__regex_replace` recompiles per call, `makeCtx` builds a dual
`List OF String` + `List OF Integer`, and the replace loop does `strings::mid` re-walks + `out & …` **O(n²)
concat**. Patterns `[0-9]+` (Class) and `cat|dog|…` (Alt) get `requiredFirstCp = -1`, so plan-77 R5's
first-scalar prefilter does not fire → the full interpreted CPS matcher runs at every start position × N
matches = O(n²).

## Fixes
- [ ] **I1** — a compiled-pattern handle so compile/find/replace parse once and reuse the program (retires
  the recompile; helps `regexbench compile`).
- [ ] **I2** — build `replace` output from `ctx.chars` slices instead of `out & …` (kills the O(n²)
  accumulation); hoist `toScalars(repl)` out of the match loop.

## Acceptance
regexbench/parse checksums + match counts + `scripts/artifact-gate.sh`.

## Note
An interpreted CPS backtracking matcher over `List OF …` cannot reach C POSIX-NFA / CPython `re`
(25 ms → 0.03 ms). I1/I2 are large constant-factor wins; **bound the expectation, do not gate on parity.**
