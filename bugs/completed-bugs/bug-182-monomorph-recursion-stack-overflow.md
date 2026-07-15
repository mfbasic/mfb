# bug-182: monomorph polymorphic recursion overflows the native stack (SIGABRT, no diagnostic)

Last updated: 2026-07-14
Effort: medium (1h–2h)
Severity: HIGH
Class: Security

Status: Fixed
Regression Test: tests/rt-error/monomorph_polymorphic_recursion_depth

A `.mfb` source file whose generic function instantiates itself at an
ever-widening concrete type (`recurse OF T` calling `recurse` with `List OF T`)
drives the monomorph pass into unbounded, distinct-key instantiation. The
compiler aborts with `fatal runtime error: stack overflow, aborting` and emits
**no diagnostic**. The untrusted party is the author of an arbitrary source file
the victim chooses to compile; the impact is a denial-of-service against anyone
building that source. The single correct behavior a fix produces: a bounded,
catchable `TYPE_*` diagnostic ("template instantiation too deep") once an
instantiation depth/budget cap is exceeded, with a clean non-zero exit and no
stack overflow.

This is the still-open audit-1 finding **FE-02**, re-verified against current
code and reproduced. See `planning/audit-2-frontend.md`.

References:

- `planning/audit-2-frontend.md` (FE-02), `planning/old-plans/audit-1-frontend.md`
- Contrast: `src/ir/verify/mod.rs:385` (`MAX_DEPTH = 256`) already caps statement
  nesting, and `src/ast/parser.rs` (`MAX_EXPR_DEPTH = 256`) caps expression
  nesting — monomorph has no equivalent cap and runs *before* `ir::verify`.

## Failing Reproduction

```
mfb init /tmp/fe02proj
cat > /tmp/fe02proj/src/main.mfb <<'EOF'
FUNC recurse OF T(x AS T) AS Integer
  LET y AS List OF T = [x]
  RETURN recurse(y)
END FUNC
FUNC main() AS Integer
  RETURN recurse(1)
END FUNC
EOF
mfb build /tmp/fe02proj
```

- Observed: `thread 'main' has overflowed its stack / fatal runtime error: stack overflow, aborting` (process aborts; no compiler diagnostic).
- Expected: a bounded diagnostic such as `error[TYPE_INSTANTIATION_TOO_DEEP]: template instantiation exceeds the 256 level limit`, non-zero exit, no crash.

Contrast cases that behave correctly today: a finite generic instantiation
(`recurse` bottoming out on a non-recursive type) compiles; and both deep
expression nesting (capped at 256, `MAX_EXPR_DEPTH`) and — where the parser
reaches it — deep statement nesting (`ir::verify` `MAX_DEPTH`) already produce
graceful diagnostics. Only the monomorph instantiation chain is uncapped.

## Root Cause

`src/monomorph/lower.rs:475` `instantiate_function` (dedup at `:528` via
`self.emitted_function_keys.insert(key)`, `key = "{name}<{args}>"`) and the
type-level twin `instantiate_type` (`src/monomorph/lower.rs:639`, dedup at
`:644`). The dedup guard only suppresses *re-emitting an already-seen* concrete
instantiation. Under polymorphic recursion each self-call produces a **distinct**
concrete type argument (`T`, `List OF T`, `List OF List OF T`, …), so every key
is new, the guard never fires, and body lowering re-enters `instantiate_function`
on fresh native frames with no depth or total-count cap. Because monomorph runs
before `ir::verify`, the verifier's `MAX_DEPTH` backstop never executes.

## Goal

- A malicious source instantiating a generic template beyond a fixed depth (or a
  monotonic total-instantiation budget) produces a catchable `TYPE_*` diagnostic
  and a clean non-zero exit — never a stack overflow / SIGABRT.

### Non-goals (must NOT change)

- Do not attempt to distinguish *productive* from *unproductive* polymorphic
  recursion precisely (halting-adjacent); a simple depth/budget cap suffices.
- Do not change the MFBASIC language surface, generics semantics, or any legal
  program's compilation. No legitimate program instantiates a single template
  chain hundreds deep.
- Do not raise the runtime thread stack as the "fix".

## Blast Radius

- `src/monomorph/lower.rs:475` `instantiate_function` — fixed by this bug (the
  primary recursion site).
- `src/monomorph/lower.rs:639` `instantiate_type` — same hazard on the type
  graph; must receive the same cap.
- `src/ir/verify/mod.rs:385` (`MAX_DEPTH`) — correct backstop, but unreachable
  for this input because monomorph crashes first; unchanged.

## Fix Design

Thread an instantiation-depth counter (or a monotonic total-instantiation
budget) through the monomorph context. On entry to `instantiate_function` /
`instantiate_type`, increment; when it exceeds a cap (256, matching
`MAX_EXPR_DEPTH` / `ir::verify` `MAX_DEPTH`), `self.report(...)` a `TYPE_*` error
and return `None` instead of recursing; decrement on exit. A total-instantiation
budget (e.g. a few thousand) additionally bounds fan-out that is wide rather than
deep. Rejected alternative: catching the stack overflow — unsound and
non-portable.

## Phases

### Phase 1 — failing test + audit
- [x] Add the reproduction above as a rt-error test asserting a graceful
      diagnostic (not a crash). Confirm it currently crashes.
- [x] Confirm both `instantiate_function` and `instantiate_type` are on the
      recursion path for the type-widening and (separately) a return-type-widening
      variant. Reproduced independently: the function-widening chain
      (`recurse OF T` calling itself at `List OF T`) is caught in
      `instantiate_function`; a self-widening generic `TYPE Nest OF T` with a
      `child AS Nest OF List OF T` field is caught in `instantiate_type`. Both
      emit `TYPE_INSTANTIATION_TOO_DEEP` and exit 1 with no stack overflow.

### Phase 2 — the fix
- [x] Add the depth/budget counter to the monomorph context; cap and report at
      both instantiation entry points. `template_instantiation_depth` is threaded
      through `Monomorphizer`, incremented on entry to fresh
      `instantiate_function` / `instantiate_type` expansion and decremented on
      exit; at `MAX_TEMPLATE_INSTANTIATION_DEPTH = 256` (matching `MAX_EXPR_DEPTH`
      / `ir::verify` `MAX_DEPTH`) it reports `TYPE_INSTANTIATION_TOO_DEEP` and
      stops recursing.

### Phase 3 — validation
- [x] Full acceptance suite green; the new test passes; no legitimate generic
      program regresses.

## Validation Plan

- Regression test: the depth reproduction, asserting the `TYPE_*` diagnostic.
- Runtime proof: `mfb build` on the repro exits non-zero with the diagnostic and
  no `fatal runtime error`.
- Full suite: `scripts/test-accept.sh`.

## Summary

The risk is purely in choosing a cap that never rejects a legitimate program;
256 (matching existing caps) is safe. The fix is a small counter at two call
sites; no language surface changes.
