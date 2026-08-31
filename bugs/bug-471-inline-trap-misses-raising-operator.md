# bug-471: an inline `TRAP` does not cover a raising **operator** inside the trapped expression

Last updated: 2026-08-30
Effort: large (a codegen capability, not a lowering tweak)
Severity: MEDIUM (miscompile class; narrower shape than bug-457)
Class: Miscompile (error handling silently defeated; no diagnostic)

Status: FIXED (see the STATUS block at the bottom)
Regression Test: `tests/rt_inline_trap_raising_operator.rs` (20 cases). **12
measured RED** against this tree before the fix — division by zero, integer
multiply overflow, `MOD` by zero, `Float` overflow, unary-negation overflow, an
operator inside a fallible call, `RECOVER` skipping the rest, left-to-right
order, first-failure-wins, an `Assign` target, an error-ignoring handler, and the
short-circuit rejection — and **5 controls** green throughout (a successful
operator, a preceding call failure, a bare-operator scrutinee still rejected,
bug-457's nested-call shape, and an operator outside any trap still propagating).
Of the remaining three: one pins the negative-literal carve-out by IR shape, and
two pin the fallibility-oracle defect below — both measured RED here and one of
them **on the merge-base compiler**, which dates that defect before this fix.

An inline `TRAP` covers every fallible **call** in the trapped expression
(bug-457, fixed in `daa6c8d35`). It does **not** cover an *operator* in that same
expression that raises — a division by zero, an arithmetic overflow. The error
propagates past the handler to the function-level trap exactly as a nested call
used to, with no diagnostic.

This is the same escape class as bug-457 but a different mechanism, and it was
deliberately left out of that fix rather than bolted on — see "Why bug-457's fix
does not reach it".

## Failing reproduction

`/tmp/b457op`, macOS aarch64, built with the bug-457 fix in place
(`daa6c8d35`), so this is *not* a shape that fix was expected to cover:

```basic
IMPORT io

FUNC two(a AS Integer, b AS Integer) AS Integer
  RETURN a * 100 + b
END FUNC

FUNC main AS Integer
  MUT z AS Integer = 0
  LET d = two(1 / z, 2) TRAP(e)
    io::print("caught operator code=" & toString(e.code))
    RECOVER -1
  END TRAP
  io::print("d=" & toString(d))
  RETURN 0
END FUNC
```

Observed:

```
Error: 7-705-0002
Argument value is not valid for the requested operation.
[exit 255]
```

Expected: `caught operator code=77050002` (the code as `toString`ed — the same
value the root-position case below prints) then `d=-1`, exit 0.

`z` is a `MUT` local rather than a literal `0` so the division is not
constant-folded at compile time.

## The error IS trappable — the escape is positional

The same division caught at the **root** of the trapped expression works today,
which is what makes this a miscompile rather than an unsupported feature:

```basic
FUNC divz(a AS Integer, b AS Integer) AS Integer
  RETURN a / b
END FUNC
...
LET d = divz(1, 0) TRAP(e)
  io::print("caught div code=" & toString(e.code))
  RECOVER -1
END TRAP
```

prints `caught div code=77050002` and `d=-1`, exit 0. The error escapes `divz`
through the ordinary call boundary and is captured as the root call's `Result`.
Only an operator raising *inside the trapped expression itself* is missed.

## Why this is a bug and not the documented behaviour

`mfb spec language error-model` §8.4 scopes an inline `TRAP` to the whole
expression, and bug-457's fix made that literal for calls: "an inline `TRAP` …
covers **every** fallible call inside that expression". An error raised while
evaluating that expression must reach the handler regardless of which node
raised it. Nothing in §8.4 or §8.8 distinguishes a call from an operator here.

## Why bug-457's fix does not reach it

Two independent reasons, both structural:

* **The capture is per-value, not per-region.** Codegen's redirect is
  `self.raw_result_capture`, set around the lowering of *one* value
  (`lower_inline_conversion_raw` / the plan-21-B member generalization in
  `builder_values.rs`), and consumed by `emit_error_register_return`
  (`error/emission/builder_error_emission.rs`). It converts that value's
  domain-error exit into a branch to a capture label. There is no notion of "every
  op in this trap region redirects here".
* **bug-457's desugar spreads the expression across statements.** The fix lifts
  each nested fallible call into its own `Bind $trap_argN` / `Bind $trap_resN` op
  ahead of the residual expression (`ir::lower::lower_inline_trap`). An operator
  that raises can therefore sit in a plain `Bind` op that is not under any
  capture at all. Extending the existing per-value capture would miss exactly
  those.

So the fix is not "widen `raw_result_capture` a bit". It needs a trap-**region**
concept in codegen — every error exit emitted while lowering the ops of one
inline-`TRAP` region routes to that region's capture — which the structured,
jump-free IR (`mfb spec architecture ir`) deliberately does not have today.

## What a fix must produce

Every error raised while evaluating the trapped expression — from a call, an
operator, or a conversion — runs the handler, and a `RECOVER` skips the
remainder of the expression, matching bug-457's guarantee for calls.

Sketch of the two plausible shapes, neither costed:

1. **A region-scoped capture.** Give the lowered inline-`TRAP` region a codegen
   label and have `emit_error_register_return` prefer it over
   `error_exit_destination()` for every op inside the region, joining bug-457's
   shared `$trap_failed`/`$trap_err` flag on the way. Needs the region boundary
   to survive from `ir::lower` into codegen, which is new IR surface.
2. **Lift raising operators the way calls are lifted.** Give arithmetic a
   `Result`-producing IR form so `ir::lower` can hoist `a / b` into a checked
   `Bind` exactly as it hoists a fallible call. Conceptually the cleanest and
   reuses bug-457's whole desugar, but there is no `BinaryResult` node today and
   every raising operator would need a raw lowering.

An acceptable interim, as with bug-457, is a **diagnostic**: reject an inline
`TRAP` whose expression contains a raising operator that is not itself the
trapped root, telling the author to bind the arithmetic first. That converts a
silent miscompile into a compile error. It must not ship as the final answer.

## Blast radius

Any `LET x = f(… a / b …) TRAP … END TRAP` where the arithmetic can raise.
Division by zero is the one confirmed instance (`7-705-0002`, measured above);
any other operator that raises through the same `emit_error_register_return`
seam is in scope by construction, but none was separately measured. Narrower
than bug-457's
(nested calls are far more common than nested raising arithmetic), and no
instance was found in `tests/`, `examples/` or `src/` by the bug-457 census, so
this is a latent shape rather than one currently breaking a fixture.

## Ordering against bug-467

bug-467 (a socket write to a closed peer kills the process with `SIGPIPE`, exit
141) is the same family — a handler the author wrote correctly never runs — but
its signal fires *inside the syscall*, before any IR-level machinery can see an
error at all. So a trap-region design landed here first would appear to fail on
socket writes when it had not: no amount of region tracking helps until the
signal is suppressed. Fix 467 before attempting either shape above, and use its
repro as an adversarial input for whichever is chosen.

References: `src/ir/lower.rs:lower_inline_trap` (bug-457's desugar);
`src/codegen/engine/value/builder_values.rs` (`raw_result_capture` setup);
`src/codegen/error/emission/builder_error_emission.rs`
(`emit_error_register_return`, where the capture is consumed);
`src/docs/spec/language/08_error-model.md` §8.4, §8.8;
`bugs/completed/bug-457-inline-trap-misses-nested-fallible-call.md`.

## STATUS: FIXED

Reproduced first, exactly as documented: `two(1 / z, 2) TRAP(e)` let `7-705-0002`
past the handler and exited 255 on macOS aarch64. The mechanism was confirmed by
dumping the `-ir`, not inferred: the division survived as a plain `binary` node
inside the `callResult`'s args, so it was lowered with no capture active and
`emit_error_register_return` took its `error_exit_destination()` branch.

Five more operator shapes were measured escaping the same way before touching
anything — integer `*` overflow (`7-705-0010`), `MOD` by zero, `Float` `*`
overflow to infinity (`7-705-0015`), unary `-` of `i64::MIN`, and a division
inside a *fallible* call's argument — so this was never only about `/`.

**The fix takes shape 2 of the two this doc sketched**, not shape 1. The doc
argued a region-scoped capture "needs the region boundary to survive from
`ir::lower` into codegen, which is new IR surface", and that lifting operators
"reuses bug-457's whole desugar" but has "no `BinaryResult` node today". The
second is the smaller change once you see that codegen *already* has the region
primitive: `raw_result_capture` is a redirect around an arbitrary stretch of
lowering, and `lower_inline_conversion_raw` / `lower_inline_builtin_raw` merely
happen to scope it to one built-in. So no per-operator "raw lowering" was needed,
and no `BinaryResult`:

* **`IrValue::Checked { type_, value }`** — "evaluate `value` with its
  domain-error exits captured, yielding `Result OF type_`". One node covers every
  raising operator rather than one `*Result` node per operator kind, and it
  extends to any future non-call raise site for free.
* **`lower_inline_trap` lifts a raising operator exactly as bug-457 lifts a
  call**, in the same evaluation order, into the same shared
  `$trap_failed`/`$trap_err` chain — `Bind $trap_resN = Checked(a / b)` +
  `If ResultIsOk`. The handler stays emitted once.
* **`lower_checked_value`** runs the inner lowering under `raw_result_capture`,
  tags the fall-through `Ok`, and materializes the `Result`.

Three things the sketch did not anticipate:

* **`Checked` has to be the observation boundary for a `Float`.** plan-17 moved
  `+`/`-`/`*`/`/`'s finiteness check off the operator and onto whatever first
  consumes the value. Lifting the operator moved it under a `ResultValue`, which
  is not an arithmetic node, so *nothing* observed it: the first version returned
  `d=2.00` for a `1.0e308 * 1.0e308` that had previously raised — a new silent
  miscompile introduced by the fix, caught by the probe rather than by reading.
  `lower_checked_value` calls `observe_float` inside the capture.
* **A `Checked` must not wrap a call.** A callee's error return does not pass
  through `emit_error_register_return` in this frame, so it would auto-propagate
  past the very handler the wrapper exists to feed. The desugar lifts calls out
  first (so the shape cannot arise from source) and
  `ir::verify::check_checked_has_no_call` rejects it on the decoded-package path.
* **A negative literal is not a computed negation.** `Unary(-, Const n)` is the
  parser's spelling for `-1`, and it was the *only* thing that appeared in all 8
  `.ir` golden diffs the first version produced — every one of them a whole
  `Result` materialization to check a negation that provably succeeds.
  `fallible::is_total_literal_negation` exempts it (excluding `Byte`, whose
  negation raises `ErrUnderflow` for any non-zero operand; the `i64::MIN`
  spelling is safe because lowering folds `-9223372036854775808` to a single
  `Const`, measured by dumping its `-ir`). With the carve-out the change is
  **byte-identical over the whole committed corpus** — `artifact-gate all`: 1299
  tests, 1456 builds, 1786 goldens, **0 diffs** — so no golden was regenerated
  and every shift the first version caused is accounted for.

**The oracle is deliberately coarse.** `fallible::operator_can_raise` answers
from the operator spelling (`+ - * / DIV MOD ^`, unary `-`) and the node's own
result type (`Byte`/`Integer`/`Fixed`/`Money`/`Float`), not arm-by-arm against
`codegen::engine::operators`. Money's dispatcher, Byte's underflow and Fixed's
`MOD` divisor check are already three separate paths there; a recogniser kept in
lockstep with a growing census is what silently loses an arm. Over-approximating
costs one always-`Ok` check inside a trapped expression, under-approximating is
the miscompile.

**Two more bugs fell out, both pre-existing.**

*Evaluation order.* `two(1 / z, inner(-1))` reported `inner`'s `9-000-0001`, not
the division's `7-705-0002`: bug-457's lift moved the fallible call ahead of an
operator that preceded it in source order, violating left-to-right evaluation
(`mfb spec language error-model` §8.1). Lifting the operator too restores it;
pinned by `the_first_failure_in_evaluation_order_wins`.

*The fallibility oracle never looked at operators.* `fallible::analyze` marked a
function fallible only if its body could `FAIL`, `PROPAGATE`, or call something
fallible — so `FUNC fltDiv(a AS Float, b AS Float) AS Float / RETURN a / b` was
recorded **infallible** despite raising `ErrFloatOverflow` on every zero divisor.
`check_root` consults that verdict whenever anything in the trapped expression
was lifted, so the ROOT call was left unchecked and its real error propagated
past the handler.

This is **not** collateral from this fix; it predates it. Measured on a release
compiler built from the merge-base (`5815262c4`) with bug-457's lift triggered by
a fallible call rather than an operator:

```basic
LET y = fltDiv(toFloat(inner(1)), 0.0) TRAP(e)   ' merge-base: exit 255,
  io::print("caught=" & toString(e.code))        ' uncaught 7-705-0015
  RECOVER 0.0
END TRAP
```

bug-471 only widened its *reach* — a lifted operator became another way to fill
`hoists` — which is how it surfaced: `tests/acceptance` went 730/732, failing
exactly the two float cases whose arguments spell `0.0 - 1.0`
(`arithmetic.mfb:329`, `:337`). The fix teaches `expression_escapes` that an
arithmetic operator escapes, judged by spelling alone (the walk has no types, and
arithmetic only type-checks on numeric operands, so the spelling implies the
type), with the same negative-literal exemption so `RETURN -1` does not make a
function fallible. Pinned by
`a_callee_that_raises_only_through_an_operator_is_still_checked` and
`bug457s_lift_also_keeps_an_operator_raising_root_checked`.

The lesson generalizes past this bug: `fallible.rs`'s own header promises a
"safe over-approximation … under-approximating would silently drop the error on
the floor", and it was under-approximating for an entire category of raise site
because the analysis was written when only calls could reach a handler.

**Deliberately NOT changed: what may *be* a scrutinee.** `LET d = 1 / z TRAP(e)`
is still `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE` — a compile error, not a silent
escape, so it is outside this bug's class. This fix widens what an inline `TRAP`
*covers*, not what it may be attached to; making a bare operator a legal
scrutinee is a language-surface change that would also make `LET x = 1 + 2 TRAP`
legal, and the existing `REQUIRES_FALLIBLE` fixtures protect that boundary.
Pinned by `a_bare_operator_scrutinee_is_still_rejected`.

**The short-circuit corner is closed the way bug-457 closed it.** A raising
operator in an `AND`/`OR` right operand cannot be lifted — hoisting evaluates it
unconditionally — so `ir::shape` reports `TYPE_INLINE_TRAP_SHORT_CIRCUIT_CALL`
rather than letting it escape. The rule *name* keeps bug-457's `_CALL` suffix on
purpose (renaming a shipped rule breaks every filter keyed on it); only the
message widened.

**On the bug-467 ordering.** This doc asked for bug-467 (SIGPIPE) to be fixed
first, on the grounds that a trap-region design would appear to fail on socket
writes when it had not. That ordering guards a *shape-1* design whose correctness
claim is "every op in the region routes here"; the shape-2 fix that landed makes
no such claim — it covers exactly the operator nodes it lifts, and a signal
raised inside a syscall is visibly outside that set. bug-467 remains open and
unaffected; nothing here makes it harder, and its repro is not a valid
counter-example to this fix.

### Verification

* `tests/rt_inline_trap_raising_operator.rs` — 18/18 (12 measured RED first).
* `tests/rt_inline_trap_nested_call.rs` — 20/20, bug-457's suite unchanged.
* `artifact-gate all` — 1299 tests, 1456 builds, 1786 goldens, 0 diffs.
* The `.mfp` round trip, by hand: a `kind: "package"` project whose exported
  `safeDiv` carries a `Checked` node builds, and an executable importing it
  prints `ok=5` / `div0=-1` — so value tag 22 encodes, decodes, passes
  `verify_package`, and lowers on the consumer side.
