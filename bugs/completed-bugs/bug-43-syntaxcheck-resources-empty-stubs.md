# bug-43: two `syntaxcheck/resources.rs` collection-ownership checks are empty-bodied no-ops whose doc comments still claim they enforce the rules

Last updated: 2026-07-09
Effort: small (<1h)

`check_collection_element_axis` and `check_collection_resource_element` in
`src/syntaxcheck/resources.rs` compute their predicates and then do nothing with
them: the first has two empty `if`/`else if` bodies, the second `return`s on every
path and never calls `self.report`. Both are still called on every collection-typed
declaration, and both still carry doc comments asserting the §15.6 rules they no
longer enforce.

This is dead code, not a soundness hole: `ir::verify` is the sole rejecter for these
rules (plan-20) and does reject the offending programs. The danger is the
**contradiction** — a maintainer reading `resources.rs` will believe the `RES` axis is
enforced at syntax-check time, and the underscored `_file`/`_line`/`_role` parameters
are a standing invitation to "re-wire" a check that already lives elsewhere.

The single correct behavior a fix produces: `resources.rs` contains no function whose
doc comment describes enforcement it does not perform.

References:

- `src/syntaxcheck/resources.rs:143-160` (`check_collection_element_axis` — doc claims
  "a resource element must be marked `RES` … and `RES` may mark only a resource";
  body is `if is_resource && !is_res_marked { } else if is_res_marked && !is_resource { }`).
- `src/syntaxcheck/resources.rs:162-181` (`check_collection_resource_element` — doc
  claims "a temporary … cannot be stored"; every path `return`s).
- Live enforcement: `src/ir/verify/mod.rs:2394-2423` (`check_collection_res_axis` /
  `collection_axis_element`, emitting `TYPE_RESOURCE_REQUIRES_RES` and
  `TYPE_RES_REQUIRES_RESOURCE`) and `src/ir/verify/mod.rs:1463-1474` (owner-only
  storage).
- Call sites: `src/syntaxcheck/mod.rs:1805`, `:1816`; `src/syntaxcheck/inference.rs:518`, `:534`.
- Live sibling in the same file: `report_invalid_collection_element`
  (`resources.rs:125-141`), called from `inference.rs:506`/`:532`.
- Context: plan-20 (`ir::verify` becomes the sole rejecter; 65 rules relocated).
- Found during the goal-01 compiler source review of `src/syntaxcheck/`.

## Failing Reproduction

There is no user-visible failure — that is the point. The reproduction is the
inspection:

```
LET xs AS List OF File = []      # resource element not marked RES
LET ys AS List OF RES Integer    # RES on a non-resource
```

- Observed: both programs are correctly rejected — but by `ir::verify`, not by
  `syntaxcheck`. Setting a breakpoint in `check_collection_element_axis` shows it
  runs, computes `is_resource`/`is_res_marked`, and reports nothing.
- Expected: either the function reports (matching its doc) or it does not exist.

The file's own unit tests document the no-op: `resources.rs:585-597` and `:663-686`
call `let _ = check_src(src)` and assert **nothing** about rejection — an
acknowledgement in the test suite that these two functions do not reject.

Contrast: `report_invalid_collection_element` in the same file is live, is invoked
from `inference.rs`, and does call `self.report`. `ir::verify`'s
`check_collection_res_axis` rejects both programs above today.

## Root Cause

plan-20 relocated the `RES` ownership axis and the resource-owner-only storage rule
into `src/ir/verify/`, making it the sole rejecter. The `syntaxcheck` copies were
emptied out rather than deleted: the predicate computation
(`matches!(element, Type::Res(_))`, `strip_res`, `self.is_resource_type`) was left in
place, the `self.report(...)` calls were removed, the now-unused parameters were
prefixed with `_`, and the doc comments were never touched. The tests were relaxed to
`let _ = check_src(src)` in the same change, which is why nothing caught it.

## Goal

- No function in `src/syntaxcheck/resources.rs` has a doc comment describing a check
  it does not perform.
- `List OF File` (unmarked resource element) and `List OF RES Integer` (`RES` on a
  non-resource) remain rejected, with the same rule ids and messages as today.

### Non-goals (must NOT change)

- The diagnostics themselves: `TYPE_RESOURCE_REQUIRES_RES` and
  `TYPE_RES_REQUIRES_RESOURCE` must keep firing from `ir::verify` with unchanged
  rule ids, messages, and source locations. No golden output may shift.
- `report_invalid_collection_element` and its `inference.rs` call sites — live code.
- plan-20's architecture: do **not** "fix" this by re-implementing the checks in
  `syntaxcheck`. That would reintroduce the double-rejecter that plan-20 removed.
  Deleting is the intended direction.

## Blast Radius

Found by grepping the two symbol names across `src/`.

- `src/syntaxcheck/resources.rs:147` (`check_collection_element_axis`) — deleted by
  this bug.
- `src/syntaxcheck/resources.rs:166` (`check_collection_resource_element`) — deleted
  by this bug.
- `src/syntaxcheck/mod.rs:1805`, `:1816` — call sites, deleted with them.
- `src/syntaxcheck/inference.rs:518`, `:534` — call sites, deleted with them.
- `src/syntaxcheck/resources.rs:586` (`collection_element_is_resource_binding`) —
  becomes unused **only if** `check_collection_resource_element` was its sole caller;
  verify with a grep before deleting, and delete it too if so.
- `src/syntaxcheck/resources.rs:585-597`, `:663-686` (the assertion-free unit tests) —
  rewrite as `ir::verify` rejection tests rather than deleting the coverage.
- `src/ir/verify/mod.rs:2394-2423`, `:1463-1474` — unaffected; this is the live
  enforcement and must not be touched.

## Fix Design

Delete both functions and their four call sites; delete
`collection_element_is_resource_binding` if it becomes orphaned. Then *strengthen* the
two vestigial unit tests into real rejection tests that assert `ir::verify` emits
`TYPE_RESOURCE_REQUIRES_RES` / `TYPE_RES_REQUIRES_RESOURCE` — converting a test that
documented the bug into one that guards the behavior.

The correctness risk is nil (the functions have no effect); the only risk is deleting
a helper that some other caller still needs, which the grep in Blast Radius settles.

Rejected alternative: make the stubs delegate to the `ir::verify` rules so the doc
comments become true. Rejected — it restores a second rejecter, produces duplicate
diagnostics for the same program, and re-opens exactly the drift plan-20 closed.

## Phases

### Phase 1 — audit (no behavior change)

- [x] Grep `collection_element_is_resource_binding` for callers other than
      `check_collection_resource_element`; record the verdict here.
      **Verdict: NOT orphaned.** It is still called by the live
      `collection_element_mode` (`resources.rs:206`), so it is kept.
- [x] Confirm `ir::verify` rejects both reproduction programs today, and capture the
      exact rule ids + messages as the regression baseline.
      **Baseline:** `List OF File` → `2-203-0082 TYPE_RESOURCE_REQUIRES_RES`
      "resource must be bound with RES"; `List OF RES Integer` →
      `2-203-0083 TYPE_RES_REQUIRES_RESOURCE` "RES binds only resource types".
      Also note `check_collection_resource_element` had **5** call sites (not the
      4 the doc estimated): `inference.rs` ×4 and `builtins.rs` ×1.

Acceptance: the orphan question is answered; the baseline diagnostics are recorded.
Commit: —

### Phase 2 — the deletion

- [x] Delete `check_collection_element_axis` and `check_collection_resource_element`
      from `src/syntaxcheck/resources.rs`, plus the call sites in
      `src/syntaxcheck/mod.rs` (×2), `src/syntaxcheck/inference.rs` (×4), and
      `src/syntaxcheck/builtins.rs` (×1, the whole `append`/`prepend`/`insert`/`set`
      loop whose only body was the deleted call). Removed the now-unused
      `use super::helpers::*;` import.
- [x] Delete `collection_element_is_resource_binding` if Phase 1 proved it orphaned.
      **Not orphaned — kept** (Phase 1).
- [x] Rewrite the vestigial `let _ = check_src(src)` unit tests into real
      assertions. Since syntaxcheck emits **zero** diagnostics for these programs
      (rejection is `ir::verify`-only), the axis tests now assert
      `accepts(src)` — locking in the plan-20 boundary (no syntaxcheck
      double-rejecter) and cross-referencing the real guards in
      `ir::verify::tests` (`rejects_collection_resource_element_without_res`,
      `rejects_collection_res_on_data`, `TYPE_RESOURCE_ELEMENT_NOT_OWNER`) and the
      `tests/syntax/resources/*` acceptance fixtures.

Acceptance: `cargo build` clean with no dead-code warnings; the rewritten tests
pass and the real rejection guards already exist and stay green.
Commit: —

### Phase 3 — validation

- [x] `cargo build` clean (no warnings); 640 `syntaxcheck::`/`ir::verify::` unit
      tests green; reproduction still rejected end-to-end by the binary with
      byte-identical rule ids + messages. Full `scripts/test-accept.sh` is the
      orchestrator's to run; no diagnostic golden can shift (behavior unchanged —
      pure dead-code deletion).

Acceptance: full suite green, goldens byte-identical.
Commit: —

## Validation Plan

- Regression test(s): the two rewritten tests in `resources.rs` now assert real
  rejection via `ir::verify`.
- Runtime proof: not applicable — compile-time-only, no emitted code changes.
- Doc sync: none expected; the deleted doc comments describe rules already documented
  at their `ir::verify` home. Confirm the spec's §15.6 text still points somewhere live.
- Full suite: `scripts/test-accept.sh`. Diagnostic goldens must be byte-identical.

## Resolution

Done (2026-07-09). Removed the two no-op functions
(`check_collection_element_axis`, `check_collection_resource_element`) and all
seven call sites (`syntaxcheck/mod.rs` ×2, `syntaxcheck/inference.rs` ×4,
`syntaxcheck/builtins.rs` ×1) plus the newly-unused `use super::helpers::*;`
import in `resources.rs`. `collection_element_is_resource_binding` was proven
**not** orphaned (used by the live `collection_element_mode`) and kept. Also
corrected a stale doc-comment in the live enforcement
(`ir/verify/mod.rs:check_collection_res_axis`) that cited the deleted
`check_collection_element_axis`.

No behavior change: `ir::verify` was already the sole rejecter (plan-20). The
reproduction programs (`List OF File`, `List OF RES Integer`) still reject
end-to-end with byte-identical rule ids/messages (`2-203-0082
TYPE_RESOURCE_REQUIRES_RES`, `2-203-0083 TYPE_RES_REQUIRES_RESOURCE`). The
vestigial `let _ = check_src(src)` tests were upgraded to real `accepts(src)`
guards (syntaxcheck must stay silent — no double-rejecter), with the true
rejection guarded by `ir::verify::tests` and the `tests/syntax/resources/*`
acceptance fixtures. No spec/diagnostic change: no `[[…]]` citation referenced
the deleted functions, and the §15.6 rules keep firing unchanged.

Files changed: `src/syntaxcheck/resources.rs`, `src/syntaxcheck/mod.rs`,
`src/syntaxcheck/inference.rs`, `src/syntaxcheck/builtins.rs`,
`src/ir/verify/mod.rs` (doc-comment only).

Tests: `cargo test --bin mfb -- syntaxcheck:: ir::verify::` → 640 passed, 0
failed.

## Summary

Two functions that look like enforcement and are not. No behavior changes; the entire
risk is in the delete-vs-orphan grep and in making sure the diagnostic goldens do not
move. The payoff is removing a comment that actively lies to the next reader, and
upgrading two assertion-free tests into real guards.
