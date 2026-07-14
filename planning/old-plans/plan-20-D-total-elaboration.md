# plan-20-D — Total, panic-free elaboration

Last updated: 2026-07-04
Effort: medium
Depends on: plan-20-B (typed IR); lands after plan-20-C
Parent: planning/plan-20-typed-ir-single-checker.md

## Goal

Make IR lowering **total**: it must never panic on ill-typed input. Today the
25 `.expect("typecheck requires …")` sites in `src/ir/lower.rs` rely on the AST
type checker having rejected ill-typed programs before lowering runs. For the
checker to move *after* elaboration (20-Z), lowering must instead stamp the
explicit `Unknown` marker and proceed. This is a safe, byte-identical change
today (on valid programs the fallbacks never execute, because every Option is
`Some`), and it is the safety net the cutover depends on.

## Scope decision (vs. master §4.2 "merge the two inference engines")

The master plan says elaboration "absorbs both `typecheck/inference.rs` and
`lower.rs:expression_type`." That engine-*merge* (deleting one inference
implementation in favor of a shared one) is an internal dedup, not a
behavioral requirement of the single-*checker* outcome: the typed IR is already
produced (20-B), and the checker consumes annotated types, not a live inference
pass. Merging the engines is deferred to 20-Z (or left as follow-up), where
`typecheck/` is reduced to elaboration-only and the duplication is removed in
one place. 20-D delivers the *behavioral* requirement — total, panic-free
lowering — which is what unblocks the cutover. Recorded here so the deferral is
explicit, not silent.

## Tasks

1. Convert each of the 25 `.expect("typecheck requires …")` sites to a total
   branch. The type-name lookups (the majority) default to `"Unknown"`; the
   handful of structural ones default to a harmless placeholder:
   - PROPAGATE outside a trap (`trap_name` is `None`) → a sentinel error local.
   - RECOVER with no inline-trap target → a discard target
     (`RecoverTarget { slot: None, type_: "Unknown" }`).
   - `error(code, message)` missing args → `Unknown`-typed const placeholders.
   Each fallback is unreachable for programs typecheck accepts, so native output
   stays byte-identical.
2. Add a lowering-totality test: lower every `*-invalid` fixture's resolved AST
   directly (bypassing the typecheck gate) and assert no panic. This is the
   acceptance for 20-D and the regression net for 20-Z.

## Acceptance

- Native goldens byte-identical (G5) — the fallbacks are dead code on valid
  programs.
- The new totality test passes: no `*-invalid` fixture panics during lowering.
- Full acceptance green (modulo pre-existing audit-usage drift).

## Commit

`refactor(ir): total panic-free IR lowering (plan-20-D)`.
