# plan-165-A: Handler routing and typed return contracts

Last updated: 2026-10-04
Overall Effort: large (estimate: 3h–1d, excluding full-suite latency)
Effort: medium (estimate: 1h–2h)
Depends on: nothing
Status: In progress (go-ahead: `/follow-plan 165`, 2026-10-05)

## Review coverage and execution order

All review points have a written work item; this ledger describes documents, not
completed compiler work. Implementation order is A → B → C → D. B's 2026-10-04
revision (the owner's proof rule) touches only the built-in census and its guards,
so it no longer uses A's runtime target; the owner may run B before or alongside A
without changing anything else. C needs B **and** bug-825. Bug-797 and bug-798,
prerequisites of the later plans, are fixed (commits a5324104b and 526600e93;
`git merge-base --is-ancestor <hash> HEAD` succeeds for both). The 3.1 design must be
accepted before D.

| Review | Owning document | Scope |
|---|---|---|
| 3.1 | [plan-165-D](plan-165-D-expression-observation-traps.md) | Complete expression/assignment observation boundary and operator-only eligibility proposal |
| 3.1 discovered defect | [bug-798](../bugs/completed/bug-798-inline-trap-float-observation.md) | Premature Float observation under inline TRAP; fixed |
| 3.2 | [plan-165-C](plan-165-C-conditional-inline-trap.md) | Conditional AND/OR evaluation with recovery |
| 3.2 discovered defect | [bug-825](../bugs/bug-825-raising-comparison-in-and-or-escapes-inline-trap.md) | Raising comparison inside AND/OR escapes its inline TRAP; open; prerequisite of C |
| 3.3 | This plan | Handler routing and lexical PROPAGATE matrix |
| 3.4 | This plan | Declared RETURN type, Error as successful data, FAIL semantics |
| 3.5 | [plan-165-B](plan-165-B-conservative-handler-retention.md) | Positive proof required before treating a call as unable to fail |

## Goal

Specify and directly test handler failure routing, nested PROPAGATE binding, and
successful Error values without changing RETURN into a failure mechanism.

## Prerequisites

Met 2026-10-05: `/follow-plan 165` is the go-ahead; worktree `worktree-P-165` forked
from main `f23af579c`, status clean.

An explicit owner go-ahead to implement this plan (for example
`/follow-plan plan-165-A` or "implement plan-165-A"). Reviewing or discussing the
plan is not one. Before starting, run `git status --short` and isolate unrelated work.

## Current state and evidence

All probes below were re-run on 2026-10-04 with a debug compiler built from main
16df1c451 into `/tmp/mfb165-target` (`git archive HEAD | tar -x -C /tmp/mfb165-src`,
then `CARGO_TARGET_DIR=/tmp/mfb165-target cargo build`).

- `src/ast/stmt.rs:maybe_attach_postfix_trap` attaches TRAP to the complete parsed
  statement expression (`rg -n 'fn maybe_attach_postfix_trap' src/ast/stmt.rs`).
- `src/ir/lower.rs:lower_inline_trap` lowers the handler with its own error binding
  (`rg -n 'lower_statement_block\(&normalized' src/ir/lower.rs`), and
  `lower_statement` translates PROPAGATE to `IrOp::Fail` of the innermost
  `trap_name` (`rg -n 'HirStatement::Propagate \{ \.\. \} => vec' src/ir/lower.rs`).
- `mfb build /tmp/mfb-error-review-red/2` (copy: `/tmp/mfb165-probes/r2`) prints
  `90000002`, `90000003`, `inner=90000004`, `90000001`, `90000005`, each on its own
  line, exit 0: failing RECOVER and RETURN operands reached the function handler;
  nested recovery preserved the outer caught error; returning an Error succeeded.
  Built with `mfb build --debug -O <n>` for n = 0, 1, 3, the same program reports
  `arena.0.live_bytes 0` and `arena.0.double_free_skips 0` at every level. These
  exploratory cases must become permanent self-contained tests.
- `mfb build /tmp/mfb-error-review-red/3` rejects returning Error from an Integer
  function with `TYPE_RETURN_MISMATCH`.
- Reusing the outer inline error binding name in the inner handler is rejected with
  `SYMBOL_DUPLICATE_LOCAL` (`/tmp/mfb165-probes/adup`); use distinct names in
  routing tests. Do not silently change the language's shadowing policy.
- `src/docs/spec/language/08_error-model.md` already states part of this contract:
  §8.6 rule 3 (a function handler's own failures reach the caller; failures leaving
  an inline handler reach the function handler), rule 4 (PROPAGATE refers to the
  current error) and §8.5 ("`RETURN v` always means function success")
  (`rg -n 'always\*\* means function success|PROPAGATE. is valid' src/docs/spec/language/08_error-model.md`).
  Missing: the innermost-lexical PROPAGATE binding, the outer binding after nested
  recovery, and failing RECOVER/RETURN operands.
- Bug-797 (commit a5324104b) added §8.6a: a function handler may read a top-level
  local only if it is initialized on every failure edge
  (`TYPE_TRAP_LOCAL_UNINITIALIZED`, `src/ir/verify/trap_init.rs`).
- Bug-816 (commit 02bb44dc0) made **every counted FOR** a failure edge: its
  increment is checked and its bound/step tests promote like comparisons
  (`rg -n 'HirStatement::For \{ \.\. \} => true' src/ir/fallible.rs`;
  `rg -n 'IrOp::For \{ \.\. \} => true' src/ir/verify/trap_init.rs`). A function
  handler that reads a local declared after any FOR is rejected:
  `/tmp/mfb165-probes/afor` (a `FOR i = 1 TO 3` loop, then `LET marker = 7`, then a
  handler `RETURN marker`) fails with `TYPE_TRAP_LOCAL_UNINITIALIZED`.
- Bug-778 (commit 0bc2c56aa) moved an inline-TRAP `Result` temporary into frame
  slots: `ResultValue` **moves** a block payload into its binding, and the slots are
  zeroed when bound so drops on paths that did not store are sound
  (`rg -n 'fn finish_unboxed_trap_result|The owned slots read 0' src/codegen/error/result/unboxed_trap.rs`).
  Nested recovery and PROPAGATE are where a move-on-read can double-free or leak,
  which is why the memory-balance assertions below are required.

## Design

An inline trap protects its expression, not its handler body. A handler may contain
another inline trap protecting a new expression. PROPAGATE selects the caught
error of the innermost lexical handler; execution after nested recovery returns to
the outer binding. An unhandled failure in an inline handler routes to the eligible
function-level handler, otherwise to the caller. A function handler never catches
its own body's failures. An outer inline handler is not a catch region for its body.

| Origin/action | Required outcome |
|---|---|
| Logging call fails in inline handler | New error routes outward |
| RECOVER operand fails | No recovery; new error routes outward |
| RETURN operand fails | No return; new error routes outward |
| Nested inline expression fails | Nested handler runs |
| PROPAGATE in nested handler | Nested caught error routes outward |
| PROPAGATE after nested recovery | Outer caught error routes outward |
| Function handler body fails | Caller receives new error |

RETURN requires a value assignable to the declared success type. An Error is data
when permitted by that type; it is never implicitly interpreted as failure. FAIL
explicitly introduces failure. PROPAGATE and automatic routing forward failures.
This is type checking, not parsing: retain `TYPE_RETURN_MISMATCH`.

## Non-goals and compatibility

Do not change assignability, error-name shadowing, resource cleanup, implicit
propagation, the §8.6a initialization rule, bug-816's FOR fallibility or the result
ABI. Do not broaden operator-only TRAP eligibility. Existing routing appears
consistent in the probed cases; untested routes are UNVERIFIED, not presumed
correct. Byte identity is not a runtime correctness gate.

## Phases

### Phase 1 — Routing matrix

- [x] Add permanent runtime tests for every row above, with and without a function
      handler, including nested PROPAGATE and a failing logging helper.
      `tests/runtime/rt_error_model_recovery.rs:every_handler_route_reaches_the_documented_handler`
      (`ROWS` × `_nf`/`_wf`, plus three function-handler rows), -O0 and -O3;
      `cargo test --test rt_error_model_recovery` → `4 passed; 0 failed`.
- [x] Test named and bare handlers; assert error code, message and origin survive
      PROPAGATE, and the new error replaces the original on a new failure.
      Rows `nested_propagate_bare`/`after_recovery_bare`/`handler_propagates`
      compare `code:message@line` (line = the helper's `FAIL`), same run.
- [x] Test RETURN Error in an Error-returning function and rejection in Integer
      and Nothing functions, including return from handler bodies.
      `an_error_returned_from_an_error_function_is_successful_data` and
      `returning_an_error_from_a_non_error_function_is_a_type_mismatch` (6 cases).
      The Nothing cases were RED: `RETURN error(...)` in `AS Nothing` compiled →
      bug-827, fixed here (`bugs/completed/bug-827-*`).
- [x] Design the routing fixtures around §8.6a and bug-816: a function handler may
      read only parameters, globals, or locals declared before the first possible
      failure edge — and every counted FOR is one. Declare any local the handler
      reads before any FOR or fallible call; never "fix" a rejection by weakening
      the census. Prefer handlers that report through `e` or parameters.
      Every function handler in the new tests reads only its error binding; no
      `TYPE_TRAP_LOCAL_UNINITIALIZED` arose (builds above succeed).
- [x] For the nested-recovery and PROPAGATE rows (including a String and a
      List/record payload, where bug-778 moves a block on read), build with
      `--debug` at `-O0`, `-O1` and `-O3` and assert `arena.0.live_bytes 0` and
      `arena.0.double_free_skips 0` with `tests/common/debug_report.rs`
      (`arena_lines`, `counter`), as `tests/runtime/rt_inline_trap_success_alloc.rs`
      does. `nested_recovery_and_propagate_balance_the_arena` (String, List OF
      String, record × rec/prop/after × 40 iterations, output checked against a
      Rust mirror) passes.
- [x] If a route fails, capture a RED reproduction and fix through a bug document.
      No route failed. The type-check gap found alongside (bug-827) is fixed with
      `tests/syntax/functions/func-nothing-returns-value-invalid` and two
      `ir::verify::tests` cases.
- [x] Register the new target with a `[[test]]` entry in `Cargo.toml`, like the
      existing `tests/runtime/rt_inline_trap_*` targets.
- [x] Added (Validation discipline): fixtures
      `tests/rt-behavior/trap/inline-trap-handler-routing-rt` and
      `tests/syntax/trap/return-error-from-non-error-function-invalid`;
      `scripts/test-accept.sh … "inline-trap-handler-routing-rt"
      "return-error-from-non-error-function-invalid" "func-nothing-returns-value-invalid"`
      → `acceptance tests passed (3 test(s) ran)`.

Acceptance: proposed `cargo test --test rt_error_model_recovery` passes, proving
observable routing, memory balance at three optimization levels and compile-time
return-type rejection (estimate: minutes). Met: `4 passed; 0 failed` (65s).
Commit: 38d7f2f8b

### Phase 2 — Canonical specification

- [x] Extend the existing §8.6 rules 3–4 in
      `src/docs/spec/language/08_error-model.md` with the lexical binding rule and
      the routing table rows they do not already state; do not restate them.
      Rule 3 gains the handler-body/inline-handler/nested routes; rule 4 the
      innermost-lexical binding and the after-recovery outer binding.
- [x] Pair successful `RETURN error(...)` in `AS Error` with `FAIL error(...)`;
      explain that a failing RETURN operand prevents return from completing.
      New §8.5 paragraph.
- [x] Update related function/type cross-references without duplicating the table.
      `language/06_functions.md`: RETURN/FAIL pointer to §8.5 and the `AS Nothing`
      RETURN-operand rule (bug-827).

Acceptance: `target/debug/mfb spec language error-model` renders the complete
contract; `cargo test --bin mfb spec` passes (estimate: minutes). Met: rendered
topic shows the new rule-3/4 and §8.5 text (`… | grep -n innermost`);
`cargo test --bin mfb -- spec nothing_function inline_builtin` → every spec test
passed (the one failure is plan-165-B's intentionally RED coverage test).
Commit: (next commit)

## Open decision — review 3.1

Recommend TRAP on production of the complete assignment value, preserving all
existing numeric failure timing. The parser already supplies the full expression;
this would not mean trapping only the last division, nor deferring every error
until a final store. Integer failures remain immediate, while Float observation
occurs at its existing source boundaries. Bug-798 (fixed) showed why this distinction matters.
The complete proposal and implementation work order are in
[plan-165-D](plan-165-D-expression-observation-traps.md); acceptance of its design
and permission to implement remain separate decisions.

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

2026-10-03 review: bug-798 and bug-797 landed after this plan was written. Fixed
the bug-798 link (now under `bugs/completed/`) and its status, recorded the
existing §8.6 rules so Phase 2 extends rather than duplicates them, noted the
bug-797 handler-local rule, added `[[test]]` registration, and replaced the
session-specific authorization text with a reusable go-ahead condition.

2026-10-04 review (against 85a5f0a29, re-verified at 16df1c451): still valid; the
2026-10-04 TRAP fixes (bug-778, 807, 811, 814, 816, 820) met none of its goals.
Replaced line-range citations (which had drifted: `lower_inline_trap` now starts
after line 1800) with `rg` symbol commands; re-ran probes 2 and 3 and added the
duplicate-name and FOR probes; added the bug-816 FOR/§8.6a fixture constraint and
bug-778 memory-balance assertions at -O0/-O1/-O3; added bug-825 to the ledger and
recorded that B's revision no longer depends on A's runtime target.

2026-10-05 (implementation): Phase 1's "rejection in … Nothing functions" was not
true at `f23af579c` — `RETURN error(1, "x")`, `RETURN 5` in an `AS Nothing` function
compiled (`ir::verify::compat::check_return_type` skipped `Nothing`, a leftover
from when `TYPE_SUB_CANNOT_RETURN_VALUE` policed SUBs; plan-159-D removed it). Filed
and fixed as bug-827 rather than weakening the test.
