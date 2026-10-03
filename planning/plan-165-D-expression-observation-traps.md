# plan-165-D: Inline TRAP for the complete expression value (review 3.1)

Last updated: 2026-10-03
Effort: medium (estimate: 1h–2h, excluding prerequisites and full-suite latency)
Depends on: plan-165-C; bug-798
Status: Design proposal only; no implementation authorized

## Goal

Allow operator-only expressions under inline TRAP with an unambiguous boundary:
produce the complete right-hand-side value for its binding or assignment, catching
failures encountered during that evaluation and its existing observation boundary.
RECOVER supplies the complete value. The trap never attaches to just the final
operator, and attaching it must not create additional numeric observation points.

## Prerequisites

- The owner accepts the proposed expression/observation contract below. Their
  current position is leaning toward assignment observation, not final approval.
- Plan-165-C is complete: inspect its active/archived task and commit ledger.
- Bug-798 is fixed: its ordinary/trapped Float parity tests pass. This plan does
  not absorb its work or implement a second observation mechanism.
- A separate explicit instruction authorizes implementation. Current work is
  authoring documents on main only; do not create a worktree or edit compiler code.

## Current state and evidence

`src/ast/stmt.rs:maybe_attach_postfix_trap` wraps the already parsed whole
expression (`sed -n '470,552p' src/ast/stmt.rs`). There is no parser ambiguity
between the last division and the whole expression.

`src/ir/lower.rs:lower_inline_trap` gates extraction on
`scrutinee_has_runtime_call` and only checks a root operator after an operand was
lifted (`sed -n '1785,1875p' src/ir/lower.rs`). Thus changing eligibility alone
would miss the single-operator root: both eligibility and capture must change.

`src/ir/verify/resources.rs:check_inline_trap_scrutinee` permits Checked but rejects
call-free ordinary values and package constants; it also emits dead-handler
warnings (`sed -n '980,1085p' src/ir/verify/resources.rs`).
`src/docs/spec/language/04_types.md` distinguishes eager integer failures from
Float observation and permits transient nonfinite intermediates
(`rg -n 'observation|intermediate' src/docs/spec/language/04_types.md`). Bug-798
reproduces an existing violation of that distinction under inline TRAP.

## Proposed language contract

For `LET value = 1 / 2 / 3 / 4 TRAP(e)`, the protected expression is the entire
right-hand side with the language's existing precedence and associativity.

| Event | Handler behavior |
|---|---|
| Checked integer operation fails before the final operation | Enter handler immediately; skip remaining evaluation |
| Call or argument evaluation fails | Enter handler; skip later operands/calls |
| Float intermediate is nonfinite but becomes finite before observation | Continue; do not invent an intermediate trap |
| Final Float assignment/argument observation fails | Enter the eligible trap protecting that evaluation |
| Short-circuit right operand is unselected | Do not evaluate it or raise its potential error |
| RECOVER succeeds | Deliver its value as the complete expression result |
| Handler fails or diverges | Follow plan-165-A routing; do not commit the failed assignment |

“Observation at assignment” identifies the produced value and its delivery boundary;
it does not mean defer all failures until after evaluating the RHS. An integer
zero divisor cannot produce a provisional value for later operations. Side effects
already performed before a failure are not rolled back. For a discarded expression,
preserve its existing observation rules; discarding must not invent a Float store.

Recommended eligibility: any well-typed expression at the currently supported
binding/assignment/bare-expression positions. A known-total expression can carry a
provably unreachable handler, consistent with known-total calls. This avoids making
syntax legality depend on the precision of fallibility inference. An alternative
is to admit only potentially failing operators; owner acceptance must settle this
before implementation. Neither option permits missing required checks.

## Compiler design

1. Preserve the parser's complete-expression attachment; no precedence change.
2. Replace the runtime-call requirement in `lower_inline_trap` with the accepted
   expression eligibility rule. Capture a raising root even when nothing is hoisted.
3. Use the corrected observation representation from bug-798 for Float trees;
   retain plan-165-C's selected-branch evaluation and shared handler.
4. Produce a correctly typed success result for known-total expressions without
   evaluating the expression twice. Warning/removal requires plan-165-B proof.
5. Update `check_inline_trap_scrutinee` for the new source contract without
   weakening general Result typing or decoded-package safety checks.
6. Keep RECOVER assignability, handler divergence, resource/state assignment and
   existing syntactic placement restrictions. Do not broaden RETURN syntax.

Risk concentrates in a root with no lifted children, Float observation timing,
assignment delivery on failure, and eligibility diagnostics inferred from lowered
shapes. Do not encode eligibility by the presence of a particular synthetic temp.

## Compatibility and non-goals

Previously rejected operator-only forms become valid after approval. Preserve
arithmetic values, failure codes/origins, side-effect order and result ABI. No
transactional rollback, exceptions, arbitrary statement-block trapping or changed
numeric overflow policy. Expected fixture differences are the newly admitted forms
and any proven-dead warnings; byte identity is not the feature acceptance gate.

Testing assertions (`expectTrap`/`expectNTrap`) share lowering but have their own
call-only shape rule. Keep that public assertion API unchanged in this plan; ensure
its diagnostics remain covered rather than widening it accidentally.

## Existing-test evidence

`git log --follow -S 'a_bare_operator_scrutinee_is_still_rejected' --
tests/runtime/rt_inline_trap_raising_operator.rs` identifies the bug-471 regression
control. It protects the then-deliberate call-only eligibility policy. Find all
consumers with `rg -n 'TYPE_INLINE_TRAP_REQUIRES_FALLIBLE|call-free|scrutinee_has_runtime_call'
src tests`. Owner acceptance of this proposal is the independent contract evidence
for replacing those rejection assertions; until then they remain correct.

## Phases

### Phase 1 — Specify and test the accepted boundary

- [ ] Record owner acceptance and the selected total-expression eligibility policy.
- [ ] Add RED runtime cases for division failures in the first, middle and last
      positions of a chain; RECOVER must replace the entire result in every case.
- [ ] Cover Integer/Byte/Fixed/Money checked operations, Float final overflow,
      transient nonfinite recovery, comparisons and mandatory FMA parity.
- [ ] Cover parenthesized expressions, assignment to an existing target, call
      arguments, conditional operators and discarded expressions. Assert that
      failure skips later effects and leaves the target unchanged until recovery.
- [ ] Preserve invalid RECOVER types, invalid handler fallthrough, forbidden trap
      positions and testing-assertion call requirements.

Acceptance: proposed `cargo test --test rt_inline_trap_expression_boundary` has RED
cases for newly admitted forms and passing controls for existing contracts
(estimate: minutes after build).
Commit: —

### Phase 2 — Expression capture and spec

- [ ] Implement accepted eligibility/root capture and typed total-expression success.
- [ ] Update error-model, operator/Float cross-references, IR lowering and diagnostic
      rule descriptions together; include the chained-division boundary example.
- [ ] Add valid fixtures under `tests/rt-behavior/trap` and invalid counterparts
      under `tests/syntax/trap`; revise only independently disproved expectations.

Acceptance: the proposed runtime target proves whole-result recovery and unchanged
observation timing; `cargo test --bin mfb spec` and rendered error-model topic agree
(estimate: minutes after build).
Commit: —

## Validation

Use scoped runtime checks while implementing, including default and optimized
builds. Run full `cargo test --no-fail-fast` and
`scripts/test-accept.sh target/debug/mfb target/accept-error-model` before any
existing golden correction. Investigate unexpected diffs and apply the repository's
history/contract proof before changing expectations. Full gates may take hours
(estimate); they cover generated artifacts beyond the scoped runtime tests.

## Open decisions

Owner approval of the proposed boundary and whether known-total expressions are
legal. The recommendation above is fully specified, but neither decision is treated
as implementation permission.

## Corrections

Review 3.1 previously had only an open-decision paragraph in plan-165-A. This
standalone proposal supplies the missing design, code changes and verification work.
