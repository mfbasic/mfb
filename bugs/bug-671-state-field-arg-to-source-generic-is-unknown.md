# bug-671: a `RES … STATE` payload field passed to a source-generic `collections` member does not type-check

Last updated: 2026-09-21
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: (to add) `tests/rt-behavior/resources/state-field-source-generic-arg-valid`

`collections::distinct(h.state.xs)`, where `h` is a `RES … STATE Cur` handle and
`xs` is a `List OF Integer` field of `Cur`, is rejected at compile time:

```
error[2-203-0021 TYPE_CALL_ARGUMENT_MISMATCH]: function call argument type does not match parameter type
               Call to `__collections_distinct` cannot infer template arguments from `Unknown`.
```

The same happens for every `collections` member whose body is MFBASIC source (a
source generic, `__collections_X OF T`): `distinct`, `take`, `drop`, `sort`,
`sortBy`, `union`, `intersection`, `difference`, `symmetricDifference`, `merge`
and `mapValues`. It happens whether the handle is an owner local or a `RES`
parameter. The same call on a record local's field (`distinct(r.xs)`) or on a
plain local compiles, and so does an inline-lowered member on the same `STATE`
field (`collections::filter(h.state.xs, p)`, `append`, …).

**The correct behavior:** `h.state.f` has the type of the `STATE` record's field
`f` wherever an expression's type is needed, so every call that type-checks with
`r.f` for a record `r` of the same type also type-checks with `h.state.f`.

References:

- Found by plan-144-B (the `STATE` self-update audit), whose probe
  `/tmp/plan-144-probes/state` could not compile 11 rows × 8 `STATE` sites (88
  functions). They are recorded as `n/a (does not compile: bug-671)` in
  `planning/plan-144-findings/record-state-self-update-audit.md` §2.
- `mfb man variable` §"A handle can carry its own data: STATE": `h.state.f` reads a
  field of the payload record.
- plan-106-C (`ParameterType::state`), which taught the IR's `expression_type` the
  `.state` member (`src/ir/lower.rs:3894-3898`).

## Failing Reproduction

`/tmp/plan-144-probes/stgen/src/main.mfb` (with a `project.json` of
`kind: executable`, `entry: main`, `targets: [native]`):

```basic
IMPORT collections
IMPORT fs
IMPORT io

TYPE Cur
  xs AS List OF Integer
END TYPE

SUB main()
  RES h AS fs::File STATE Cur = fs::openFile("/tmp/plan-144-probes/project.json")
  LET ys AS List OF Integer = collections::distinct(h.state.xs)
  io::print(toString(len(ys)))
  LET zs AS List OF Integer = collections::filter(h.state.xs, isPos)
  io::print(toString(len(zs)))
END SUB

FUNC isPos(v AS Integer) AS Boolean
  RETURN v > 0
END FUNC
```

`target/release/mfb build /tmp/plan-144-probes/stgen` at `efdb54bb7`:

- Observed: `main.mfb:11 error[2-203-0021 TYPE_CALL_ARGUMENT_MISMATCH]` "Call to
  `__collections_distinct` cannot infer template arguments from `Unknown`", and exit 1.
- Expected: `Wrote executable`.

Contrast cases that work today (and become regression guards):

- `collections::filter(h.state.xs, isPos)` (line 13): no diagnostic. `filter` is
  inline-lowered and has no template.
- `collections::distinct(r.xs)` with `MUT r AS Cur`: compiles (plan-144-A's record
  probe, all 11 source generics at every record site).
- `h.state.xs = collections::append(h.state.xs, 1)`: compiles and is lowered in place
  (plan-144-B, Layer 2).

## Root Cause

The monomorphization pass types the argument, not the IR, and its type-of-expression
helper cannot see a `STATE` payload.

1. `src/monomorph/lower.rs` `Monomorphizer::lower_expression`, the call arm
   (`:1498-1504`), types each argument with
   `self.expression_type(arg, context).unwrap_or(ParameterType::Unknown)`. Then
   `collections_internal_callee` (`:219`, applied at `:1523-1525`) rewrites
   `collections.distinct` to the source generic `__collections_distinct`, and
   `instantiate_function` (`:796`) unifies each parameter pattern with the argument
   type (`unify_type`, `src/monomorph/helpers.rs:176`). `List OF T` against
   `Unknown` fails (`helpers.rs:229-234`), and the error is raised at `lower.rs:843-849`.
2. `expression_type` (`lower.rs:2065`) returns `None` for `h.state.xs`, for two
   reasons that compound:
   - **The local's `STATE` is dropped.** The `Let` arm (`lower.rs:1139-1166`) lowers
     `state_type` but inserts only the base type into `context.locals` (`:1165`),
     so `h` is recorded as `fs::File`, not `Stateful { fs::File, Cur }`. Parameters
     do the same (`lower_function_inner`, `:699-706`, ignores `param.state_type`).
   - **There is no `.state` member case.** The `MemberAccess` arm
     (`lower.rs:2122-2128`) only looks the member up in `record_fields(target_type)`.
     `fs::File` is not a record, so `h.state` is `None`, and `h.state.xs` is `None`.
3. Why the contrast cases are immune. A record local's type *is* the record, so
   `record_fields(Cur)` finds `xs`. An inline-lowered member has no template, so
   `instantiate_function` returns early (`:802`), and the overload resolvers treat
   `Unknown` as a wildcard. The IR's own `expression_type` (`src/ir/lower.rs:3822`)
   has both pieces: locals carry `with_state` (`ir/lower.rs:653, 739, 1074, 1109`),
   and it has the `member == "state"` arm (`:3894-3898`). So everything after
   monomorphization types the expression correctly.

## Goal

- The reproduction builds (`Wrote executable`) and prints the deduplicated length.
  All 11 source-generic members accept an `h.state.f` argument, both on an owner
  handle and on a `RES` parameter.

### Non-goals (must NOT change)

- The `STATE` rules themselves (`TYPE_STATE_INVALID` for a non-copyable payload such
  as `json::Json`, and the `.state` assignment desugar).
- What `collections_internal_callee` routes to a source generic, and the generic
  bodies.
- **Tempting wrong fix:** making `unify_type` accept `Unknown` as a wildcard for a
  template parameter. That would instantiate `__collections_distinct$Unknown`, or
  bind `T` wrongly, and hide every other place `expression_type` loses a type.
  Forbidden. The type must be computed, not guessed.

## Blast Radius

Every caller of `Monomorphizer::expression_type` (`src/monomorph/lower.rs`), found by
`grep -n 'expression_type(' src/monomorph/lower.rs`. With a `.state.f` value:

- `:1501` call-argument types — **fixed by this bug** (the reported error for source
  generics). For a builtin or imported overload, `Unknown` matches any parameter,
  so resolution still succeeds today. It is fixed by the same change.
- `:1162` an inferred `LET x = h.state.xs` (no `AS`): `x` is never added to
  `locals`, so a later `distinct(x)` fails the same way. **Fixed by this bug** (same
  helper); add a test.
- `:1391` the element type of `FOR EACH v IN h.state.xs`: `v` is not recorded, so
  a generic call on `v` fails. **Fixed by this bug**; add a test.
- `:1357/1358/1362` `FOR` bounds from `h.state.n`, `:1625` generic record constructor
  inference, `:2049` `builtin_call_return_type` arguments, and the recursive uses
  inside `expression_type`: they lose type information the same way. They are
  fixed by the same change, and the Phase 1 audit writes a verdict for each.
- `:1235` the `StateAssign` arm passes the local's type (`fs::File`) as the
  expected type of `h.state = …`, so return-type overload selection there sees the
  wrong expected type. Latent, and fixed by (a) below plus `target.state()`.
- `helpers.rs:673` `function_signature_types` ignores `return_state_type`, so
  `f().state.xs` for a user `FUNC f() AS RES … STATE S` has the same gap. Latent,
  and in scope if Phase 1 reproduces it.

## Fix Design

Both halves are needed; either alone leaves `h.state` untyped:

- (a) Record the state in `locals`. In the `Let` arm, insert
  `lowered_type.with_state(&state)` when there is a lowered state (next to
  `lower.rs:1158-1165`). Do the same for parameters in `lower_function_inner`
  (`:699-706`). Both mirror `src/ir/lower.rs:1074/1109`.
- (b) Add the `.state` arm to `expression_type`'s `MemberAccess` case
  (`lower.rs:2122`): `if member == "state" { if let Some(s) = target_type.state() {
  return Some(s) } }`, mirroring `src/ir/lower.rs:3894-3898`.
- Then check every reader of `context.locals` that may now see a `Stateful`
  instead of the bare base. `types_compatible` already handles `Stateful`
  (`lower.rs:313, 334-338`). `StateAssign`'s expected type becomes
  `target.state()`.

Rejected: typing `h.state.f` only at the call site (`:1501`). It would leave `:1162`,
`:1391` and the other callers broken.

No generated-output shift is expected for programs that compile today. Every such
program's `h.state.f` was either not passed to a source generic or not typed by
this helper.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Add `tests/rt-behavior/resources/state-field-source-generic-arg-valid`: the
      reproduction, extended to all 11 source generics on an owner handle and on a
      `RES` parameter, plus an inferred `LET` and a `FOR EACH v IN h.state.xs` with a
      generic call on `v`. Confirm it fails with the documented diagnostic.
      RED: `mfb build` at `a09e3d88d` → 24× `TYPE_CALL_ARGUMENT_MISMATCH` (lines
      53–63 param, 75–85 owner, 92 inferred LET, 96 FOR EACH), exit 1; and
      `test-accept.sh <main's mfb> … state-field-source-generic-arg-valid` →
      "3 mismatch(es)". The `f().state.xs` case (below) was added after it
      reproduced the same diagnostic.
- [x] Complete the blast-radius audit: a verdict per `expression_type` caller.
      - `:1501` call arguments, `:1162` inferred LET, `:1391` FOR EACH element:
        reproduced and fixed (fixture lines `owner/param`, `let`, `each`).
      - `:1357/1358/1362` FOR bounds, `:1625` generic constructor inference,
        `:2049` builtin return-type args, recursive uses: all read the same
        `MemberAccess` arm, so they now get the field type; no separate code.
      - `:1235` `StateAssign` expected type: now `locals[h].state()`.
      - `helpers.rs` `function_signature_types`: reproduced (`len(distinct(opened().state.xs))`
        → the same diagnostic) and fixed; fixture line `return`.

Acceptance: the new test fails for the documented reason; every audit site has a
verdict.
Commit: —

### Phase 2 — the fix

- [x] (a) and (b) in `src/monomorph/lower.rs` (and `function_signature_types` if
      Phase 1 reproduces the return-state gap). All three landed; the fixture
      builds and prints the expected values (`test-accept.sh target/debug/mfb …
      state-field-source-generic-arg-valid` → passed).
- [x] `StateAssign`'s expected type from `target.state()`.

Acceptance: the Phase 1 test passes; the contrast cases still compile; nothing in
Non-goals changed.
Commit: —

### Phase 3 — full validation

- [ ] Run the full suite (`./scripts/test-accept.sh`, `./scripts/artifact-gate.sh
      ./target/release/mfb all`, `cargo test --bin mfb`); no golden should move.
- [ ] Re-run the reproduction, and plan-144-B's `STATE` probe with
      `exclude.txt` emptied of its 88 bug-671 entries: 0 diagnostics.

Acceptance: the full suite is green with no golden delta, and the reproduction
builds.
Commit: —

## Validation Plan

- Regression test: `tests/rt-behavior/resources/state-field-source-generic-arg-valid`.
- Runtime proof: run the reproduction; it prints the deduplicated and filtered
  lengths.
- Doc sync: none expected (the language already allows this; only the compiler
  was wrong).
- Full suite: as Phase 3.

## Open Decisions

- Fix `function_signature_types`'s return state here, or separately. Recommended:
  here if Phase 1 reproduces it, because it is the same gap in the same helper.

## Summary

The fix is two small edits in the monomorphizer, copying what the IR already does.
The risk is in the readers of `context.locals` that start seeing a `Stateful` type,
which the Phase 1 audit enumerates. Language rules and the generic bodies are left
untouched.
