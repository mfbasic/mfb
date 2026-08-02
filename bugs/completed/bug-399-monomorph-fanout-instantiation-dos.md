# bug-399: monomorphization has no total-instantiation budget → exponential fan-out DoS (compiler never terminates on a ~7-line generic)

Last updated: 2026-07-28
Effort: medium (1h–2h)
Severity: HIGH
Class: Security (DoS on untrusted input) / Robustness

Status: FIXED (7da77e30c, merged dbed8e93c)
Regression Test: tests/syntax/monomorph/monomorph_instantiation_fanout_bounded
(the doc's exact fan-out repro → a single bounded diagnostic + prompt exit) plus
a white-box unit test (`total_instantiation_budget_halts_wide_fanout`) driving
`charge_instantiation` past the budget.

## STATUS: FIXED (7da77e30c, merged to main dbed8e93c)

Monomorphization now bounds breadth as well as depth. Two shared bounds are
checked at both instantiation entry points (`instantiate_function`,
`instantiate_type`), via a new `charge_instantiation` helper:

- **Total-instantiation budget** — `MAX_TOTAL_INSTANTIATIONS = 4096` (functions +
  user types), counted by a monotonic `total_instantiations`. Wide fan-out that
  reaches it is rejected with the new rule `2-203-0135
  TYPE_INSTANTIATION_BUDGET_EXCEEDED`.
- **Halt-on-first-limit** — `instantiation_limit_reached` latches the moment
  *either* limit trips (this budget or the existing depth cap), so enumeration
  stops after a **single** bounded diagnostic instead of re-reporting on every one
  of the exponentially-many sibling leaves. This is what actually terminates the
  doc's deep fan-out: its first DFS path legitimately hits the depth cap (256),
  and the latch then prunes the rest of the tree.

Verification (`target/debug/mfb`, macOS aarch64):

- The doc's Failing Reproduction now emits **one** `TYPE_INSTANTIATION_TOO_DEEP`
  and exits in ~0.06s (was: non-terminating, killed by the 10s CPU `ulimit` at
  exit 152 after ~800 diagnostics).
- The budget path itself (`TYPE_INSTANTIATION_BUDGET_EXCEEDED`) was confirmed
  end-to-end on a bounded-depth wide fan-out (~4096 distinct user-record
  instantiations → a single budget diagnostic, exit 1). It is not a committed
  *fixture* because reaching the several-thousand budget with deepening types is
  inherently ~seconds of CPU (each instantiation does O(type-size) work); the
  committed unit test drives the counter directly instead.
- Full suite green: `cargo test` 3708 unit + all integration binaries 0 failed.

Deviation from the original Blast Radius: the fix adds `halt-on-first-limit`
alongside the requested budget. The budget alone bounds *count/RSS* but, because
DFS front-loads a single deep path, the deep fan-out repro would still emit a
handful of depth diagnostics before the budget tripped; the latch makes the
output a clean single diagnostic and is the component that actually halts the
repro. Non-goals held: the 256-depth cap is unchanged and normal generic
programs, well under both limits, are unaffected.

---


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
