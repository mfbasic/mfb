# bug-754: a `Float` overflow inside a builtin vanishes when the result is an operand or an inline builtin's argument

Last updated: 2026-09-27
Effort: medium
Severity: HIGH
Class: Correctness (silent wrong value; error not raised)

Status: Fixed
Regression Test: `tests/rt-behavior/arithmetic/bug754-float-overflow-raises-inside-expression-rt`;
unit test `float_result::tests::no_classified_member_is_rechecked_at_a_boundary`

Found in the coordinator's review of bug-644. With no `TRAP`,
`LET c AS Float = 1.0 / collections::sum([1.0e308, 1.0e308])` prints `c=0.00` and
exits 0, while the same `sum` bound on its own raises `7-705-0015`. Whether an
overflow raises must not depend on where the value sits in an expression.

**The single correct behavior:** a builtin whose `Float` result overflows raises at
the call, and a `Float` argument of any builtin is an observation boundary, whatever
the enclosing expression and with or without a `TRAP`.

## Reproduction and sweep

Each value below is computed with no `TRAP` of its own and propagated to a trapping
caller (the regression fixture). At base (the main checkout's release compiler,
`b1a563acd`), and on this branch before the fix:

| Expression | Before | After |
| --- | --- | --- |
| `1.0 / collections::sum([1e308, 1e308])` | `0.00` | `77050015` |
| `collections::sum(...) + 1.0` | `77050015` | `77050015` |
| `math::abs(x * x)`, `x = 1e200` | `1.797e308` (`+Inf` bound) | `77050015` |
| `math::atan(x * x)` | `1.57` | `77050015` |
| `math::min(x * x, 1.0)` | `1.00` | `77050015` |
| `math::exp(-(x * x))` | `0.00` | `77050015` |
| `math::max`, `math::sqrt`, `math::log` of `x * x` | `+Inf` / `709.78` | `77050015` |
| `vector::length(v)`, `v` = three `1e200` lanes | `+Inf` bound | `77050015` |
| `1.0 / vector::distance(v, w)` | `0.00` | `77050015` |
| `1.0 / vector::dot(v, v)` | `77050015` | `77050015` |
| `1.0 / collections::reduce(...)` (Float `+` callback) | `77050015` | `77050015` |
| `1.0 / math::exp(1000.0)`, `math::pow(1e300, 2.0)` | `77050014` | `77050014` |
| `toFloat("1e400")` | `77050010` | `77050010` |

There is no `product`, `mean` or `average` member. `math::tan` of the nearest
`Float` to π/2 is finite (`1.6e16`).

## Root cause

Two defects:

1. **`collections::sum` left its `Float` total unchecked** (`lower_sum`,
   `src/codegen/builtins/collections/func_sum.rs`), relying on bug-590's re-check at
   the next observation boundary (`FLOAT_RESULTS` row `Unconstrained`). An operand
   is not a boundary, so `1.0 / +Inf` recovered to `0.0`. bug-644 patched the
   inline-`TRAP` path with a check inside the raw capture, which covered only that
   one position.
2. **An inline builtin's `Float` argument was never observed.** `emit_call` observes
   every argument of a user function, runtime helper or `LINK` thunk (spec §4:
   "passed as an argument"), but `lower_abi_inline_args` — the argument path of every
   `Body::AbiInline` member (`math::*`, most `collections::*`) — did not. Seven
   collection lowerings observed their element argument by hand
   (`observe_float_vr`); `math::*` observed nothing, so `math::abs(x * x)` bound
   `+Inf`. `vector::length`/`distance` hit it through the `math::sqrt(...)` call their
   bodies and inline arms build, which is why `FLOAT_RESULTS`' justification for them
   ("the sum is observed as the sqrt argument") was false.

## Fix

- [x] `lower_sum` checks the `Float` total after the fold (`emit_float_result_check`,
  `ErrFloatOverflow`): the elements are finite, so an overflow keeps the total at
  `±Inf` and one check sees it. `lower_abi_inline_args` observes each argument as it
  is lowered, replacing the seven hand-written `observe_float_vr` calls (removed with
  the helper). `sum` is now a `Finite` row for reason 4, the `Unconstrained`
  classification and bug-644's `observe_float_call_result` are gone, and no
  classified member is re-checked at a boundary. Spec §4 and §8 say both forms raise
  at the call. Commit: COMMIT_754
