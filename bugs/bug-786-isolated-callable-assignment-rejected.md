# bug-786: IR verification rejects isolated callables in ordinary callable slots

Last updated: 2026-10-01
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness
Status: Open
Regression Test: proposed `tests/syntax/functions/isolated-callable-assignment-valid` and matching runtime fixture; not added in this documentation-only task.

An isolated `FUNC(Integer) AS Integer` cannot initialize an ordinary
`FUNC(Integer) AS Integer` local. The HIR shape checker explicitly permits this
isolation widening, but IR verification requires the isolation flags to equal.
The correct behavior is to accept the safe conversion, execute the ordinary
callable normally, and continue rejecting the reverse conversion.

Found while checking the architecture spec against implementation. Filed only,
as requested; no compiler or test changes are included.

## Failing Reproduction

Run from the repository root:

```sh
mkdir -p /tmp/mfb-architecture-callable
cat > /tmp/mfb-architecture-callable/project.json <<'JSON'
{"name":"probe","version":"0.1.0","mfb":"1.0","kind":"executable","sources":[{"root":"main.mfb"}],"entry":"main"}
JSON
cat > /tmp/mfb-architecture-callable/main.mfb <<'MFB'
ISOLATED FUNC identity(x AS Integer) AS Integer
  RETURN x
END FUNC
FUNC main AS Integer
  LET f AS FUNC(Integer) AS Integer = identity
  RETURN f(0)
END FUNC
MFB
./target/release/mfb build /tmp/mfb-architecture-callable --ir
```

Observed on macOS AArch64: exit 1, at the `LET`:

```text
error[2-203-0007 TYPE_BINDING_MISMATCH]: binding initializer type does not match declared type
Binding `f` has initializer type ISOLATED FUNC(Integer) AS Integer, expected FUNC(Integer) AS Integer.
```

Control: removing `ISOLATED` from `identity` makes the same release command
exit 0 and write `probe.ir`.

Expected: successful IR output. A plain executable build should run and exit 0.
The isolation flag is an additional guarantee; removing that guarantee at an
ordinary callable use does not permit a non-isolated function into an isolated
context.

## Root Cause

`src/ir/shape.rs:compatible` checks function isolation with
`!expected_isolated || actual_isolated`. In contrast,
`src/ir/verify/compat.rs:TypeEnv::compatible` accepts function pairs only with
`expected_isolated == actual_isolated`, equal returns, and matching parameter
shapes modulo `RES`. The fallback rendered-identity comparison cannot equate
`ISOLATED FUNC(...)` with `FUNC(...)`.

`TypeEnv::check_binding_type` reaches that predicate through
`expression_compatible`; the source pipeline always runs IR verification after
lowering (`src/cli/build/mod.rs:build_project`). Thus the HIR rule does not make
the program accepted. The architecture topic `type-inference` describes this
current difference; it must change with the fix.

Evidence commands:

```sh
sed -n '2965,3067p' src/ir/shape.rs
sed -n '550,685p' src/ir/verify/compat.rs
rg -n 'fn check_binding_type|expression_compatible|\.compatible\(' src/ir/verify
```

## Goal and Non-goals

- Accept isolated-to-ordinary callable assignment with otherwise identical
  parameter and return types, and prove its runtime result.
- Reject ordinary-to-isolated assignment and incompatible parameter/return types.
- Preserve the existing `RES` parameter-marker compatibility rule.
- Do not remove isolation checks, weaken negative fixtures, or add a special case
  only for the source `LET` syntax that bypasses decoded-package verification.
- General callable parameter/return variance is a separate semantic and ABI
  audit; this reproduction requires only the isolation flag conversion.

## Blast Radius

Search: `rg -n 'expression_compatible|\.compatible\(' src/ir/verify`.

- `compat.rs:check_binding_type` — observed failing local initialization; in scope.
- `compat.rs:check_return_type` and `calls.rs:check_call_argument_types` — share
  assignability and need positive/negative isolation tests; not independently
  reproduced yet.
- `values.rs:check_value_depth` — container element validation shares the
  predicate; test a list of callable values for the same conversion.
- `compat.rs:check_call_result_type`, `check_member_access_type`, and
  `check_operator_result_type` — annotation integrity checks also consume
  compatibility. Audit before widening to avoid accepting false package result
  annotations.
- `shape.rs:compatible` — already has the intended isolation direction;
  preserve it and test agreement for this narrow case.
- Monomorphized overload matching — separate exact-match selection contract;
  no change required merely to allow assignment after overload selection.

## Fix Design

Add narrowly scoped isolation widening in IR callable assignability while
retaining parameter/return equality and `RES` handling. Audit annotation checks
separately: directional assignability must not silently become equality of an
attacker-controlled annotation and its independently derived type.

## Phases

### Phase 1 — failing tests and consumer audit

- [ ] Add a positive syntax fixture reproducing the rejection and a runtime
      fixture that calls the assigned function.
- [ ] Add reverse-direction and incompatible-signature negative cases.
- [ ] Exercise return, argument, container, and decoded-package consumers.

Acceptance: the positive case fails for `TYPE_BINDING_MISMATCH`; the audit
records each consumer's required relation.
Commit: —

### Phase 2 — fix

- [ ] Implement safe isolation widening at the proper compatibility seam.
- [ ] Preserve independent annotation-integrity checks and `RES` compatibility.
- [ ] Update the architecture type-inference contract.

Acceptance: positive syntax and runtime cases pass; negative cases still reject.
Commit: —

### Phase 3 — validation

- [ ] Run the full suite before considering any expected-output changes; prove
      each changed expectation wrong before editing it.
- [ ] Run `cargo test` and
      `scripts/test-accept.sh target/debug/mfb target/accept-actual`.
- [ ] Rebuild release and rerun this reproduction as an executable; assert exit 0.

Acceptance: full validation passes, the original failure is resolved, and
package annotations cannot evade the verifier.
Commit: —
