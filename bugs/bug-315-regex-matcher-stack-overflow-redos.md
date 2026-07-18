# bug-315: regex matcher crashes (SIGSEGV) on modestly long input and blows up exponentially on adversarial patterns

Last updated: 2026-07-17
Effort: large (3h–1d)
Severity: HIGH
Class: Robustness (DoS on benign and adversarial input)

Status: Open
Regression Test: tests/rt-error (new) — matching `^a*$` over a long string, and `^(a+)+$` over an adversarial string, return within a bounded step budget without crashing

The MFBASIC-source regex matcher is continuation-passing with no trampolining and no
step budget, giving it two robustness failures:

1. **Stack overflow on benign input.** A consuming node calls its continuation
   without unwinding, and each greedy iteration recurses through
   `ContRep → __regex_matchRep → __regex_matchNode`, so native call-stack depth grows
   ~1 frame per consumed scalar. Each frame is large (union MATCH dispatch + caps
   threading), so the stack is exhausted after a few hundred to ~1000 scalars. Any
   `*`/`+`/`.`/`{n,}` scanned over a paragraph of text crashes the process with an
   uncatchable SIGSEGV rather than a `FAIL`.

2. **Catastrophic backtracking (ReDoS).** Pure backtracking with no memoization and
   no step cap explores exponentially many input partitions for nested/ambiguous
   quantifiers. The engine accepts untrusted patterns *and* untrusted text, so this
   is a DoS vector.

The single correct behavior a fix produces: the matcher runs in bounded stack and a
bounded step budget for any pattern/input, `FAIL`-ing with a clear error past the
limit instead of crashing or hanging.

References:

- `mfb man regex` / `mfb spec stdlib regex` (matcher accepts untrusted patterns and
  text; documented as backtracking, never claims linear time).
- Found during goal-06 review of `src/builtins/regex_package.mfb`.

## Failing Reproduction

```
' regex::match(strings::repeat("a", 1000), "^a*$")   -> exit 139 (SIGSEGV)
' regex::match(strings::repeat("a", 2000), "^a*$")   -> exit 139
' regex::match("aaaaaaaaaaaaaaaaaaaaX", "^(a+)+$")   -> N=20: 2.87s; N=24: >2 min
```

- Observed: SIGSEGV on ~1000+ scalars of a quantified match; exponential time on
  `^(a+)+$`.
- Expected: a bounded result or a clean `FAIL` (documented complexity limit).

`N=200/500` run without crashing → a stack-depth threshold, the signature of stack
overflow.

## Root Cause

`src/builtins/regex_package.mfb:685-785` (`__regex_matchNode` / `__regex_matchCont` /
`__regex_matchRep`): continuation-passing recursion with stack depth ∝ input length
(item 1); `src/builtins/regex_package.mfb:739-785` (`__regex_matchRep` /
`__regex_matchAlt`): pure backtracking with no memoization or step budget — the
empty-iteration guard (`:780`) stops infinite loops but not exponential blowup
(item 2).

## Goal

- Convert the repeat/continuation loop to an explicit heap-allocated worklist/stack
  so match depth does not consume native stack.
- Thread a global backtrack/step counter through the matcher and `FAIL`
  (e.g. `error(77050003, …)`) when exceeded.

### Non-goals (must NOT change)

- Match results for inputs within the budget (correctness must be preserved).
- The public `regex::` API.

## Blast Radius

- `__regex_matchNode`/`__regex_matchCont`/`__regex_matchRep`/`__regex_matchAlt` —
  fixed here.
- All `regex::` entry points (`match`/`find`/`replace`/`split`) route through the
  matcher — all benefit.

## Fix Design

Iterative worklist matcher (explicit backtrack stack on the arena/heap) plus a step
budget; on budget exhaustion `FAIL` with a documented complexity-limit error.
Memoization (NFA-simulation / Thompson) is the stronger fix and removes exponential
blowup entirely — worth considering given the untrusted-input exposure. Rejected:
merely raising the native stack limit — input-length-proportional recursion cannot be
bounded that way.

## Phases

### Phase 1 — failing test
- [ ] rt-error tests for the long-input crash and the `^(a+)+$` blowup (with a step
      budget assertion). Confirm both fail today.
### Phase 2 — the fix
- [ ] Iterative worklist + step budget (and/or memoized matcher).
### Phase 3 — validation
- [ ] Full regex acceptance suite green; the repros terminate cleanly; results
      unchanged within budget.

## Validation Plan

- Regression: long-input and ReDoS tests.
- Runtime proof: no SIGSEGV; adversarial pattern `FAIL`s within the budget.
- Doc sync: document the step/size limit in the regex man/spec.

## Summary

The regex engine crashes on benign paragraph-length input and hangs on adversarial
patterns because it recurses per scalar with no step cap. An iterative worklist plus
a step budget (ideally memoization) fixes both; this is the largest single fix in the
goal-06 batch and matters because regex takes untrusted input.
