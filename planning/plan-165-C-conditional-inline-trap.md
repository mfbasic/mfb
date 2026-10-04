# plan-165-C: Preserve conditional evaluation under inline TRAP

Last updated: 2026-10-03
Effort: medium (estimate: 1h–2h)
Depends on: plan-165-B; bug-798 (fixed)
Status: Not started; implementation awaits the owner's go-ahead

## Goal

Allow fallible right operands of AND/OR under inline TRAP. Evaluate only selected
operands, in source evaluation order; any selected failure reaches the one inline
handler and skips the rest of the guarded expression.

## Prerequisites

- Plan-165-B complete: inspect its task/commit ledger at its active or archived path.
- Bug-798 is fixed (commit 526600e93, `bugs/completed/bug-798-inline-trap-float-observation.md`).
  Before starting, confirm its parity matrix still passes:
  `cargo test --test rt_inline_trap_float_observation`. Conditional lowering must
  preserve that observation model, not bypass it.
- An explicit owner go-ahead to implement this plan (for example
  `/follow-plan plan-165-C`). Reviewing or discussing the plan is not one.

## Current state and evidence

`src/ir/lower.rs:scan_trap_operands` and `rewrite_trap_operands` skip the right
operand of AND/OR; `src/ir/shape.rs:check_trap_short_circuit` rejects fallible calls
and raising operators there (`sed -n '2375,2445p' src/ir/lower.rs`;
`sed -n '1810,2028p' src/ir/shape.rs`).
Baseline `target/release/mfb build /tmp/mfb-error-review-red/0` and the analogous
`/1` project reject conditional calls and division with
`TYPE_INLINE_TRAP_SHORT_CIRCUIT_CALL`; root Boolean expressions may additionally
report `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE` because the skipped call was not lifted.

`lower_inline_trap` already uses one shared failure flag/error slot and emits one
handler after its evaluation chain (`sed -n '1980,2150p' src/ir/lower.rs`).
Extend that structure rather than cloning handlers.

## Design

Represent extracted evaluation as ordered steps. A value step uses the existing
CallResult/Checked success check. A conditional step owns its right operand's steps:

1. Evaluate the left operand and store the Boolean result.
2. For AND, evaluate the right steps only if left is true; for OR, only if false.
3. Assign the conditional result only when its selected right evaluation succeeds.
4. Continue later expression steps only if the shared failure flag is still clear.
5. Enter the existing single handler on failure; RECOVER supplies the entire
   trapped expression's success value, not just the failed Boolean operand.

A conditional with an infallible but side-effecting right call must also remain
ordered before a later failing sibling. Handle conditional roots as well as nested
arguments. Make Boolean result locals visible after the selected branch without
exposing uninitialized values on failure. Preserve typed recovery for assignment,
state assignment, bindings and discarded expressions.

Do not hoist right-operand evaluation unconditionally. Do not treat an outer
inline handler body as an eligible trap region. Preserve bug-798's corrected Float
model: a Float `+`, `-`, `*`, `/`, `DIV` or negation is lifted only at a boundary
(not under another operator), so `a*b-c` contraction and transient nonfinite
intermediates survive; `MOD`/`^` stay lifted in place as capture-only checks
(`Checked.observe = false`, IR tag 23). A newly lifted right operand follows the
same rule. Preserve the decoded-package verifier's Checked invariants.

Bug-797's failure-edge census (`src/ir/verify/trap_init.rs`) treats a call or
operator under an inline TRAP (`CallResult`, `Checked`) as captured, not as an edge
into the function handler. Newly lifted right operands must lower to those same
forms so a function handler's readable locals do not change; the inline handler's
own `FAIL`/`PROPAGATE` stays an ordinary edge.

## Non-goals and compatibility

Operator-only expressions remain subject to the existing call-eligibility rule
until review 3.1 is explicitly decided. No new syntax, exception unwinding, result
ABI or package opcode is required for conditional control flow itself. Retire the
obsolete rejection without reusing its diagnostic number for another meaning.
Newly accepted programs intentionally change output; byte identity is not the gate.

## Existing-test evidence before changes

- `git log --follow -S 'a_short_circuited_raising_operator_is_rejected' --
  tests/runtime/rt_inline_trap_raising_operator.rs` identifies the bug-471 fix.
- `git log --follow -S 'a_fallible_call_in_a_short_circuited_operand_is_rejected' --
  tests/runtime/rt_inline_trap_nested_call.rs` identifies the bug-457 fix.
- Protected behavior: a conditional failure must never silently escape its inline
  handler or execute when its operand is skipped.
- Consumer census: `rg -l 'TYPE_INLINE_TRAP_SHORT_CIRCUIT_CALL' tests src/docs/spec
  src/rules` finds runtime tests, architecture/error-model specs and rule registries.
- Independent reason rejection becomes obsolete: the owner's requested semantics
  permit conditional evaluation with recovery. Replace rejection assertions with
  runtime evidence of skipped effects and selected failure capture, not mere build
  success. Preserve the original escaping-error protection.

## Phases

### Phase 1 — Runtime contract

- [ ] Add RED tests for selected/skipped AND and OR calls and raising operators.
- [ ] Include nested conditions, a failing left operand, later arguments that must
      be skipped after failure, and infallible side effects before later failures.
- [ ] Cover call arguments and Boolean expression roots; use bindings/assignment/
      discard/state targets where valid. Preserve invalid recovery-type fixtures.
- [ ] Include Float comparison/domain/boundary cases from bug-798 and both default
      and optimized builds.
- [ ] Assert a function-level handler may still read a local declared after a
      trapped conditional whose selected right operand can fail (bug-797 census).
- [ ] Register the new target with a `[[test]]` entry in `Cargo.toml`, like the
      existing `tests/runtime/rt_inline_trap_*` targets.

Acceptance: proposed `cargo test --test rt_inline_trap_short_circuit` fails only
because the currently rejected semantics are missing (estimate: minutes after build).
Commit: —

### Phase 2 — Structured lowering and documentation

- [ ] Implement conditional steps in `src/ir/lower.rs` using existing structured IR.
- [ ] Remove the obsolete shape rejection and any newly unused helper; retain
      other inline-trap eligibility and type checking.
- [ ] Replace only disproved existing rejection assertions using the evidence above.
- [ ] Update `src/docs/spec/language/08_error-model.md`,
      `src/docs/spec/architecture/04_ir.md`, and diagnostic rule documentation.
- [ ] Remove the unsafe suggestion to pre-evaluate the right operand in a binding;
      illustrate conditional evaluation directly.

Acceptance: short-circuit runtime tests prove exact effects, results and handler
selection; nested-call/operator regression targets also pass (estimate: minutes).
Commit: —

## Open decisions

Review 3.1 remains separate. Conditional evaluation does not require deciding whether
an expression with no runtime call may carry an inline TRAP.

## Validation discipline

New runtime tests execute compiled programs and assert stdout, error codes and
side-effect order. Add matching valid/invalid fixtures under the repository's
`tests/rt-behavior/trap` and `tests/syntax/trap` trees. Do not weaken existing
assertions: record history, protected behavior, consumers and an independent
contract/reproduction proving each changed expectation wrong.

Run formatting and scoped tests during implementation. At the end of this
sub-plan, run `cargo test --no-fail-fast` and
`scripts/test-accept.sh target/debug/mfb target/accept-error-model` before any
existing golden correction; full cargo testing includes the artifact gate.
These broad checks are required by the repository and may take hours (estimate).
After justified corrections, rerun affected checks and required final gates.
Verify specs with `cargo build`, `cargo test --bin mfb spec`, and rendered topics.

## Corrections

Planning only. An initial implementation attempt was removed at the owner's
instruction; no compiler or test changes are retained. Any experimental success
is not acceptance evidence for a future implementation.

2026-10-03 review: bug-798 and bug-797 landed after this plan was written. Marked
the bug-798 prerequisite met with its regression command, named the Float model it
shipped, added the bug-797 census interaction and a test for it, added `[[test]]`
registration, and replaced the session-specific authorization text with a reusable
go-ahead condition.
