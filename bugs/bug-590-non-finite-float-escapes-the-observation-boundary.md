# bug-590: a non-finite `Float` returned by a builtin call escapes every observation boundary

Last updated: 2026-09-12
Effort: unknown — the predicate is one line; the audit of which call results
must be re-observed is the work
Severity: HIGH — breaks a documented language guarantee, and the observable
symptom is silently wrong output rather than an error
Class: Correctness / float observation boundary (plan-17)

Status: Fixed (2026-09-12)
Regression Test: `tests/rt-error/collections/sum_float_overflow_rt` (the filed
repro) and `tests/rt-error/vector/dot_float_overflow_inline_rt` (a SECOND
instance found while auditing the blast radius), plus the positive pin
`tests/rt-behavior/arithmetic/float-call-boundary-finite-rt` and the unit
census `codegen::builtins::float_result::tests::float_results_is_total`.

Found while auditing `collections`' declared errors for bug-563. **Reproduced
before filing**, on the release compiler at `ceeefcf24`.

## The guarantee being broken

`mfb spec language types` (`src/docs/spec/language/04_types.md:107`):

> MFBASIC guarantees that **no user-accessible `Float` is non-finite**, enforced
> at *observation boundaries* rather than after each operation: a finiteness
> check fires only where a `Float` becomes observable — **bound to a named
> local/global**, assigned, stored into a collection element or record field,
> **returned**, passed as an argument, or **printed/converted**. … At a boundary
> a `NaN` fails with `ErrFloatNaN` (`77050013`) and an infinity fails with
> `ErrFloatOverflow` (`77050015`).

## Reproduction (measured, not reasoned)

```basic
IMPORT io
IMPORT collections

SUB main()
  MUT xs AS List OF Float = []
  xs = collections::append(xs, 1.0e308)
  xs = collections::append(xs, 1.0e308)
  xs = collections::append(xs, 1.0e308)
  LET total AS Float = collections::sum(xs)     ' boundary 1: bound to a named local
  io::print("sum3=" & toString(total))          ' boundary 2: printed/converted
END SUB
```

Observed — **no raise**, exit 0, and this printed:

```
sum3=17976931348623159077293051907890247336179769789423065727343008115773267580550096313270847732240753602112011387987139335765878976881441662249284743063947412437776789342486548527630221960124609411945308295208500576883815068234246288147391311054082723716335051068458629823994724593847971630483535632962422413721
6.00
```

That is the decimal expansion of `f64::MAX`. **The value is not `f64::MAX` — it
is `Inf`**, and `toString` renders it as MAX's digits.

Proof it is `Inf`, using the boundary that DOES work as the instrument:

```basic
  LET d AS Float = total - total
```

raises `Error: 7-705-0013 Floating-point operation produced a NaN result.`
(exit 255). `Inf - Inf` is `NaN`; `MAX - MAX` is `0.0`. So `total` is infinite,
it was bound to a named local, and it was printed — three boundaries the spec
names, none of which fired.

## Root Cause

`float_arith_node` (`src/codegen/builtins/math/gen_math.rs`) decides what a
boundary must re-check:

```rust
pub(crate) fn float_arith_node(value: &NirValue) -> bool {
    matches!(value, NirValue::Binary { .. } | NirValue::Unary { .. })
}
```

Its comment states the reasoning: "Only the float arithmetic operators produce
`NaN`/`Inf` … every other node is finite by construction (plan-17)."

**A builtin `Call` is neither `Binary` nor `Unary`, and the premise is false for
it.** `collections::sum`'s `Float` arm (`func_sum.rs:218-228`) accumulates with a
plain `abi::float_add_d` — no check, no raise — so the call result can be
non-finite while the boundary declines to look at it.

That premise held while every float-producing builtin either checked internally
or could not overflow. `sum` does neither.

## Blast radius — this is the part to get right

**Do not fix only `sum`.** The defect is in the predicate, so the question is
which builtin `Call` results can be non-finite and are therefore currently
unchecked at their boundaries. Enumerate them and assert the enumeration is
TOTAL — a wildcard-free `match`, so a new float-returning builtin is a build
error rather than a silent hole. Obvious neighbours: any reducer or accumulator
over `Float` (`sum`, and check whether `math::` kernels, `average`-shaped
members, or collection folds have the same shape).

Note `math::` kernels already raise `ErrFloatInf` themselves on a genuine domain
overflow (per the same spec paragraph), so the fix must not double-check them
into raising a *different* error code than the spec assigns.

## Secondary defect, same repro — `toString` of a non-finite

`toString(Inf)` printed `f64::MAX`'s digits instead of raising. Whether that is
a separate bug or falls out of the boundary fix depends on where the conversion
re-checks; the spec calls "printed/converted" a boundary, so a correct boundary
should raise before `toString` ever formats. **But a formatter that renders a
non-finite as a finite-looking number is its own hazard** — if the boundary fix
does not make this unreachable, it needs closing separately, because silent
wrong output is worse than the leak of an `Inf`.

## Goal

- A non-finite `Float` produced by a builtin call raises `ErrFloatOverflow` (or
  `ErrFloatNaN`) at the first boundary it reaches, per the spec.

### Non-goals (must NOT change)

- Do not re-check anonymous intermediates. The spec deliberately allows a
  transient non-finite that recovers (`1.0 / (1e200 * 1e200)` -> `+0.0`).
- Do not break FMA contraction, which is semantics here and not an optimization:
  `LET r AS Float = a * 2.0 - a` must still yield a finite result.
- Do not change which error code a `math::` kernel raises for its own domain
  overflow.

## Gate (required)

1. the RED fixture raises instead of printing digits;
2. cite the spec paragraph above as the contract the fix realizes, and show the
   fix only ADDS a check;
3. artifact-gate delta confined to fixtures that call a float-returning builtin,
   everything else byte-identical; report exact counts;
4. **POSITIVE pins**: an ordinary finite `Float` program is unchanged; the
   spec's own transient-recovery example still does NOT trap; and an FMA-
   contracted chain still yields its finite result. A boundary that fires too
   eagerly breaks documented, tested behaviour.

## Resolution (2026-09-12)

### What was wrong

`float_arith_node` answered `true` for `Binary`/`Unary` only. It is now a
**wildcard-free** `match` over every `NirValue` variant (a new variant is a build
error), and the `Call`/`CallResult`/`RuntimeCall` arm asks
`builtins::float_result::builtin_float_result_may_be_nonfinite`.

### The enumeration, and why it is TOTAL

The set of `Float`-returning builtins cannot be found by grep:
`return_type: ParameterType::Float` matches **4** members, because every `math::`
kernel declares `ParameterType::Arg(0)` ("echo the operand's type") instead. A
registry walk that resolves `Arg(n)` against the overload's parameters reports the
real population: **43 implementations across 31 members** —

| package | members | verdict |
|---|---|---|
| `collections` | `sum` | **Unconstrained** |
| `collections` | `get`, `getOr`, `reduce`, `reduceRight` | Finite |
| `color` | `luminance`, `contrastRatio` | Finite |
| `general` | `toFloat` | Finite |
| `math` | `abs acos asin atan atan2 clamp cos exp log log10 max min pow sin sqrt tan` | Finite |
| `thread` | `accept`, `receive`, `waitFor` | Finite |
| `vector` | `angle`, `distance`, `dot`, `length` | Finite |

`src/codegen/builtins/float_result.rs` carries that table with a per-row reason,
and `float_results_is_total` re-walks the registry and fails if a member is
missing or a row is stale — so a new `Float`-returning builtin is a red test, not
a silent hole. The lookup additionally fails CLOSED (an unclassified registry
member that can return a `Float` is re-checked).

### The second instance

`vector::dot` leaked an `Inf` too, by a different route: `try_inline_vector_op`
(plan-01-vector) replaces the `__vector_dot_*` FUNC with its expression tree at
the call site, and with it the `RETURN` that observed the sum. `length`/`distance`
were safe by accident — they hand their sum to `math::sqrt` as an *argument*,
which is a boundary in its own right. The `dot` arm now observes its own sum, so
the inline and out-of-line paths raise identically.

### The recogniser, too

`module_analysis::value_may_emit_float_arithmetic_error` decides whether a module
emits the float error message strings. It is the *measurer* for the same
behaviour and had the same blind spot — the first build after the emitter change
failed with `native code string literal 'Floating-point arithmetic overflowed to
infinity.' has no data object`. Its `Call` arm now consults the same predicate,
gated on the call's static `Float` type.

### `toString(Inf)` — verdict

**Made unreachable by this fix on every path measured; NOT closed separately.**
`toString`'s own boundary observes its argument node, so
`toString(collections::sum(xs))` raises at the argument and
`LET t = collections::sum(xs)` raises at the bind — `toString` is never reached
with a non-finite. The formatter's fallback (rendering `Inf` as `f64::MAX`'s
digits) is still latent and would resurface behind any *future* unobserved
producer; the `float_results_is_total` census is what prevents that producer from
existing.

### Found but NOT fixed here — `collections::sum`'s Float overload declares the wrong error

`src/codegen/builtins/collections/func_sum.rs` declares `errors:
vec!["ErrOverflow"]` on **all three** overloads. The `Float` arm cannot raise
`ErrOverflow` (`77050010`) — it has no check at all; the value now raises
`ErrFloatOverflow` (`77050015`) / `ErrFloatNaN` (`77050013`) at the caller's
boundary. This is the row bug-563 archived as "set/sum blocked on the
float-observation question".

It is left open deliberately: whether a *boundary* raise belongs in the callee's
declared-error list is a design question, not a mechanical correction. The spec
attributes the raise to the statement's location, not to the call, and the
`errors` list also drives the inline-`TRAP` fallibility census
(`native_member_declares_error`), so changing it moves more than a man page.
Decide it, then it is a one-line edit plus a golden refresh.
