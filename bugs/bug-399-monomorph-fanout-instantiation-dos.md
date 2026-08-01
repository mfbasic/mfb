# bug-399: monomorphization has no total-instantiation budget → exponential fan-out DoS (compiler never terminates on a ~7-line generic)

Last updated: 2026-07-28
Effort: medium (1h–2h)
Severity: HIGH
Class: Security (DoS on untrusted input) / Robustness

Status: Open
Regression Test: tests/ — a compile fixture: the fan-out program below must fail
with a bounded "too many instantiations" diagnostic and exit promptly, not run
unbounded.

The monomorphizer bounds instantiation **depth** but not **breadth**. `template_
instantiation_depth` (`src/monomorph/lower.rs:648`, cap
`MAX_TEMPLATE_INSTANTIATION_DEPTH = 256`) limits recursion along a single path, but
a generic function that recurses through two (or more) distinct type-widening
self-calls fans out into a tree: `recurse<Integer>` →
`recurse<List OF Integer>` + `recurse<Set OF Integer>` → 4 → 8 → … Every path
produces a distinct `name<args>` key, so the `emitted_function_keys` memoization
never collapses siblings, and the depth cap fires only per-leaf (returning `None`)
without ever halting the exponential exploration (≈2^256 nodes). The result: the
process never terminates, `emitted_function_keys` grows without bound (unbounded
RSS), and a diagnostic is emitted per leaf (unbounded stderr/log/disk).

`instantiate_type` (`src/monomorph/lower.rs:775`) shares the same depth counter and
has the identical fan-out gap for a generic TYPE with two widening fields.

Same threat model as bug-182 (an author of an arbitrary `.mfb` the victim compiles
→ DoS on the builder), which was rated HIGH. bug-182's own Fix Design explicitly
specified a second mitigation — "A total-instantiation budget (e.g. a few thousand)
additionally bounds fan-out that is wide rather than deep" — but Phase 2 implemented
only the depth cap; a grep of `src/` confirms only `MAX_TEMPLATE_INSTANTIATION_
DEPTH` exists, no total-instantiation/budget cap anywhere. goal-05 marked
`src/monomorph/**` reviewed but did not catch this.

References:

- `src/monomorph/lower.rs:648` (`instantiate_function` depth-only cap), `:775`
  (`instantiate_type`, same gap).
- bug-182 (`bugs/completed/`), Fix Design lines 99–101 (unimplemented total budget)
  vs Phase 2 lines 117–124 (depth only). Found during goal-07.

## Failing Reproduction

```
mfb init /tmp/fanout
# overwrite /tmp/fanout/src/main.mfb:
FUNC recurse OF T(x AS T) AS Integer
  LET a AS List OF T = [x]
  LET b AS Set OF T = Set OF T { x }
  RETURN recurse(a) + recurse(b)
END FUNC
FUNC main() AS Integer
  RETURN recurse(1)
END FUNC
( ulimit -t 10; mfb build /tmp/fanout )
```

- Observed (verified 2026-07-28, `target/debug/mfb`): the build does NOT terminate;
  killed only by the 10s CPU-time `ulimit` (exit 152 = SIGXCPU), having emitted 816
  `2-203-0102 TYPE_INSTANTIATION_TOO_DEEP` diagnostics into a 258 KB-and-growing
  output. RSS climbs monotonically.
- Expected: a bounded "too many instantiations" error and prompt exit.

Contrast (immune): bug-182's single-branch repro (`RETURN recurse(a)` only —
one widening self-call) terminates cleanly at depth 256. The distinguishing factor
is ≥2 widening self-calls, which the depth-only cap cannot bound.

## Root Cause

No total-instantiation budget. `emitted_function_keys` (and the `concrete_types`
map) grow with the number of *distinct* `name<args>` tuples, which is exponential
in the tree depth when ≥2 widening self-calls exist; the per-leaf depth cap returns
`None` per leaf but does not stop the enumeration.

## Goal

- A global cap on total instantiations (functions + types), of a few thousand,
  after which monomorphization stops with a single bounded diagnostic and a
  non-zero-but-prompt exit — bounding wide fan-out the way the depth cap bounds
  deep recursion.

### Non-goals (must NOT change)

- No lowering of the legitimate depth cap; no change to correct monomorphization of
  normal generic programs well under the budget.

## Blast Radius

- `src/monomorph/lower.rs:648` (`instantiate_function`) and `:775`
  (`instantiate_type`) — both share the depth counter and both need the budget
  check (a single shared counter incremented at each instantiation entry point).
