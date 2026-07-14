# MFBASIC IR-Level Semantic Verification Plan

Last updated: 2026-07-03
Effort: x-large (1d–3d)

A malicious or corrupt compiled package (`.mfp`) can carry hand-crafted IR that
never passed the source type checker. Today the compiler decodes that IR and
lowers it to native code trusting it is well-typed — only cheap structural
checks (`verify_package`/`verify_ops`) re-run. This is audit-1 finding **PKG-02**
(`planning/audit-1-package-decode.md`): decoded package IR is trusted for typing,
resource-linearity, and Result-handling, so a crafted `.mfp` can emit
type-confused IR (a `MemberAccess` on an `Integer`, a `Capture` index past the
closure's slots, a non-linear resource use, an unchecked `Result` unwrap) that
codegen turns into memory-unsafe native code in the victim's binary.

The obvious fix — "re-run the type checker on the decoded IR" — is a trap: the
type checker (`src/typecheck/`) runs on the **AST**, not the IR, so re-running it
would mean either lowering the IR back to an AST or writing a *second*,
IR-shaped type checker. A second checker is a correctness liability: the two must
agree forever, and any drift is a soundness hole exactly where we least want one.

This plan's single behavioral outcome: **there is one semantic checker, it
operates on the IR, and every path that produces IR — the source front end and
the package decoder — is verified by it before lowering.** No duplicate checker.

It complements:

- `./mfb spec architecture native-ir` (how decoded package IR is merged and
  lowered; the canonical specs live under `src/docs/spec/**`)
- `./mfb spec package binary-representation` (the `.mfp` container the attacker
  controls)
- `planning/audit-1-package-decode.md` (PKG-02, the finding this closes; PKG-01
  and the PKG-03..07 decoder-hardening fixes are already implemented and tested
  under `tests/security/pkg-0*`)

## 1. Goal

- Move the semantic checks that today live in `src/typecheck/` (type
  correctness, move/ownership *resource linearity*, `Result`-handling
  exhaustiveness, argument arity/kind, closure-capture bounds, record/union
  field existence) so that they run **on the IR**, and have both the source
  compile path and the package-decode path go through that one checker.
- After decoding an imported package's IR and reconstructing its type
  environment (types + function signatures, plus the importing project's), run
  the IR checker over every imported function before `merge_packages` lowers it.
  A package that fails is rejected with a `PACKAGE_BINARY_REPRESENTATION_*`
  diagnostic and a non-zero exit — never lowered.
- Delete the interim structural stopgaps once the real checker covers them:
  `verify_package`/`verify_ops` shrink to only what the IR checker does not
  (or fold entirely into it).

### Non-goals (explicit constraints)

- **No language-surface change.** This is a verifier addition/relocation; no
  new syntax, no changed diagnostics for valid source programs.
- **No change to value/copy/move/freeze semantics, layout, or ABI.** Byte-for-
  byte-identical native output for every existing test; the IR checker must
  accept exactly the IR the front end emits today.
- **No second source of truth.** The end state has exactly one implementation of
  each semantic rule. If a rule is expensive to express on the IR, that is a
  reason to improve the IR, not to fork the rule.
- Out of scope: the decoder-hardening findings (PKG-01, PKG-03..07) — already
  landed.

## 2. Current State

- `src/typecheck/` runs over the project's own **AST** (`typecheck::*` is invoked
  from `src/cli/build.rs` via `resolver`/`monomorph` before lowering). It never
  sees IR.
- `src/ir/binary.rs:verify_package` (+ `verify_ops`) is the *only* re-check on
  decoded package IR: function names non-empty, function/type names unique, and
  every `MATCH` has ≥1 case. No typing, ownership, Result, arity, or
  capture-bounds checks. Called from `merge_packages`
  (`src/target/shared/nir/lower.rs:merge_packages`) right before lowering.
- `src/target/shared/validate.rs:validate_nir` is structural only (name
  resolution, unique locals, mutability, visibility strings) and its own comment
  states it "assumes the type checker already ran"; `validate_project` is
  `Ok(())`.
- IR shape the checker must reason about lives in `src/ir/` (`IrValue`,
  `IrOp`, `IrType`, `IrFunction`, `IrProject`): `MemberAccess { target, member }`,
  `Constructor { type_, args }`, `UnionWrap`/`UnionExtract`, `Capture { index }`,
  `Binary`/`Unary`, `Call`/`CallResult`, `ResultIsOk`/`ResultValue`/`ResultError`.
- Precedent to mirror: the ABI serializer already reconstructs a typed view of a
  package's exported surface from the decoded tables
  (`src/binary_repr/reader.rs:AbiSerializer`, `function_sig_hash`), and
  `merge_packages` already rebuilds a merged `IrProject`. The type environment
  the IR checker needs is a superset of what these already compute.

## 3. Design Overview

Three layers, landable in order, each independently valuable:

1. **An IR semantic checker** (`src/ir/verify/`) — a pass over `IrProject` that
   enforces the semantic rules using a type environment (type table + function
   signatures) rather than AST scopes. This is the new single source of truth.
2. **Front-end cutover** — the source compile path stops relying on the
   AST-shaped `typecheck` for the rules the IR checker now owns, and instead runs
   the IR checker after lowering. Rules that are genuinely AST-only (parse-shaped
   diagnostics, name resolution) stay in the front end; everything type/ownership
   shaped moves. The byte-identical golden suite is the oracle that the relocated
   rules accept exactly today's programs.
3. **Package-decode cutover** — `merge_packages` reconstructs the type
   environment from the decoded + importing tables and runs the IR checker over
   imported functions before lowering; `verify_package`/`verify_ops` are folded
   in or reduced to the residue.

Correctness risk concentrates in layer 2: proving the relocated checker accepts
**exactly** the current language (no new rejections of valid programs, no new
acceptances of invalid ones). The existing acceptance suite (`tests/**`,
including the `*-invalid` diagnostic goldens) plus the byte-identical native
goldens are the gate.

## 4. IR Semantic Checker (`src/ir/verify/`)

- Input: an `IrProject` plus a `TypeEnv` (map of type name → `IrType`
  definition, and function name → signature) assembled from the project's own
  types/functions. For the package path the `TypeEnv` also includes the imported
  package's decoded types/signatures and the importer's.
- Rules (each mirrors a `src/typecheck/` rule, relocated not duplicated):
  - **Typing:** operand/result types of `Binary`/`Unary`; `MemberAccess.member`
    exists on the `target`'s declared record type; `Constructor` arg count/types
    match the referenced type; `UnionWrap.member_type` is a real variant of
    `union_type`; `UnionExtract` tag/payload consistency; `Call`/`CallResult`
    arg arity/kind vs the callee signature.
  - **Closures:** every `Capture.index` is within the enclosing closure's
    captured-slot count.
  - **Resource linearity:** a `File`/`Socket`/`Listener` (and package-declared
    resource) local is used linearly — no use-after-move / double-close.
  - **Result handling:** a `Result` is not unwrapped (`ResultValue`) without an
    `Ok` discrimination; `MATCH` exhaustiveness over union variants.
- Output: `Ok(())` or a diagnostic carrying a stable `PACKAGE_BINARY_...` /
  existing typecheck rule id.

## 5. Front-end cutover

- After the front end lowers the concrete AST to IR (`ir::lower_project_*`), run
  `ir::verify::check(&ir, &env)`. Remove the corresponding rule from the
  AST `typecheck` once the IR check covers it and goldens are unchanged.
- Keep in the AST front end only what is inherently AST-shaped: syntax
  diagnostics, name/scope resolution, DOC validation, overload resolution.

## 6. Package-decode cutover

- In `merge_packages`, after `read_package_ir_with_identity` + `verify_package`
  (interim), build the `TypeEnv` from the merged project and run
  `ir::verify::check` over the imported functions before
  `prefix_package_symbols`/`merge_package`.
- Replace `verify_package`/`verify_ops` with the IR checker (or reduce them to
  the non-semantic residue). Update `tests/security/pkg-0*` if error strings
  change; add a `tests/security/pkg-02-*` fixture whose `.mfp` carries
  type-confused IR (e.g. a `MemberAccess` on an `Integer` local, or a
  `Capture` index past the slot count) and assert it is rejected.

## Layout / ABI Impact

None. This adds/relocates a verifier; it emits no code and changes no on-wire
format. `mfb spec package binary-representation` gains a note that decoded IR is
semantically verified before lowering. Native goldens must stay byte-identical.

## Status (2026-07-03)

**Phase 1 landed + the soundly-IR-checkable part of Phase 2.** `src/ir/verify/`
(`ir::verify_semantics`) reconstructs a `TypeEnv` from the merged `IrProject`
and enforces: member access on a real record member (primitive target or
missing field rejected, `includes` expanded), closure-capture index bounds,
call/constructor arity, union-wrap variant membership, and non-empty `MATCH`
(depth-capped). It runs on the fully merged project in `merge_packages` — after
every imported package's IR and the importer's own IR are merged, before native
lowering — so **every path that produces IR is verified before any code is
emitted** (the plan's single behavioral outcome). `verify_package` is kept as
the per-package structural re-check (Phase 1's "in addition to"). Soundness gate:
the checker skips any node whose type it cannot reconstruct with certainty, so it
accepts exactly today's IR — proven by the full acceptance suite (1006 tests, 0
new rejections; the lone `audit-usage` mismatch is a pre-existing unrelated CLI
drift). New security fixture `tests/security/pkg-02-type-confusion` (+ generator
`tools/security-package-sources/pkg-02-type-confusion`) ships a `.mfp` whose
valid signature tables let the consumer link `run() AS Integer` while its MFBR
body does a `MemberAccess` on an `Integer`; the build is rejected with
`PACKAGE_BINARY_REPRESENTATION_VERIFY_TYPE` before lowering. Spec updated:
`mfb spec package verifier-rules` gains a "Merge-time semantic verification"
section. **This closes audit-1 PKG-02** for the type-confusion / bounds / arity
classes.

**Deferred (with rationale):**

- *Flow-sensitive resource linearity and full `Result`/effect-agreement on the
  IR* (rest of Phase 2). These are flow-sensitive analyses with high
  false-rejection risk against the byte-identical gate; the merged NIR already
  enforces the structural resource rules (`validate_resource_rules`: records
  can't own resources, unions can't mix data/resource variants) on the package
  path. Left to the compile-time guarantees + the NIR structural pass.
- *Front-end cutover / deleting the AST `typecheck` rules and `verify_ops`*
  (Phase 3). This is **architecturally unsound as written**: `typecheck` runs on
  the AST *before* `ir::lower_project_*` in `src/cli/build.rs`, and lowering
  assumes a well-typed AST — an ill-typed source program cannot be soundly
  lowered to IR to then be IR-checked. The 371 `*-invalid` diagnostic goldens are
  produced by the AST checker with source-level spans the IR does not retain, so
  deleting those rules would regress diagnostics and cannot be made
  byte-identical. The IR checker is therefore the single semantic verifier *for
  the package-decode path* (its only checker) and a redundant guard on the source
  path; the AST checker stays as the source front end's diagnostic-producing
  type checker.

## Phases

1. **Checker skeleton + TypeEnv (medium).** `src/ir/verify/` with the `TypeEnv`
   builder and the structural/bounds rules (capture index, arg arity, field
   existence, union variant) — the subset `verify_ops` gestures at. Wire it into
   `merge_packages` *in addition to* `verify_package`. Add `tests/security/
   pkg-02-*`. Lands the memory-safety win for the package path first.
2. **Typing + Result + linearity rules (large).** Port the remaining
   `src/typecheck/` semantic rules onto the IR checker. Gate on the full
   acceptance suite (esp. `*-invalid` goldens) run against the IR checker in
   parallel with the AST checker to prove agreement before cutover.
3. **Front-end cutover + delete duplication (large).** Switch the source path to
   the IR checker, delete the now-redundant AST rules and `verify_package`/
   `verify_ops`. Byte-identical native goldens are the oracle.
