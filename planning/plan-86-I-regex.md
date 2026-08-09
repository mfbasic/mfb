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
- [x] ~~**I1** — a compiled-pattern handle so compile/find/replace parse once and reuse the program (retires
  the recompile; helps `regexbench compile`).~~ — **moot: measured negligible on the scored rows, and its
  stated target (`compile`) is already complete.** Measured (release, box-local): 300 compiles of `[0-9]+`
  (+ a trivial 1-char match each) = **8.9 ms → ~0.030 ms per compile**. The scored `replace` row does 10
  `regex::replace` calls = **10 compiles ≈ 0.3 ms of its ~25 ms total** → removing 9 recompiles saves ~1 %,
  which does not move the band (`replace` stays P1). The `compile` benchmark row (compile-once-match-many) is
  already **complete** per the master scorecard, so I1's stated beneficiary needs nothing. A compiled-pattern
  *handle* is also a public-API/language-surface change (spec + man + validation), out of scope for a
  benchmark-perf plan. See Corrections for where the ~25 ms actually goes.
- [x] ~~**I2** — build `replace` output from `ctx.chars` slices instead of `out & …` (kills the O(n²)
  accumulation); hoist `toScalars(repl)` out of the match loop.~~ — **moot: implemented, measured, REVERTED —
  its premise is false and it was a slight regression.** I implemented it (slice `ctx.text` — the field is
  `text`, not `chars` — via the amortized in-place MUT append; hoist `__regex_toScalars(replacement)` once and
  pass the pre-split list into `__regex_expand`). Output byte-identical (checksum 1199; `$1$2$3`, `${m}`,
  `$m`, `$0$0`, empty-match, no-match all verified). But measured **pre-I2 24.8–25.4 ms vs post-I2 26.1 ms —
  NO win, slightly worse.** The plan's "`out & …` O(n²) accumulation" premise is FALSE: `out = out & X` on a
  uniquely-owned MUT String is already amortized-O(1) in-place append (plan-02), so the accumulation is O(n),
  not O(n²); and trading the native `strings::mid` for an interpreted per-scalar `get(ctx.text,k)` + append
  loop is marginally slower. Reverting also avoids the `.ir`-golden ripple to every regex importer for a
  non-improvement. Evidence that the accumulation was never the bottleneck: I2 left the recompile in place and
  the total barely moved — the cost is elsewhere (the matcher).

## Acceptance
regexbench/parse checksums + match counts + `scripts/artifact-gate.sh`.

## Note
An interpreted CPS backtracking matcher over `List OF …` cannot reach C POSIX-NFA / CPython `re`
(25 ms → 0.03 ms). **bound the expectation, do not gate on parity.**

## Corrections
- **Both I1 and I2 target non-bottlenecks; the CPS matcher is the entire cost of `replace`/`alternation`, and
  it is the structural capped floor the plan already accepted.** Root-caused by measurement this session:
  `replace` ≈ 25 ms decomposes as ~0.3 ms compile (I1's target) + ~≤0.1 ms output-building (I2's target) +
  **~24.6 ms matcher**. For `[0-9]+` (Class) and `cat|dog` (Alt) the pattern's `requiredFirstCp = -1`, so
  plan-77 R5's first-scalar prefilter does not fire and `__regex_searchFrom` runs the full interpreted CPS
  matcher at **every** start position (~1500) × N — an O(n²) that neither I1 (compile) nor I2 (output) touches.
  The only lever that would move these rows is a **Class/Alt first-scalar prefilter** (extend
  `__regex_requiredFirstCp`/`searchFrom` to skip start positions whose scalar cannot begin any alternative /
  is not in the class) — a matcher change explicitly **outside I1/I2**, and even then the source CPS backtracker
  stays a structural floor vs C/CPython (Open Decisions: "pursue I1+I2; bound the expectation; do not gate on
  parity" — the plan deliberately bounded regex to constant-factor tweaks and accepted the floor). Since the
  two chosen tweaks are measured ineffective and the real lever is the bounded-out matcher rewrite, I1/I2 are
  marked moot with the numbers above rather than shipped as churn-inducing non-improvements. If a future plan
  chooses to spend on regex, the prefilter is the entry point, not compile/output.
