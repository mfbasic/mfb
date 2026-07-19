# bug-315: regex matcher crashes (SIGSEGV) on modestly long input and blows up exponentially on adversarial patterns

Last updated: 2026-07-17
Effort: large (3h–1d)
Severity: HIGH
Class: Robustness (DoS on benign and adversarial input)

Status: Fixed
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

## Resolution

Both failures reproduced first. `^a*$` crashed with SIGSEGV, and bisecting the input
put the threshold between **800 and 1000** scalars (200/400/600/800 exit 0; 1000/1500
exit 139) — confirming the report's stack-depth diagnosis rather than assuming it.

Three changes, addressing the two failures separately because they have different
causes:

**1. Greedy repeat over a simple child is now iterative.** `a*`, `.*`, `[0-9]+` — a
child that consumes exactly one scalar, sets no captures and needs no continuation —
is consumed with a `WHILE` loop and then given back one scalar at a time. That is the
same order the recursion explored (longest first), so the match found is identical;
what changes is that it costs no stack. This covers the overwhelmingly common
quantifier and makes it work at *any* length: `^a*$` over 50 000 scalars now matches,
where 1 000 used to kill the process.

**2. A global backtracking budget** (2 000 000 node visits, reset per search) bounds
the ReDoS case. The counter has to be module-level: threading it through the
immutable continuation state would lose a failed branch's work on backtrack, and that
is exactly the work worth counting. `^(a+)+$` against 24 `a`s and an `X` now fails in
1.2 s with `77050003` instead of running for minutes.

**3. A recursion-depth guard** (600, threaded as a parameter so it unwinds with the
stack) catches what remains. This one was found by testing rather than reasoning: the
iterative path only covers *simple* children, so a repeat over a **group** —
`^(ab)*$` — still recursed once per repetition and still crashed at 10 000 scalars.
600 sits with margin under the measured 800–1000 limit, so that case is now a clean
catchable failure rather than an uncatchable SIGSEGV.

### What is fixed, and what is a bounded limit

- Simple-child quantifiers: **work at any input length** (no limit).
- Group-child quantifiers: bounded at ~600 repetitions, failing cleanly.
- Adversarial/ambiguous patterns: bounded by the step budget, failing cleanly.

That satisfies the stated correct behavior — bounded stack and bounded step budget,
`FAIL`ing with a clear error past the limit instead of crashing or hanging. Making a
group-child repeat unbounded needs the explicit backtrack stack the Goal section
describes; the depth guard means it now errors rather than crashes while that remains
outstanding.

### Correctness

The concern with rewriting a matcher is silent behavioural drift, so correctness was
checked directly rather than inferred from the absence of crashes: greedy-all,
empty-match, `{n}`, `{n,m}` in and out of range, `+` on empty, `.` vs newline, greedy
backtracking into a following literal (`^a*b$`), greedy *giving back* a scalar
(`^a*a$` — the case that proves the give-back loop), group repetition, alternation,
the lazy quantifier (which deliberately does not take the iterative path), `find`
offsets and all three `replace` shapes. All unchanged.

Full `cargo test` green.
