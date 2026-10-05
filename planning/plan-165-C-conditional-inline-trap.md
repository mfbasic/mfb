# plan-165-C: Preserve conditional evaluation under inline TRAP

Last updated: 2026-10-04
Effort: medium (estimate: 1h–2h)
Depends on: plan-165-B; bug-825; bug-798 (fixed)
Status: In progress (go-ahead: `/follow-plan 165`, 2026-10-05)

## Goal

Allow fallible right operands of AND/OR under inline TRAP. Evaluate only selected
operands, in source evaluation order; any selected failure reaches the one inline
handler and skips the rest of the guarded expression. "Fallible right operand"
includes a fallible call, a raising arithmetic operator, and a raising comparison
(`src/ir/fallible.rs:comparison_can_raise`, bug-816).

## Prerequisites

Met 2026-10-05: plan-165-B ticked (commit 695295c48); bug-825 fixed
(`git merge-base --is-ancestor 58a4e76f7 HEAD`); `cargo test --test
rt_inline_trap_float_observation` → `1 passed` at the plan-B tip; `/follow-plan 165`.

- Plan-165-B complete: inspect its task/commit ledger at its active or archived path.
- Bug-825 fixed (`bugs/bug-825-raising-comparison-in-and-or-escapes-inline-trap.md`,
  Status: Open on 2026-10-04). Bug-825 makes today's short-circuit rejection cover a
  raising comparison; this plan then replaces that rejection with lowering. Do not
  fix bug-825 here or duplicate its spec/comment corrections; its regression fixture
  becomes one of this plan's consumers (see "Existing-test evidence").
- Bug-798 is fixed (commit 526600e93, `bugs/completed/bug-798-inline-trap-float-observation.md`).
  Before starting, confirm its parity matrix still passes:
  `cargo test --test rt_inline_trap_float_observation`. Conditional lowering must
  preserve that observation model, not bypass it.
- An explicit owner go-ahead to implement this plan (for example
  `/follow-plan plan-165-C`). Reviewing or discussing the plan is not one.

## Current state and evidence

Probes re-run on 2026-10-04 with a debug compiler built from main 16df1c451
(`/tmp/mfb165-target/debug/mfb`; copies under `/tmp/mfb165-probes`).

`src/ir/lower.rs:scan_trap_operands` and `rewrite_trap_operands` skip the right
operand of AND/OR via `is_short_circuit_operator`
(`rg -n 'fn scan_trap_operands|fn rewrite_trap_operands|is_short_circuit_operator' src/ir/lower.rs`);
`src/ir/shape.rs:check_trap_short_circuit` rejects fallible calls and raising
operators there (`rg -n 'fn check_trap_short_circuit|operator_can_raise' src/ir/shape.rs`).
`mfb build /tmp/mfb-error-review-red/0` rejects all five trapped statements with
`TYPE_INLINE_TRAP_SHORT_CIRCUIT_CALL`; four Boolean-root ones additionally report
`TYPE_INLINE_TRAP_REQUIRES_FALLIBLE` because the skipped call was not lifted.
`/tmp/mfb-error-review-red/1` (`finish(selected AND 1 / zero > 0)`) is rejected with
`TYPE_INLINE_TRAP_SHORT_CIRCUIT_CALL` twice.

A raising comparison in the right operand is **not** rejected today: the check asks
only `operator_can_raise`. `/tmp/p165/c1` (`finish(selected AND fx < big)` with `fx`
Fixed, `big = 3000000000`) compiles and prints `function handler 77050010`, exit 3 —
the `ErrOverflow` escapes the inline TRAP. That is bug-825, filed separately.

`lower_inline_trap` already uses one shared failure flag/error slot and emits one
handler after its evaluation chain
(`rg -n 'fn lower_inline_trap|The single handler, reached from whichever check failed' src/ir/lower.rs`).
Extend that structure rather than cloning handlers.

Bug-778 (commit 0bc2c56aa) keeps each trap temporary in frame slots when
`function_lowering.rs:unboxed_trap_result_candidates` proves it is bound once and
read only by `ResultIsOk`/`ResultValue`/`ResultError`; otherwise it falls back to a
heap `Result` block. The slots are zeroed **where the temporary is bound**, so a
drop on a path that never stored is sound
(`rg -n 'The owned slots read 0' src/codegen/error/result/unboxed_trap.rs`).
`tests/runtime/rt_inline_trap_success_alloc.rs` pins allocation parity
(`arena.0.alloc_calls` trapped = plain) and balance (`live_bytes`,
`double_free_skips`).

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

Right-operand raise sites are exactly what `trap_hoist_kind` lifts elsewhere: a
fallible call, a raising operator (`operator_can_raise`), and a comparison for which
`comparison_can_raise(op, left, right)` holds — a `Fixed` compared with an
`Integer`, `Float` or `Unknown` operand, in either order (`ErrOverflow` out of the
Fixed range, `ErrInvalidFormat` for a non-finite Float). A comparison of
Float/Integer/Byte operands with no Fixed side is not a raise site.

Do not hoist right-operand evaluation unconditionally. Do not treat an outer
inline handler body as an eligible trap region. Preserve bug-798's corrected Float
model: a Float `+`, `-`, `*`, `/`, `DIV` or negation is lifted only at a boundary
(not under another operator), so `a*b-c` contraction and transient nonfinite
intermediates survive; `MOD`/`^` stay lifted in place as capture-only checks
(`Checked.observe = false`, IR tag 23). A newly lifted right operand follows the
same rule. A comparison is still not an observation boundary for Float-vs-Float/
Integer/Byte, but since bug-816 a non-finite Float compared with a Fixed is
converted and raises `ErrInvalidFormat` (spec `language/04_types.md`,
`rg -n 'compared with a .Fixed., it is first converted' src/docs/spec/language/04_types.md`).
Preserve the decoded-package verifier's Checked invariants.

**Bug-778 slots under a skipped branch.** A `CallResult`/`Checked` lowered inside a
conditional branch binds — and therefore zeroes — its slots only when the branch
runs. Every drop or move-on-read of that temporary must either be confined to the
branch or have its slots zeroed on a path that dominates the drop (for example at
the conditional step's entry), so a skipped branch never frees garbage and a loop
iteration never sees the previous iteration's payload. The candidate analysis must
keep accepting the branch-bound temporaries; a fallback to the heap form is an
allocation-parity regression, not an acceptable outcome.

Bug-797's failure-edge census (`src/ir/verify/trap_init.rs`) treats a call or
operator under an inline TRAP (`CallResult`, `Checked`) as captured, not as an edge
into the function handler. Newly lifted right operands — raising comparisons
included — must lower to those same forms so a function handler's readable locals
do not change; the inline handler's own `FAIL`/`PROPAGATE` stays an ordinary edge.

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
- Bug-825's regression fixture (an invalid fixture expecting
  `TYPE_INLINE_TRAP_SHORT_CIRCUIT_CALL` for a raising comparison) is added by that
  bug; treat it exactly like the two above.
- Protected behavior: a conditional failure must never silently escape its inline
  handler or execute when its operand is skipped.
- Consumer census: `rg -l 'TYPE_INLINE_TRAP_SHORT_CIRCUIT_CALL' tests src/docs/spec
  src/rules` finds `tests/runtime/rt_inline_trap_nested_call.rs`,
  `tests/runtime/rt_inline_trap_raising_operator.rs`,
  `src/docs/spec/language/08_error-model.md`, `src/docs/spec/architecture/04_ir.md`,
  `src/docs/spec/diagnostics/01_rule-codes.md` and `src/rules/table.rs` (2026-10-04;
  re-run after bug-825 lands).
- Independent reason rejection becomes obsolete: the owner's requested semantics
  permit conditional evaluation with recovery. Replace rejection assertions with
  runtime evidence of skipped effects and selected failure capture, not mere build
  success. Preserve the original escaping-error protection.

## Phases

### Phase 1 — Runtime contract

- [x] Add RED tests for selected/skipped AND and OR calls, raising operators, and
      raising comparisons (Fixed vs Integer out of range → `ErrOverflow`; Fixed vs
      non-finite Float → `ErrInvalidFormat`; both operand orders), each asserting
      the inline handler — not the function handler — catches it (the
      `/tmp/p165/c1` shape with a function-level TRAP present).
      `tests/runtime/rt_inline_trap_short_circuit.rs` (every `main` carries a
      function-level TRAP printing `fh …`).
- [x] Include nested conditions, a failing left operand, later arguments that must
      be skipped after failure, and infallible side effects before later failures.
      `nested_conditions_and_failures_skip_exactly_the_rest`,
      `an_infallible_side_effect_stays_ordered_before_a_later_failure` — the latter
      BUILDS at HEAD and prints `marks=0`: the infallible `mark` in `yes AND mark(1)`
      ran after the lifted `boom` (an ordering bug this plan's lowering fixes).
- [x] Cover call arguments and Boolean expression roots; use bindings/assignment/
      discard/state targets where valid. Preserve invalid recovery-type fixtures.
      `conditional_results_reach_every_target` (binding, assignment, discard); the
      STATE-assign target shares `InlineTrapTarget` delivery and is not separately
      exercised (no conditional-specific code touches it). No recovery-type fixture
      is edited.
- [x] Float cases from bug-798 and bug-816: a transient non-finite compared with a
      Float/Integer/Byte in a selected right operand does not trap; compared with a
      Fixed it raises `ErrInvalidFormat` into the handler; contraction parity holds.
      Run default and optimized builds. `a_float_comparison_in_a_selected_operand_…`
      and the `nonfinite` rows of `raising_operators_…`, each at -O0 and -O3.
- [x] Memory and allocation: for String, record and List payloads in a right
      operand, build with `--debug` at `-O0`, `-O1`, `-O3` and assert
      `arena.0.live_bytes 0` and `arena.0.double_free_skips 0` with the branch
      skipped, selected-and-succeeding, and selected-and-failing, including inside a
      loop that alternates them; assert per-evaluation `arena.0.alloc_calls` equals
      the plain (untrapped, pre-bound) twin on the success path, using the
      N/2N-delta method of `tests/runtime/rt_inline_trap_success_alloc.rs`.
      `right_operand_payloads_balance_the_arena_on_every_path`,
      `the_trapped_conditional_success_path_allocates_what_the_plain_one_does`.
- [x] Assert a function-level handler may still read a local declared after a
      trapped conditional whose selected right operand can fail (bug-797 census);
      declare that local before any counted FOR (bug-816 makes every FOR a failure
      edge). `a_trapped_conditional_is_not_a_function_handler_failure_edge`.
- [x] Register the new target with a `[[test]]` entry in `Cargo.toml`, like the
      existing `tests/runtime/rt_inline_trap_*` targets.

Acceptance: proposed `cargo test --test rt_inline_trap_short_circuit` fails only
because the currently rejected semantics are missing (estimate: minutes after build).
Met: `0 passed; 9 failed` — eight on `TYPE_INLINE_TRAP_SHORT_CIRCUIT_CALL` (with
`REQUIRES_FALLIBLE` on Boolean roots, `TRAP_LOCAL_UNINITIALIZED` in the census
case), one on the ordering bug (`marks=0`).
Commit: (next commit)

### Phase 2 — Structured lowering and documentation

- [ ] Implement conditional steps in `src/ir/lower.rs` using existing structured IR,
      asking `trap_hoist_kind` (which already consults `comparison_can_raise`) for
      right-operand raise sites.
- [ ] Keep bug-778 slot zeroing dominating every drop of a branch-bound temporary;
      confirm with the Phase 1 balance and parity assertions.
- [ ] Remove the obsolete shape rejection and any newly unused helper; retain
      other inline-trap eligibility and type checking.
- [ ] Replace only disproved existing rejection assertions using the evidence above.
- [ ] Update `src/docs/spec/language/08_error-model.md`,
      `src/docs/spec/architecture/04_ir.md`, and diagnostic rule documentation.
- [ ] Remove the unsafe suggestion to pre-evaluate the right operand in a binding;
      illustrate conditional evaluation directly.

Acceptance: short-circuit runtime tests prove exact effects, results, handler
selection, memory balance and allocation parity; `rt_inline_trap_nested_call`,
`rt_inline_trap_raising_operator`, `rt_inline_trap_success_alloc` and
`rt_inline_trap_float_observation` also pass (estimate: minutes).
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

2026-10-04 review (against 85a5f0a29, re-verified at 16df1c451): still valid; probes
0 and 1 still give `TYPE_INLINE_TRAP_SHORT_CIRCUIT_CALL`/`REQUIRES_FALLIBLE`.
Replaced drifted line ranges with `rg` symbol commands. Added raising comparisons
(bug-816 `comparison_can_raise`) as right-operand raise sites in goal, design and
tests; updated the Float rules for bug-816's Float/Fixed conversion
(`ErrInvalidFormat`); added the bug-778 skipped-branch slot hazard with balance and
allocation-parity tests; made bug-825 (the escape the review found) a prerequisite
rather than part of this plan.
