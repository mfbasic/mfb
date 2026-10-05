# plan-165-D: Inline TRAP for the complete expression value (review 3.1)

Last updated: 2026-10-05
Effort: medium (estimate: 1h–2h, excluding prerequisites and full-suite latency)
Depends on: plan-165-C; bug-825; bug-798 (fixed)
Status: Implemented 2026-10-05 (go-ahead: `/follow-plan 165`)

## Goal

Allow operator-only expressions under inline TRAP with an unambiguous boundary:
produce the complete right-hand-side value for its binding or assignment, catching
failures encountered during that evaluation and its existing observation boundary.
RECOVER supplies the complete value. The trap never attaches to just the final
operator, and attaching it must not create additional numeric observation points.

## Prerequisites

- Met (2026-10-05): the owner accepted the expression/observation contract below
  and chose eligibility **Option 1** (any well-typed expression).
- Met 2026-10-05: plan-165-C is complete (commits d1fe2cdac, 63eacbbbb; no
  unticked box); `cargo test --test rt_inline_trap_float_observation` → `1 passed`
  at the plan-C tip; `/follow-plan 165` is the go-ahead.
- Plan-165-C is complete: inspect its active/archived task and commit ledger.
- Met: bug-825 is fixed (commits 58a4e76f7, dbcb17396). Rule 11 now says
  `a & b TRAP(e)` is rejected with `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE`; under
  Option 1 this plan rewrites that sentence to the dead-handler warning.
- Bug-798 is fixed (commit 526600e93). Before starting, confirm its parity
  matrix still passes: `cargo test --test rt_inline_trap_float_observation`. This
  plan reuses that observation mechanism and must not add a second one.
- A separate explicit owner go-ahead to implement (for example
  `/follow-plan plan-165-D`), given after the contract is accepted. Accepting the
  design or discussing the plan is not one.

## Current state and evidence

Probes re-run on 2026-10-04 with a debug compiler built from main 16df1c451
(`/tmp/mfb165-target/debug/mfb`; copies under `/tmp/mfb165-probes`).

`src/ast/stmt.rs:maybe_attach_postfix_trap` wraps the already parsed whole
expression (`rg -n 'fn maybe_attach_postfix_trap' src/ast/stmt.rs`). There is no
parser ambiguity between the last division and the whole expression.

`src/ir/lower.rs:lower_inline_trap` gates extraction on
`scrutinee_has_runtime_call` and only checks a root operator after an operand was
lifted (`rg -n 'scrutinee_has_runtime_call|let root_operator_raises' src/ir/lower.rs`).
Thus changing eligibility alone would miss the single-operator root: both
eligibility and capture must change. Bug-779 (fixed, `9f6965ebc`) added that gate so
a call-free operator chain (`LET v = i * 2 + 3 TRAP(err)`) is rejected; its fixture
`tests/syntax/trap/inline-trap-call-free-operator-chain-invalid` protects today's rule.

`src/ir/verify/resources.rs:check_inline_trap_scrutinee` permits Checked but rejects
call-free ordinary values and package constants; it also emits dead-handler
warnings (`rg -n 'fn check_inline_trap_scrutinee|"TYPE_INLINE_TRAP_DEAD_HANDLER"|"TYPE_INLINE_TRAP_REQUIRES_FALLIBLE"' src/ir/verify/resources.rs`).
`src/docs/spec/language/04_types.md` distinguishes eager integer failures from
Float observation and permits transient nonfinite intermediates
(`rg -n 'observation|intermediate' src/docs/spec/language/04_types.md`). Bug-798's
fix (commit 526600e93) lifts a Float `+`, `-`, `*`, `/`, `DIV` or negation only at a
boundary (not under another operator) as an observing `Checked`, and lifts
`MOD`/`^` in place as a capture-only `Checked` (`observe = false`, IR tag 23) for
their domain errors.

Comparisons can raise since bug-816 (commit 02bb44dc0): a `Fixed` compared with an
`Integer` or `Float` promotes into `Fixed` with a checked conversion — `ErrOverflow`
out of range, `ErrInvalidFormat` for a non-finite Float
(`rg -n 'pub\(crate\) fn comparison_can_raise' -A20 src/ir/fallible.rs`; spec
`language/04_types.md`, `rg -n 'compared with a .Fixed., it is first converted' src/docs/spec/language/04_types.md`).
Whether a comparison is a raise site therefore depends on operand types.

`&` is never fallible: the owner's 2026-10-04 exception
(`rg -n 'string concatenation\) is the one case' src/ir/fallible.rs`). Today
`LET s = a & b TRAP(e)` is rejected with `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE`
(`/tmp/mfb165-probes/d1`), contradicting rule 11's sentence that it is flagged
`TYPE_INLINE_TRAP_DEAD_HANDLER`; bug-825 corrects that sentence to the rejection.

Bug-797 (commit a5324104b) added a failure-edge census for function-level handler
locals (`src/ir/verify/trap_init.rs`). It treats any `Checked` or `CallResult` as
captured by its inline TRAP, so newly admitted operator-only traps must lower to
those forms to keep handler-local readability unchanged.

Bug-778 (commit 0bc2c56aa) keeps a trap temporary in frame slots, not a heap
`Result` block, when `function_lowering.rs:unboxed_trap_result_candidates` accepts it;
every trapped producer ends in `finish_unboxed_trap_result`
(`rg -n 'fn finish_unboxed_trap_result' src/codegen/error/result/unboxed_trap.rs`).
`tests/runtime/rt_inline_trap_success_alloc.rs` pins trapped-vs-plain allocation
parity.

Bug-820 (commit c0ef514ad) made a Float-suffixed literal in a Money slot a compile
error (`tests/syntax/types/float-suffixed-literal-into-money-invalid`) and converts
one in a Fixed slot.

## Proposed language contract

For `LET value = 1 / 2 / 3 / 4 TRAP(e)`, the protected expression is the entire
right-hand side with the language's existing precedence and associativity.

| Event | Handler behavior |
|---|---|
| Checked integer operation fails before the final operation | Enter handler immediately; skip remaining evaluation |
| Call or argument evaluation fails | Enter handler; skip later operands/calls |
| Float intermediate is nonfinite but becomes finite before observation | Continue; do not invent an intermediate trap |
| Final Float assignment/argument observation fails | Enter the eligible trap protecting that evaluation |
| Comparison promotes into Fixed and fails (out of range: `ErrOverflow`; non-finite Float, including a transient intermediate: `ErrInvalidFormat`) | Enter handler at the comparison; skip remaining evaluation |
| Float compared with Float/Integer/Byte, nonfinite operand | Continue; a comparison is not an observation boundary |
| Short-circuit right operand is unselected | Do not evaluate it or raise its potential error |
| RECOVER succeeds | Deliver its value as the complete expression result |
| Handler fails or diverges | Follow plan-165-A routing; do not commit the failed assignment |

“Observation at assignment” identifies the produced value and its delivery boundary;
it does not mean defer all failures until after evaluating the RHS. An integer
zero divisor cannot produce a provisional value for later operations. Side effects
already performed before a failure are not rolled back. For a discarded expression,
preserve its existing observation rules; discarding must not invent a Float store.

**Eligibility — the owner chose Option 1 on 2026-10-05:**

- **Option 1 (recommended): any well-typed expression** at the currently supported
  binding/assignment/bare-expression positions. A known-total expression can carry a
  provably unreachable handler, flagged `TYPE_INLINE_TRAP_DEAD_HANDLER`, consistent
  with known-total calls. Syntax legality then does not depend on the precision of
  fallibility inference. Consequence: `a & b TRAP(e)` becomes legal with a
  dead-handler warning (`&` is never fallible), and rule 11's `&` sentence — set by
  bug-825 to the rejection — is rewritten to the warning in the same change.
- **Option 2: only potentially raising operators.** An operator-rooted scrutinee is
  eligible when it contains a raising arithmetic operator (`operator_can_raise`), a
  raising unary operator, or a comparison for which
  `comparison_can_raise(op, left, right)` holds for the operand types; a comparison
  of two Integers or two Floats is not eligible. `&` is never eligible:
  `a & b TRAP(e)` stays `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE`, matching rule 11 as
  corrected by bug-825.

Neither option permits missing required checks, and neither treats `&` as fallible.

## Compiler design

1. Preserve the parser's complete-expression attachment; no precedence change.
2. Replace the runtime-call requirement in `lower_inline_trap` with the accepted
   expression eligibility rule. Capture a raising root even when nothing is hoisted;
   a raising comparison root is captured like a raising arithmetic root.
3. Use bug-798's representation for Float trees: an observing `Checked` at the
   boundary, capture-only `Checked` for in-tree `MOD`/`^`. A Float root under the
   new eligibility is a boundary. Retain plan-165-C's selected-branch evaluation
   and shared handler.
4. Produce a correctly typed success result for known-total expressions without
   evaluating the expression twice and **without a per-evaluation heap `Result`**:
   an operator-rooted trapped expression must take bug-778's unboxed-slot path
   (`unboxed_trap_result_candidates` must accept it), and a known-total one must
   allocate nothing beyond its untrapped twin. Warning/removal requires plan-165-B
   proof.
5. Update `check_inline_trap_scrutinee` for the new source contract, accepting both
   observing and capture-only `Checked`, without weakening general Result typing or
   decoded-package safety checks.
6. Keep RECOVER assignability, handler divergence, resource/state assignment and
   existing syntactic placement restrictions. Do not broaden RETURN syntax.

Risk concentrates in a root with no lifted children, Float observation timing,
assignment delivery on failure, operand-type-dependent comparison eligibility
(Option 2), and eligibility diagnostics inferred from lowered shapes. Do not encode
eligibility by the presence of a particular synthetic temp.

## Compatibility and non-goals

Previously rejected operator-only forms become valid after approval. Preserve
arithmetic values, failure codes/origins, side-effect order and result ABI. No
transactional rollback, exceptions, arbitrary statement-block trapping or changed
numeric overflow policy. `&` stays outside the fallibility census. Expected fixture
differences are the newly admitted forms and any proven-dead warnings; byte
identity is not the feature acceptance gate.

Testing assertions (`expectTrap`/`expectNTrap`) share lowering but have their own
call-only shape rule. Keep that public assertion API unchanged in this plan; ensure
its diagnostics remain covered rather than widening it accidentally.

## Existing-test evidence

`git log --follow -S 'a_bare_operator_scrutinee_is_still_rejected' --
tests/runtime/rt_inline_trap_raising_operator.rs` identifies the bug-471 regression
control. It protects the then-deliberate call-only eligibility policy, as does
bug-779's `tests/syntax/trap/inline-trap-call-free-operator-chain-invalid`
(`git log --follow -- tests/syntax/trap/inline-trap-call-free-operator-chain-invalid`).
Find all consumers with `rg -n 'TYPE_INLINE_TRAP_REQUIRES_FALLIBLE|call-free|scrutinee_has_runtime_call'
src tests -l` (2026-10-04: 27 files, including spec `language/08_error-model.md`,
`architecture/04_ir.md`, `package/08_ir-section.md`, `package/12_verifier-rules.md`,
`diagnostics/01_rule-codes.md`, `src/rules/table.rs`, `src/ir/verify/tests.rs`, and
the two runtime targets). Owner acceptance of this proposal is the independent
contract evidence for replacing those rejection assertions; until then they remain
correct.

## Phases

### Phase 1 — Specify and test the accepted boundary

- [x] Record owner acceptance and the selected eligibility option (2026-10-05,
      Option 1; recorded in this plan).
- [x] Add RED runtime cases for division failures in the first, middle and last
      positions of a chain; RECOVER must replace the entire result in every case.
      `tests/runtime/rt_inline_trap_expression_boundary.rs:a_chained_division_failure_anywhere_…`.
- [x] Cover Integer/Byte/Fixed/Money checked operations, Float final overflow,
      transient nonfinite recovery and mandatory FMA parity. Write Fixed/Money
      operands with their own literal suffixes (`1.5F`, `5.00m`) or conversions —
      never a Float-suffixed literal in a Money slot, which bug-820 made a compile
      error. `checked_integer_byte_fixed_and_money_…`, `float_observation_timing_…`.
- [x] Comparisons: Fixed vs out-of-range Integer/Float (`ErrOverflow`) and Fixed vs
      a non-finite Float, including a transient one (`ErrInvalidFormat`), each enter
      the handler; Float vs Float/Integer/Byte with a non-finite operand does not.
      Under Option 2, an Integer/Integer comparison stays
      `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE`. `comparisons_raise_only_when_…` (Option 1
      chosen: the Integer/Integer case is a dead handler, in the `&` case below).
- [x] `&`: under Option 1, `a & b TRAP(e)` builds with
      `TYPE_INLINE_TRAP_DEAD_HANDLER` and runs; under Option 2 it stays
      `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE`. Either way a function containing only
      `&` stays infallible. `a_known_total_expression_carries_a_dead_handler`
      (`a & b`, `n = 3`, `n` — three warnings; `cat` reads a local declared after
      `a & b` in its function handler).
- [x] Cover parenthesized expressions, assignment to an existing target, call
      arguments, conditional operators and discarded expressions. Assert that
      failure skips later effects and leaves the target unchanged until recovery.
      `every_position_takes_the_whole_value` (call arguments: the `note` operands).
- [x] Allocation parity (bug-778): for an operator-rooted trapped expression and a
      known-total one, per-evaluation `arena.0.alloc_calls` equals the untrapped twin
      at `-O0`, `-O1`, `-O3` (N/2N-delta method of
      `tests/runtime/rt_inline_trap_success_alloc.rs`), and the failing forms keep
      `arena.0.live_bytes 0` and `arena.0.double_free_skips 0`.
      `operator_rooted_traps_allocate_what_the_plain_expression_does`.
- [x] Preserve invalid RECOVER types, invalid handler fallthrough, forbidden trap
      positions and testing-assertion call requirements.
      `recover_typing_fallthrough_and_testing_assertions_are_unchanged` (passes at
      HEAD and must stay passing); forbidden positions stay covered by
      `tests/syntax/trap/inline-bare-trap-illegal-position`.
- [x] Assert a function-level handler may still read a local declared after an
      operator-only trapped statement (bug-797 census); declare it before any
      counted FOR (bug-816 makes every FOR a failure edge).
      `an_operator_only_trap_is_not_a_function_handler_failure_edge`.
- [x] Register the new target with a `[[test]]` entry in `Cargo.toml`, like the
      existing `tests/runtime/rt_inline_trap_*` targets.

Acceptance: proposed `cargo test --test rt_inline_trap_expression_boundary` has RED
cases for newly admitted forms and passing controls for existing contracts
(estimate: minutes after build). Met: `1 passed; 8 failed` — the control
(`recover_typing_…`) passes; the eight fail on `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE`
(26 sites) or its knock-on `TYPE_TRAP_LOCAL_UNINITIALIZED` (the census case).
Commit: c7d048193

### Phase 2 — Expression capture and spec

- [x] Implement accepted eligibility/root capture and typed total-expression success
      on the unboxed-slot path. `lower_inline_trap` lifts every scrutinee (the
      `scrutinee_has_runtime_call` gate is deleted); with nothing lifted that can
      fail, a non-call root (package constants included) is one `Checked`
      (`observe` FALSE only for a discarded `Float`), preceded by the unchecked call
      lifts a `Checked` cannot hold; `check_checked_has_no_call` exempts package
      constants; `check_inline_trap_scrutinee` warns `TYPE_INLINE_TRAP_DEAD_HANDLER`
      on a `Checked` with no raising operator; `lower_checked_value` claims a fresh
      block result on the unboxed path (parity for `a & b`). Allocation parity at
      -O0/-O1/-O3 is green for `i * 2 + 3`, `a & b`, `i / d`.
- [x] Update error-model (rule 11, including the `&` sentence for the chosen
      option), operator/Float cross-references, IR lowering and diagnostic rule
      descriptions together; include the chained-division boundary example.
      Rule 11 (any well-typed expression; `a / b / c / d` example; discarded
      `Float`; `&` → dead-handler warning); 04_ir (`Checked` node and the
      single-check shape); rules table + `01_rule-codes.md` (DEAD_HANDLER summary
      "the guarded expression cannot fail"; REQUIRES_FALLIBLE documented as
      decoded-package only).
- [x] Add valid fixtures under `tests/rt-behavior/trap` and invalid counterparts
      under `tests/syntax/trap`; revise only independently disproved expectations
      (bug-471's and bug-779's controls, with the owner's acceptance as evidence).
      bug-779's `inline-trap-call-free-operator-chain-invalid` became the valid
      `tests/rt-behavior/trap/inline-trap-call-free-operator-chain-rt` (prints the
      whole-value recoveries); bug-471's `a_bare_operator_scrutinee_is_still_rejected`
      and bug-457's `a_trapped_expression_with_no_call_is_still_rejected` became
      runtime assertions. Corrected only the disproved lines: the REQUIRES_FALLIBLE
      entries of `inline-trap-infallible-builtin-invalid` (still invalid:
      SYMBOL_NOT_CALLABLE) and `control-flow-inline-trap-invalid` (line 48), now
      the dead-handler warning; the DEAD_HANDLER summary text in
      `inline-trap-infallible-builtin-valid`, `strings-inline-trap-fallible-rt` and
      `toInt_enum`. Invalid counterparts that still hold: RECOVER typing /
      fall-through (`control-flow-inline-trap-invalid`,
      `inline-trap-conditional-recover-type-invalid`), positions
      (`inline-bare-trap-illegal-position`). Trap-family acceptance: 42 tests, the
      one mismatch was the intended summary text, now corrected.

Acceptance: the proposed runtime target proves whole-result recovery, unchanged
observation timing and allocation parity; `cargo test --test rt_inline_trap_success_alloc`
still passes; `cargo test --bin mfb spec` and the rendered error-model topic agree
(estimate: minutes after build). Met: `cargo test --test
rt_inline_trap_expression_boundary --test rt_inline_trap_success_alloc` (+ nested_call,
raising_operator, short_circuit, float_observation, error_model_recovery) → 9, 2,
21, 20, 11, 1, 4 passed; `cargo test --bin mfb -- spec` → 46 passed; the rendered
topic shows the `a / b / c / d` example; `cargo test --bin mfb -- ir:: trap spec
rules:: testing` → 1089 passed.
Commit: 30931bb43

## Validation

Use scoped runtime checks while implementing, including default and optimized
builds. Run full `cargo test --no-fail-fast` and
`scripts/test-accept.sh target/debug/mfb target/accept-error-model` before any
existing golden correction. Investigate unexpected diffs and apply the repository's
history/contract proof before changing expectations. Full gates may take hours
(estimate); they cover generated artifacts beyond the scoped runtime tests.

## Open decisions

None. The owner approved the boundary and Option 1 on 2026-10-05, so
`a & b TRAP(e)` becomes legal with `TYPE_INLINE_TRAP_DEAD_HANDLER`. This
acceptance is not implementation permission; that is a separate go-ahead.

## Corrections

Review 3.1 previously had only an open-decision paragraph in plan-165-A. This
standalone proposal supplies the missing design, code changes and verification work.

2026-10-03 review: bug-798 and bug-797 landed after this plan was written. Marked
the bug-798 prerequisite met with its regression command, replaced the stale
"reproduces an existing violation" evidence with the shipped Float representation,
named it in compiler-design steps 3 and 5, added the bug-797 census interaction and
a test for it, added `[[test]]` registration, and replaced the session-specific
authorization text with a reusable go-ahead condition.

2026-10-04 review (against 85a5f0a29, re-verified at 16df1c451): still valid as a
proposal; owner acceptance still required. Replaced drifted line ranges
(`check_inline_trap_scrutinee` moved from ~996 to 1125) with `rg` symbol commands.
Added: comparison fallibility by operand type (`comparison_can_raise`) in Option 2;
the `&` consequence of each option and its link to bug-825's rule 11 correction;
bug-816's raising Float/Fixed comparison rows; bug-778's unboxed-slot requirement
and allocation-parity tests; bug-820's literal constraint for Fixed/Money cases;
bug-779's fixture as a consumer; bug-825 as a prerequisite.

2026-10-05: owner accepted the contract with Option 1. Updated Status,
Prerequisites and Open decisions; bug-825 is now fixed, so its prerequisite is met.
Under Option 1 the Phase 1 `&` case and Phase 2's rule 11 edit follow the
dead-handler branch only.

2026-10-05 (implementation):
- A discarded `Float` expression has no observation boundary untrapped (measured:
  a bare `f * f` statement with `f = 1.0e200` does not raise at `f23af579c`), so
  the root `Checked` of a discarded `Float` is capture-only (`observe` FALSE). The
  contract's "discarding must not invent a Float store" is implemented that way.
- `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE` is not retired: source lowering no longer
  produces it, but `ir::verify` still reports a decoded package whose `$trap_res`
  is bound to a non-`Result` value (that would otherwise be a type confusion).
  Its rule row says so.
- The `TYPE_INLINE_TRAP_DEAD_HANDLER` summary read "the guarded call cannot fail";
  under Option 1 the guarded thing may be any expression, so it now reads "the
  guarded expression cannot fail". Every golden carrying it (five) was corrected
  in that one line only.
- A package-constant root (`math::pi TRAP(e)`) is a known-total expression under
  Option 1 and warns instead of erroring; `check_checked_has_no_call` exempts
  package constants because they fold to their value.
- The full suite (§5) found `diagnostics::the_syntax_corpus_reproduces_its_goldens_in_process`
  RED on the two invalid fixtures that now carry a dead-handler warning: its
  `golden_rules` read only `error[` lines, so a warning the golden records looked
  unreviewed. It now reads `warn[` lines too (the golden records them; nothing is
  skipped). `cargo test --bin mfb -- the_syntax_corpus_…` → 1 passed.
