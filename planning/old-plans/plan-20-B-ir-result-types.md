# plan-20-B — Result types on every computed IR value node

Last updated: 2026-07-03
Effort: medium
Depends on: plan-20-A (spans, format v3)
Parent: planning/plan-20-typed-ir-single-checker.md

## Goal

Every computed `IrValue` node carries its result type so the IR checker can
apply type-relational rules without inference, and the serialized `.mfp` is
fully typed (completing the package-path checker becomes possible — 20-C).

## Design

- Add `type_: String` to: `Call`, `CallResult`, `Binary`, `Unary`,
  `MemberAccess`, `ResultIsOk`, `ResultValue`, `ResultError`
  (`src/ir/value.rs`). Type-name strings are the same canonical forms used
  everywhere (`"List OF Integer"`, …).
- `Local`/`Global` stay unannotated by design (master plan §4.1): their types
  come from the enclosing `Bind`/param/`IrBinding` via environment lookup.
- Lowering stamps the type at construction using the existing
  `expression_type` engine (`lower.rs`); where lowering synthesizes nodes with
  a known type (temp locals, result plumbing), stamp that known type.
  A node whose type cannot be named stamps `"Unknown"` — on today's valid
  programs this must not occur for user-reachable nodes (invalid programs are
  rejected by typecheck before lowering until 20-Z).
- `encode_value`/`decode_value` gain the field (type-string written after the
  existing per-variant fields, before the trailing `loc` where present).
  `BINARY_REPR_VERSION` stays 3 (v3 == the plan-20 format; it has not shipped
  between sub-plans — goldens regenerate again).
- JSON projection: the 8 variants gain `"type"` in the `-ir` dump.

## Acceptance

- `-ir` dump shows a type on every computed node; grep a sample dump for
  `"kind": "call"` entries lacking `"type"` → none.
- Native goldens byte-identical (G5); `.ir`/`.hex`/`.mfp` regenerate.
- Full acceptance green (modulo the pre-existing audit-usage drift).
- Spec: `08_ir-section.md` expression section notes per-node result types.

## Execution record (2026-07-03)

- Fields landed: `type_` on `Call`/`CallResult`/`Binary`/`Unary`/
  `MemberAccess`/`ResultValue` (NOT on `ResultIsOk`/`ResultError`, which are
  implicitly `Boolean`/`Error` — `IrValue::annotated_type()` encodes this).
  Codec writes the type string before the trailing `loc`.
- Stamping: at construction in `lower.rs` via `expression_type` on the source
  expression (Call/Binary/Unary/MemberAccess arms) and locally-known types for
  synthesized plumbing (inline-trap `CallResult`/`ResultValue` = success type;
  match-case `ResultValue` = the `Result`'s success type).
- Spot-check: `x + y` (Byte+Byte) stamps `Byte`, the enclosing `- z` stamps
  `Integer`, `f + g` stamps `Fixed` — numeric-promotion-faithful.
- All `.mfp` fixtures regenerated a second time (format changed again within
  v3; both changes land in one commit so no v3 ambiguity ships).
- `mfp_craft.py` pkg-02 crafted body updated: `MemberAccess` carries a lying
  `type_: "Integer"` — exactly the claim 20-C's checker must not trust.

## Commit

Combined with 20-A as one commit (both are the pre-release format-v3
representation change; committing A alone would have shipped an intermediate
v3 byte layout that B immediately invalidated):
`feat(ir): typed, spanned IR — Binary Representation v3 (plan-20-A/B)`.
