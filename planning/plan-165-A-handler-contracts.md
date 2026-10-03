# plan-165-A: Handler routing and typed return contracts

Last updated: 2026-10-03
Overall Effort: large (estimate: 3h–1d, excluding bug-798 and full-suite latency)
Effort: medium (estimate: 1h–2h)
Depends on: nothing
Status: Planning only; not authorized for implementation

## Review coverage and execution order

All review points have a written work item; this ledger describes documents, not
completed compiler work. Implementation order is A → B → C → D, with bug-798 fixed
before C and D, and the 3.1 design accepted before D.

| Review | Owning document | Scope |
|---|---|---|
| 3.1 | [plan-165-D](plan-165-D-expression-observation-traps.md) | Complete expression/assignment observation boundary and operator-only eligibility proposal |
| 3.1 discovered defect | [bug-798](../bugs/bug-798-inline-trap-float-observation.md) | Reproduced premature Float observation |
| 3.2 | [plan-165-C](plan-165-C-conditional-inline-trap.md) | Conditional AND/OR evaluation with recovery |
| 3.3 | This plan | Handler routing and lexical PROPAGATE matrix |
| 3.4 | This plan | Declared RETURN type, Error as successful data, FAIL semantics |
| 3.5 | [plan-165-B](plan-165-B-conservative-handler-retention.md) | Proof required before handler removal |

## Goal

Specify and directly test handler failure routing, nested PROPAGATE binding, and
successful Error values without changing RETURN into a failure mechanism.

## Prerequisites

The owner must explicitly authorize implementation of this plan. The current
instruction is to write bugs and plans only. Before starting, run
`git status --short` and isolate unrelated work.

## Current state and evidence

- `src/ast/stmt.rs:maybe_attach_postfix_trap` attaches TRAP to the complete parsed
  statement expression (`sed -n '470,552p' src/ast/stmt.rs`).
- `src/ir/lower.rs:lower_inline_trap` lowers handlers with their own error binding;
  `lower_statement` translates PROPAGATE to FAIL of that binding
  (`sed -n '1175,1203p' src/ir/lower.rs`; `sed -n '1895,1950p' src/ir/lower.rs`).
- Baseline build/run of `/tmp/mfb-error-review-red/2` produced
  `90000002`, `90000003`, `inner=90000004`, `90000001`, `90000005`, each on its own
  line: failing RECOVER and RETURN operands reached the function handler; nested
  recovery preserved the outer caught error; returning an Error succeeded.
  These exploratory cases must become permanent self-contained tests.
- `target/release/mfb build /tmp/mfb-error-review-red/3` rejected returning Error
  from an Integer function with `TYPE_RETURN_MISMATCH`.
- Reusing the outer inline error binding name in the inner handler was rejected
  with `SYMBOL_DUPLICATE_LOCAL`; use distinct names in routing tests. Do not
  silently change the language's shadowing policy.

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
propagation or the result ABI. Do not broaden operator-only TRAP eligibility.
Existing routing appears consistent in the probed cases; untested routes are
UNVERIFIED, not presumed correct. Byte identity is not a runtime correctness gate.

## Phases

### Phase 1 — Routing matrix

- [ ] Add permanent runtime tests for every row above, with and without a function
      handler, including nested PROPAGATE and a failing logging helper.
- [ ] Test named and bare handlers; assert error code, message and origin survive
      PROPAGATE, and the new error replaces the original on a new failure.
- [ ] Test RETURN Error in an Error-returning function and rejection in Integer
      and Nothing functions, including return from handler bodies.
- [ ] If a route fails, capture a RED reproduction and fix through a bug document.

Acceptance: proposed `cargo test --test rt_error_model_recovery` passes, proving
observable routing and compile-time return-type rejection (estimate: minutes).
Commit: —

### Phase 2 — Canonical specification

- [ ] Add the routing table and lexical binding rule to
      `src/docs/spec/language/08_error-model.md`.
- [ ] Pair successful `RETURN error(...)` in `AS Error` with `FAIL error(...)`;
      explain that a failing RETURN operand prevents return from completing.
- [ ] Update related function/type cross-references without duplicating the table.

Acceptance: `target/debug/mfb spec language error-model` renders the complete
contract; `cargo test --bin mfb spec` passes (estimate: minutes).
Commit: —

## Open decision — review 3.1

Recommend TRAP on production of the complete assignment value, preserving all
existing numeric failure timing. The parser already supplies the full expression;
this would not mean trapping only the last division, nor deferring every error
until a final store. Integer failures remain immediate, while Float observation
occurs at its existing source boundaries. Bug-798 shows why this distinction matters.
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
