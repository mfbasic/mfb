# bug-301: semantic-analysis LOW cluster (bare imported type names, allow_sub_call leak, dead check_link_function, thread STATE sendability)

Last updated: 2026-07-17
Effort: small (<1h across items)
Severity: LOW
Class: Correctness / Dead-code

Status: Open
Regression Test: per-item

LOW-severity resolver / verify / syntaxcheck residuals found during goal-06.
Distinct root causes, one document per the repo's low-cluster convention. Several are
latent (no current trigger); noted as such.

References:

- Found during goal-06 review of `src/resolver/**`, `src/ir/verify/mod.rs`,
  `src/syntaxcheck/**`.

## Items

### G1 — imported binary-package type names are accepted as *bare* (unqualified) references
- `src/resolver/packages.rs:78-83` (`install_package_type_names`).
- `install_package_type_names` inserts each `.mfp` export's type name (and variant
  names) into the flat `self.types` set with no package qualification. Bare names are
  consumed by `resolve_type_name`'s bare arm; a qualified `pkg.Type` routes through
  `resolve_package_qualified_name`, which for a non-builtin import returns without
  validation. So a bare `Point` for an imported type is accepted, contrary to the
  architecture spec (imported symbols are `packageName.exportName`), and IR merge only
  registers the prefixed `pkg.Point` — so the bare declared type fails to resolve
  later (deferred/absent diagnostic) instead of a clean resolve-time
  `SYMBOL_UNKNOWN_TYPE`. Only *types* are bare-injected (never functions), and only
  for installed `.mfp` (not source packages) — the asymmetry marks it incidental.
  POSSIBLE (no `.mfp` type-exporting fixture available to repro).
- Fix: qualify inserted names (`format!("{pkg}.{name}")`) so only qualified references
  resolve, or drop the insertion and validate qualified imported-type references
  against the package exports — applied consistently to the source-package path too.

### G2 — `allow_sub_call` leaks into the first nested call, silently accepting a value-less SUB call under an operator/constructor
- `src/ir/verify/mod.rs:1457` (`check_value_depth`, Call/CallResult arm) with `:1048`
  (`IrOp::Eval` sets `allow_sub_call`).
- `allow_sub_call` is a single `Cell` set true for a statement-position value and
  consumed by the first Call node checked. But `check_value_depth` recurses into
  operands/args before the wrapping node's own rule, so for `Eval(Binary(Local a,
  Call(sub)))` the DFS reaches the nested SUB call while the flag is still true,
  marking it statement-position and skipping `TYPE_SUB_HAS_NO_VALUE`. Intent (comment
  at `:503-505`) is that only the top-level statement call may be value-less. Many
  shapes are masked by the adjacent operand type rule seeing `Nothing`
  (`a + doEffect()` still fails `TYPE_BINARY_OPERATOR_MISMATCH`), limiting impact.
  LIKELY. `TYPE_SUB_HAS_NO_VALUE` is in `RELOCATED_TO_IR_VERIFY`, so verify is the
  sole rejecter on the source path.
- Fix: snapshot-and-reset `allow_sub_call` at the top of `check_value_depth` for
  non-Call nodes (so only a value whose root is the call sees it), or pass
  statement-position as an explicit parameter rather than a shared Cell.

### G3 — `check_link_function` is dead code
- `src/syntaxcheck/mod.rs:688` (`check_link_function`).
- `pub(super) fn check_link_function` wraps `check_link_function_in(file, function,
  &[])`; the live path calls `check_link_function_in` directly from `check_link_block`
  with the real cstruct list. A repo-wide grep for `.check_link_function(` returns
  zero hits; `pub(super)` keeps the dead-code lint silent. CONFIRMED.
- Fix: delete `check_link_function` (688-694); callers already use
  `check_link_function_in`.

### G4 — thread resource-plane `STATE` payload is not sendability-checked
- `src/syntaxcheck/mod.rs:2308` (`check_type_reference`, Thread/ThreadWorker arm) and
  `src/syntaxcheck/resources.rs:413` (`check_thread_boundary_sendability`).
- For a `Thread(message, resource, resource_state, output)` the checker runs
  `require_thread_sendable_type` on `message`, `output`, and `resource`, but
  `resource_state` gets only an existence walk — no sendability requirement; the
  transfer/accept boundary check likewise omits it. `ir::verify` constrains a STATE
  type only to be copyable + defaultable (`TYPE_STATE_INVALID`), not sendable. A record
  STATE such as `TYPE S { files AS List OF RES File }` is copyable and defaultable yet
  not sendable, so it could cross a thread carrying resource borrows to sender-owned
  resources, contravening §15.6. POSSIBLE (latent; needs a package-level
  stateful-plane setup to repro).
- Fix: add `require_thread_sendable_type(..., resource_state)` in both
  `check_type_reference` and `check_thread_boundary_sendability`; or confirm STATE is
  barred from containing `RES` members and document the reliance.

## Goal

- G1/G2/G4 close leniency/latent-soundness gaps with a targeted check; G3 removes dead
  code.

### Non-goals (must NOT change)

- Valid programs (G1/G2/G4 must not reject currently-accepted correct programs).
- The relocated-rules split (G2 keeps verify as the rejecter).

## Blast Radius

Each item is a single cited site. G2/G4 are latent; G3 is pure dead-code; G1 is a
resolve-time leniency.

## Fix Design / Phases

- [ ] Phase 1: unit tests for G2 (SUB-under-operator reject) and G4 (unsendable STATE
      reject) where constructible; a resolver test for G1 if a type-exporting `.mfp`
      fixture is added.
- [ ] Phase 2: apply per-item fixes; delete G3.
- [ ] Phase 3: full `cargo test` green; no valid-program regressions.

## Validation Plan

- Regression: per item as above.
- Doc sync: architecture/03_packages.md (G1), language/15-16 (G4) if behavior
  tightens.

## Summary

Four semantic-analysis residuals; three are small targeted checks (two latent) and
one is a dead-method deletion. No active miscompile; value is closing leniency and
latent-soundness corners.
