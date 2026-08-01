# bug-404: IR verifier trusts attacker-controlled type annotations on `ResultValue`, `UnionWrap`, and `WithUpdate` without reconciling them against the actual value → type/layout confusion from a crafted `.mfp` (completes bug-162)

Last updated: 2026-07-28
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Security (verifier gap on untrusted imported IR) / Memory-safety

Status: Open
Regression Test: tests/ — verify-layer unit tests: a decoded `ResultValue` /
`UnionWrap` / `WithUpdate` whose annotation disagrees with its value's inferred
type must be rejected (Err), and legitimate source-lowered IR must still pass.

The IR verifier is the sole safety net for **untrusted imported-package IR**
(`check()` runs on decoded `.mfp`, with no syntaxcheck behind it). bug-162 fixed
the "unreconciled type annotation → member/layout confusion" class for `Call`,
`UnionExtract`, and member access — but its own text (bug-162 lines 26-27)
explicitly noted the **same pattern applies to `ResultValue`/`ResultError`/
`UnionExtract`**, and the landed fix only added `check_union_extract`. Three sibling
sites remain unreconciled:

### (a) `ResultValue.type_` — `src/ir/verify/values.rs:97`
The `ResultValue { value, .. }` arm only recurses into `value`; it never reconciles
the node's `type_` (attacker-controlled, `value.rs:156`) against the inner Result's
element type. `infer_type` (mod.rs:976) then trusts that `type_`. A crafted `.mfp`
emitting `MemberAccess { target: ResultValue { type_: "Account", value: <a Result
OF Integer> }, member: "balance" }` passes `check_member_access` (target type
`Account`, `balance` exists), and codegen reads `Account`'s record layout off an
`Integer`. Binding to the fabricated type also dodges `check_binding_type`
(declared == fabricated == `Account`). (`ResultError`/`ResultIsOk` are safe — their
annotated types are hardcoded `Error`/`Boolean`, `value.rs:165-166`.)

### (b) `UnionWrap` payload — `src/ir/verify/values.rs:80` → `check_union_wrap` (`compat.rs:632`)
`check_union_wrap` validates only that `member_type` is a declared variant of
`union_type`; it never checks the wrapped `value`'s type against `member_type`. A
crafted `UnionWrap { union_type: U, member_type: VariantA, value: Const{Integer,"5"}
}` passes (the tag is a real variant) but carries a wrong-typed payload; a later
`MATCH`/`UnionExtract` reads `VariantA`'s record layout off the `Integer` — the read
side is guarded (`check_union_extract`, bug-162) but the **wrap payload is not**.

### (c) `WithUpdate.type_` — `src/ir/verify/values.rs:125`
The `WithUpdate` arm takes `base`/`fields` from the node's `type_` and only falls
back to inferring the `target` when `type_` is empty/`Unknown`. A crafted
`WithUpdate { type_: "Account", target: <record B>, updates: [...] }` is checked
entirely against `Account`'s fields while the target is a different record;
`infer_type(WithUpdate)` returns the trusted `Account`, and codegen updates by
`Account`'s offsets → layout confusion. (Confidence: the verifier-side trust gap is
definite; the downstream memory-unsafety is inferred, not traced through
WithUpdate's exact lowering.)

**Each fix is provably false-reject-free** because source-path lowering makes the
annotation self-consistent: `ResultValue.type_ = success_type`
(`lower.rs:1141-1143`), `UnionWrap.member_type = actual inner type`
(`lower.rs:3312-3322`), and WithUpdate stamps `type_` from the target's type — so a
"reconcile annotation vs `infer_type(value)`, skip-if-unknown" check never rejects
legitimate IR.

References:

- `src/ir/verify/values.rs:97` (ResultValue), `:80`+`compat.rs:632` (UnionWrap),
  `:125` (WithUpdate).
- bug-162 (`bugs/completed/`), lines 26-27 (names ResultValue/ResultError as the
  same pattern) and its `check_union_extract` (read-side) fix — the model to mirror.
  Found during goal-07.

## Failing Reproduction

Static analysis across decode → verify → codegen; no crafted `.mfp` fixture built
(hand-authoring a malformed signed package is non-trivial). The verifier arms above
demonstrably lack any `type_`/`member_type`-vs-value reconciliation, unlike the
guarded `check_union_extract`.

- Observed: a `ResultValue`/`UnionWrap`/`WithUpdate` with a fabricated annotation
  passes `check()`.
- Expected: rejected with `VERIFY_TYPE`, like `UnionExtract`.

## Root Cause

verify/values.rs trusts the decoded `type_`/`member_type` annotation on these three
value kinds instead of reconciling it against `infer_type(value)`.

## Goal

- `ResultValue`, `UnionWrap` (payload), and `WithUpdate` reconcile their annotation
  against the actual value's inferred type (skip-if-unknown), rejecting a mismatch
  with `VERIFY_TYPE` — closing the member/layout-confusion class bug-162 began.

### Non-goals (must NOT change)

- No rejection of legitimate source-lowered IR; the skip-if-unknown discipline of
  `check_union_extract` must be preserved.

## Blast Radius

- `src/ir/verify/values.rs:80/:97/:125` and `src/ir/verify/compat.rs:632`
  (`check_union_wrap` payload check) — fixed by this bug.
- `check_union_extract` (`compat.rs:652`) — already guarded (bug-162); the template.
