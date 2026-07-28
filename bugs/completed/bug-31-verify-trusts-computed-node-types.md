# bug-31: `ir::verify_semantics` trusts computed nodes' self-reported result types, so hostile package IR defeats the member-access / operator type-confusion checks

Last updated: 2026-07-08
Effort: large (3h–1d)

`ir::verify_semantics` (`src/ir/verify/mod.rs`) is the semantic-verification pass
run in `merge_packages` before native lowering — the trust boundary that is meant
to reject type-confused IR from a decoded package (the mitigation for audit-1
PKG-02). Its type reconstruction, `infer_type` (`:3667-3683`), resolves every
**computed** node (`Call`/`CallResult`/`Binary`/`Unary`/`MemberAccess`/…) by
returning the node's **own serialized `type_`** via `annotated_type()`
(`value.rs:~137`). Nothing reconciles a computed node's declared result type
against its structural source of truth:

- `FnSig` (`:395-402`) stores `total`/`optional`/`params`/`kind` but **no return
  type**, so a `Call`/`CallResult` node's `type_` is never compared to the callee's
  actual `returns`.
- `Binary`/`Unary`/`MemberAccess` result types are never re-derived from operands.

On the source path the front end emits truthful annotations, so this is invisible.
On the **package path** — the very boundary this pass exists to guard — the
annotations are attacker-controlled, so the type-confusion rules validate a
fiction:

- `check_member_access` (`:1671-1707`): a `MemberAccess` whose `target` is a
  `CallResult{target:"pkg.getName", type_:"Account"}` (where `getName` actually
  returns `String` and `Account` is a record with an `Integer balance`) is
  accepted, because `infer_type(target)` echoes `"Account"`. Codegen then reads a
  `String` value at `Account.balance`'s offset — a type confusion / OOB read.
- `check_binary_operands` (`:1741-1746`): a `CallResult` to a `String`-returning
  FUNC annotated `type_:"Integer"` makes `numeric(lt)` true, so `stringResult - 5`
  passes and codegen emits an integer subtract over a string pointer — exactly the
  pointer-arithmetic hazard the function's own doc (`:1729-1733`) claims to prevent.

The single correct behavior a fix produces: on the package path, a computed node's
annotated `type_` is **reconciled** against a type derived from its structural
source (callee return type for calls, operand-derived type for Binary/Unary/member)
and any mismatch is rejected — so a hostile annotation cannot make type-confused IR
pass verification.

Severity HIGH: a soundness hole in the defense-in-depth trust boundary for decoded
packages. Practical exploitation is gated by the `.mfp` signature check
(`classify_installed_package` → `Verified`, see bug-27/cli) — so it requires a
malicious/compromised *signer* (supply-chain) — and the memory-unsafety consequence
depends on codegen consuming the same node annotation for layout/carrier selection
(register-native carrier selection and record-field layout do, per plan-20-B/C).
But this pass's stated purpose (plan-19, PKG-02) is precisely to catch such IR even
from a signed-but-malicious package, and it does not.

References:

- `src/ir/verify/mod.rs:3667-3683` (`infer_type` — echoes `annotated_type()` for
  computed nodes), `:395-402` (`FnSig` — no return type), `:1671-1707`
  (`check_member_access`), `:1729-1746` (`check_binary_operands` + its doc claim).
- `src/ir/value.rs:~137` (`annotated_type`).
- `merge_packages` → `check` (the decoded-package entry to this pass).
- audit-1 PKG-02 (decoded IR not re-typechecked — this pass is its mitigation).
- Contrast (safe): `MemberAccess` on a `Local`/`Global` resolves through the
  binding environment (`:846,3669-3670`), not a self-reported annotation; a
  `MemberAccess` on a `Const{Integer}` is correctly rejected.
- Found during goal-01 review of `src/ir/verify/**`.

## Failing Reproduction

Craft a `.mfp` whose IR declares `FUNC getName AS String`, a record `Account` with
`Integer balance`, and emits
`MemberAccess{ target: CallResult{target:"pkg.getName", args:[], type_:"Account"},
member:"balance", type_:"Integer" }`; import it.

- Observed: `verify_semantics` accepts the module; codegen reads the `String`
  result at the `Account.balance` offset (type confusion). Likewise a
  `String`-returning call annotated `Integer` passes `check_binary_operands` and
  codegens integer arithmetic over a string pointer.
- Expected: verification rejects the module — the call result annotated `Account`
  disagrees with `getName`'s `String` return type; the `Integer`-annotated string
  result disagrees with the callee's `String` return.

Contrast: member access on a `Local`/`Global` (type from the environment) and on a
`Const` primitive are checked correctly today.

## Root Cause

`infer_type` treats a node's serialized `type_` as ground truth for computed
nodes, and the pass never stores/derives the independent type it would need to
contradict a lie: `FnSig` omits the return type, and no operand-derivation exists
for `Binary`/`Unary`/`MemberAccess`. The "skip-if-unknown" discipline that is
sound for genuinely-unresolvable types becomes "trust-if-annotated" for
attacker-supplied annotations.

## Goal

- On the package path, every computed node's annotated result type is reconciled
  against a derived type and mismatches are rejected: `Call`/`CallResult` vs the
  callee's `returns`; `Binary`/`Unary`/`MemberAccess` vs operand/field-derived
  types.

### Non-goals (must NOT change)

- Source-path behavior (front-end annotations are truthful; goldens must not move
  for valid programs).
- The `"Unknown"` unresolved marker semantics (plan-20-C) for genuinely
  un-nameable types — but an `"Unknown"` must not be usable to *bypass* a check
  that a derived type would fail.

## Blast Radius

- `infer_type`, `FnSig`, `check_member_access`, `check_binary_operands`, and any
  other rule that consumes `infer_type` on a computed node. Add the callee return
  type to `FnSig` and thread operand-derivation into the result-type inference.

## Fix Design

Extend `FnSig` with the declared return type; when inferring a `Call`/`CallResult`
result, use (and, on the package path, *require agreement with*) the callee's
`returns` rather than the node annotation. Derive `Binary`/`Unary` result types
from operand types and `MemberAccess` from the resolved field type, and reject when
the node's annotation is incompatible with the derived type. Keep the source path
byte-identical by making reconciliation a rejection only on genuine disagreement
(the front end never disagrees).

## Phases

### Phase 1 — failing test + audit

- [ ] Add IR-verify tests feeding hand-built type-confused nodes (call result
      annotated as a foreign record; string result annotated Integer); assert
      rejection. Confirm they pass verification today.
- [ ] Audit every `infer_type` consumer for the same trust assumption.
- [x] Structural confirmation complete (above).

### Phase 2 — the fix

- [ ] Add return type to `FnSig`; reconcile computed-node annotations against
      derived types; reject mismatches on the package path.

### Phase 3 — validation

- [ ] `scripts/test-accept.sh` — valid-program goldens byte-identical; new
      rejection fixtures added.

## Validation Plan

- Regression test(s): the type-confusion rejection tests + a broad valid-program
  suite proving no false rejections.
- Runtime proof: importing the crafted `.mfp` is rejected instead of codegenning
  the confusion.
- Full suite: `scripts/test-accept.sh`.

## Open Decisions

- Reconcile on the package path only, or universally? Universal is simpler and
  catches front-end regressions, but risks golden churn if any existing annotation
  is loose. Recommended: universal reconciliation with the "Unknown" escape hatch
  preserved, validated against the full suite.

## Summary

The verifier's type reconstruction trusts computed nodes' self-reported types, so a
hostile package annotation defeats the member/operator confusion checks. Closing it
requires the pass to derive types independently (starting with a return type in
`FnSig`) and reject annotation-vs-derivation mismatches, without moving valid-program
goldens.
