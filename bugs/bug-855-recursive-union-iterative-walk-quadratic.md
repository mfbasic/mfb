# bug-855: walking a recursive union with `MUT cur = c.rest` deep-copies the remaining tail every step — O(n²) time and allocations

Last updated: 2026-10-05
Effort: medium (1h–2h)
Severity: LOW
Class: Footgun

Status: Open
Coordinator note: repro measured by the review agent only; the coordinator did not re-run it.
Regression Test: tests/perf or rt fixture counting `--debug` `arena.0.alloc_calls` for an iterative list walk — name suggested

**Owner decision (2026-10-05):** treat this as an optimization that must never break value semantics: "slow and correct
rather than fast and broken." In the repro `c` is never written, so nothing can change the
nesting and `MUT cur = c.rest` can read in place without copying the tail. A copy is still
made wherever semantics require one (for example when `cur` is returned, stored, or bound as
`LET a = cur` and then either side is mutated). The fix is admitted only where it is
provably unobservable. It is lower priority than the correctness bugs.

Class: UNCLEAR (performance; §14 permits but does not require the copy→move replacement).

The idiomatic iterative traversal of a recursive union list

```
MUT cur AS Lst = l
WHILE going
  MATCH cur
    CASE Cons(c)
      t = t + c.v
      cur = c.rest
    CASE Nil(z)
      going = FALSE
  END MATCH
END WHILE
```

allocates ~n²/2 blocks: each `cur = c.rest` deep-copies the entire remaining tail, then drops the
old `cur`. The equivalent recursive walk (`RETURN c.v + rtotal(c.rest)`) is linear. Three 300 000-node walks did
not finish within 120 s; 2 000 → 4 000 nodes goes 2.0 M → 8.0 M allocations.

Expected (owner decision): since the old `cur` is dropped by the same assignment, `c.rest` can be
moved out of it (§14.1: "The compiler may replace a semantic copy with a move when it proves the
source is not used afterward"), making the walk linear like the recursive form.

References:

- src/docs/spec/language/14_memory-semantics.md §14.1 (copy→move optimization), §14.2 (MUT
  reassignment drops the old value), §14.5 (recursive edges are owned children).

## Failing Reproduction

`/tmp/hardening/B03/memory/src/rec3.mfb` (build + iterative `total`), built with `--debug`:

| n | arena.0.alloc_calls | arena.0.alloc_bytes |
| --- | --- | --- |
| 2000 | 2 011 017 | 65 025 168 |
| 4000 | 8 022 017 | 258 049 168 |

Recursive version (`rec6.tmpl`, `rtotal`): n=2000 → 4 016 allocs; n=4000 → 8 016 allocs.
Build alone (`l = Cons[i, l]`, `rec5.mfb`) is linear (0.18 s for 16 000).

- Observed: quadratic allocations; values correct (2001000, 8002000).
- Expected: linear.

## Root Cause

Hypothesis (not traced in codegen): the MATCH binding `c` is a view into `cur`, and the
assignment `cur = c.rest` must evaluate the RHS (copy) before dropping the old `cur` (§14.2 "if
evaluating the right-hand side fails, the old value remains live"); no pass recognizes that the
copied subtree is owned by the value being dropped and can be detached instead.

## Goal

- Iterative walk allocation count linear in n.

### Non-goals (must NOT change)

- Value semantics: a LET copy of `cur` taken before the step must keep its full list.

## Blast Radius

- Any `MUT x = <field/child of x>` self-narrowing (trees: `node = node.left`).

## Fix Design

Detect `MUT x = <path rooted at a MATCH/field view of x>` and lower it as detach-child +
drop-rest-of-old. Rejected: making MATCH bindings copies (worse).

## Phases

### Phase 1 — perf fixture counting allocations. Commit: —
### Phase 2 — detach lowering. Commit: —
### Phase 3 — full suite + perf goldens. Commit: —

## Validation Plan

- Allocation-count fixture; full suite.

## Open Decisions

- Whether §14 should state a cost bound for self-narrowing (as §12 does for collection
  self-updates).

## Summary

Correct results, pathological cost on a natural idiom.
