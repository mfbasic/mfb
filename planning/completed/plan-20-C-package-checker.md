# plan-20-C — Complete the package-path checker (the early security win)

Last updated: 2026-07-04
Effort: small/medium
Depends on: plan-20-B (types serialized in the .mfp)
Parent: planning/plan-20-typed-ir-single-checker.md

## Goal

With types serialized (20-B), close the residual PKG-02 gap on the package
path: reject member-confusion on **computed** value targets, not just on the
locals/constructors the old subset checker could type. This is the bankable
pause point — if the census relocation (20-E..I) is deferred, the security
goal has still landed.

## What shipped

- **`infer_type` totalized** (`src/ir/verify/mod.rs`). It now resolves the
  type of every computed node via `IrValue::annotated_type()` (added in 20-B),
  filtering the explicit `"Unknown"` marker through a new `usable_type` helper
  so an unresolved type never forces a rejection. Previously it had an explicit
  `_ => None` arm covering `Call`/`CallResult`/`Binary`/`Unary`/`ResultValue`,
  so a member access on any of those results was silently skipped. Now
  `(a + b).x`, `f().x`, etc. on a primitive result are caught.
- **G6 fixture `pkg-02b-computed-confusion`**: a crafted package whose `run`
  body is `Return(MemberAccess(Binary("+",1,2), "x"))` — the confused target is
  a `Binary` Integer result, which the pre-20 checker's `_ => None` skipped.
  Generator `mutate_type_confusion_computed` in `mfp_craft.py`; tool source
  under `tools/security-package-sources/pkg-02b-computed-confusion/`. The build
  fails with `PACKAGE_BINARY_REPRESENTATION_VERIFY_TYPE: member \`x\` accessed
  on an \`Integer\` value`.

## Scope decision (logged fork vs. the master plan's G6 list)

The master §5 G6 lists five type-relational classes (member, operand,
arg/param, return, use-after-move). Only **member confusion** is completed
here. The other four require a type-**compatibility** relation (numeric
widening, union-variant subsumption, `Result` wrapping, borrow tracking). The
sound way to get that relation is to relocate the front end's *exact* algebra
(`typecheck/types.rs:compatible` + the resource linearity pass), which is
precisely the 20-E..I census port. Re-deriving an approximate compatibility
check here would risk **false rejections on the source path** (the checker runs
in `merge_packages` for source builds too, so any over-eager rule breaks the
whole 1006-test suite), for a class the source path already rejects via
typecheck until 20-Z. So: member confusion now (zero false-positive risk —
it only strengthens an existing sound rule); operand/arg/return/use-after-move
land with their owning rule families in E..I, at which point their package-path
fixtures join the pkg-02b battery. The master plan's G6 wording is updated to
reflect this staging.

## Acceptance

- pkg-02b rejects at the verify stage (shown).
- Full acceptance green except the pre-existing audit-usage drift — the
  totalized `infer_type` must produce **zero** new rejections on valid programs
  (the false-positive gate; run recorded).
- No native golden changes (checker is metadata-only).

## Commit

`feat(ir)+sec: complete member-confusion check on typed package IR (plan-20-C)`.
