# bug-834: inline `TRAP` on `total(x) + 1` is rejected as "not a call" — legality depends on whether the callee's body can fail

Last updated: 2026-10-05
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: tests/rt-behavior/trap/inline-trap-total-call-root-operator-rt (to add)

Take an inline `TRAP` whose scrutinee has a raising operator at its root
(`+ - * / ^ MOD`, unary `-`) and whose only runtime calls go to callees the census proves
total, such as a project `FUNC` that cannot fail. That TRAP is rejected with
`TYPE_INLINE_TRAP_REQUIRES_FALLIBLE`, and the message is wrong: "this expression is not a
call". Two nearly identical programs show the problem:

- `LET c = id(big) + 1 TRAP(e)`, where `id` is `RETURN x`, is **rejected**.
- The same line with an `id` that contains an unreachable `FAIL` **compiles**, and its
  handler catches the `+` overflow (77050010).
- `LET c = id(1) + id(big) * 2 TRAP(e)`, with the total `id`, **compiles** and catches
  the overflow, because the raising `*` is nested rather than at the root.

So whether a call site is legal depends on the callee's implementation and on operator
nesting. The spec says neither. Editing a library function so it can no longer fail
breaks unrelated callers' builds.

The single correct behavior, per spec rule 11: a scrutinee that contains a runtime call
is accepted. The inline TRAP then covers every raising operator in it, the root operator
included. `LET c = id(big) + 1 TRAP(e)` compiles and RECOVERs `77050010`.

References:

- `src/docs/spec/language/08_error-model.md` §8.6 rule 11:
  - "Two rejections: a scrutinee with no runtime call to trap anywhere in it — a
    call-free expression, or a package constant (`TYPE_INLINE_TRAP_REQUIRES_FALLIBLE`)".
  - "the scrutinee must still contain a runtime call".
  - "An expression whose outermost node is not a call but which does contain a fallible
    call is accepted and traps it, operators included".
  - "the handler never fires, exactly as a `TRAP` on an infallible user `FUNC` would
    behave". The `TRAP` on a bare infallible user call `id(1) TRAP(e)` is accepted
    today.
- bugs/completed/bug-779 (call-free chains) and bug-471 (root operator coverage).

## Failing Reproduction

```basic
IMPORT io
FUNC id(x AS Integer) AS Integer
  RETURN x
END FUNC
FUNC main() AS Nothing
  LET big = 9223372036854775807
  LET c = id(big) + 1 TRAP(e)
    RECOVER e.code
  END TRAP
  io::print(toString(c))
END FUNC
```
`mfb build /tmp/hardening/B01/repro/r_trapop`

- Observed: `error[2-203-0069 TYPE_INLINE_TRAP_REQUIRES_FALLIBLE]: inline TRAP requires a fallible call` / `Inline TRAP requires a call to trap; this expression is not a call.`
- Expected: the program builds and prints `77050010`.

Contrasts (they bound the bug):

| Scrutinee | `id` body | Result |
|---|---|---|
| `id(big) + 1` | `RETURN x` | rejected ✗ |
| `id(big) + 1` | contains unreachable `FAIL` | builds, prints 77050010 ✓ (r_trapop2) |
| `id(1) + id(big) * 2` | `RETURN x` | builds, prints 77050010 ✓ (r_trapop3) |
| `id(1)` | `RETURN x` | builds ✓ |
| `-id(1)`, `1 + id(1)` | `RETURN x` | rejected ✗ |

## Root Cause

`src/ir/lower.rs:lower_inline_trap`. When `scrutinee_has_runtime_call(&root)` is true,
it calls `hoist_trap_calls`. That function scans only the root's *operands*
(`scan_trap_operands`) for fallible nodes. A total callee is not fallible, so nothing is
lifted and `hoists` is empty. Then `root_operator_raises` requires
`!hoists.is_empty()`, so the raising root `+` is left unchecked. The bind reaches
`src/ir/verify/resources.rs` (`check_inline_trap_scrutinee`) as a bare non-call bind,
and the verifier reports it as "not a call". bug-779's gate was meant to separate
call-free scrutinees from call-bearing ones. Here, "nothing fallible was lifted" is
being used as a stand-in for "no runtime call".

## Goal

- When the scrutinee contains a runtime call, a raising root operator is checked
  whether or not anything was hoisted. `id(big) + 1 TRAP(e)` RECOVERs 77050010.
- Call-free scrutinees (`1 / z`, `i * 2 + 3`) stay rejected (bug-779 fixture).

### Non-goals

- Do not make a call-free operator scrutinee legal.
- Do not change the dead-handler warning for a bare infallible built-in.

## Blast Radius

- `lower_inline_trap`'s `root_operator_raises` / `check_root` computation: fixed by this
  bug.
- Unary root (`-id(x)`) and binary root on either side: covered by the same predicate.

## Fix Design

Gate `root_operator_raises` on `scrutinee_has_runtime_call(&root)` (already computed)
instead of `!hoists.is_empty()`.

## Validation Plan

- An rt fixture with the five rows above. Run the bug-779 and bug-471 fixtures, then the
  full suite.

## Summary

This is a small predicate change in the inline-TRAP lowering. The risk is golden churn
for call-bearing root-operator scrutinees.
