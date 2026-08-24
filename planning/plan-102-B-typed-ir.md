# plan-102-B: Typed IR (String → ParameterType below the AST→IR boundary)

Last updated: 2026-08-23
Effort: x-large (1d–3d) — re-measure and split into parts at kickoff
Depends on: plan-102-A (the `Copy` interner and complete type vocabulary must
already exist; without it, per-node `ParameterType` leaks).

Flip the IR's type fields from `String` to `ParameterType` and convert type
strings to `ParameterType` **once**, at the AST→IR lowering boundary
(`ir::lower_augmented_project`). Every IR consumer below that point (codegen,
`ir::verify`, `binary_repr`) reads `ParameterType`; only the wire-format
serializers render back to strings via `name()`. This is the "IR-only cut": it
delivers a typed IR and removes the downstream string compares/allocations from
codegen, without touching the AST or monomorph yet. (monomorph is still
string-based on the AST after this sub-plan — that is plan-102-D/E.)

See plan-102-A §3 for the full layering, the byte-identity gate, and the roadmap.

References:

- `src/ir/types.rs`, `src/ir/value.rs` — the IR type fields being flipped.
- `src/ir/lower.rs` (`lower_augmented_project`, `src/ir/lower.rs:140`) — the single
  AST→IR conversion boundary.
- `src/ir/binary.rs`, `src/ir/json.rs` — the wire serializers that must keep
  emitting type *strings*.
- `.ai/testing-gates.md`, `.ai/codegen-invariants.md`.

## Prerequisites

See plan-102-A §Prerequisites (shared). Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-102-A complete | `rg 'Box::leak' src/types.rs` → empty; `MapEntryOf`/`ResultOf` exist | NOT MET until A lands |

## 1. Goal

- The IR's type fields are `ParameterType`, not `String`: `IrType`/`IrParam`/
  `IrField`/`IrBinding`/`IrFunction.returns`/`ExternalFunctionParam` and the 17
  `type_: String` fields of `IrValue`.
- `ir::lower_augmented_project` converts each source type string to
  `ParameterType` exactly once (via `ParameterType::parse`); nothing below re-parses
  a type string.
- The `.mfp`/IR wire format is byte-identical: `ir/binary.rs`/`ir/json.rs` call
  `name()` at the serialize seam, so the emitted bytes are unchanged.

### Non-goals (explicit constraints)

- No change to compiled output — byte-identical `.ncode`/`.ncodesum` (this is a
  representation swap on an already-post-monomorph, already-concrete IR).
- No change to the IR JSON/binary artifact bytes (wire format stays string).
- Do not touch the AST or monomorph — `ir::lower` still consumes the post-monomorph
  concrete AST (strings) and converts at the boundary. (Switching `ir::lower` to
  consume HIR is plan-102-C.)

## 2. Current State

`ir::lower_augmented_project` (`src/ir/lower.rs:140`) is called on the
post-monomorph concrete AST (`src/ir/lower.rs:141` says so explicitly) and builds
IR whose type fields are `String`. Codegen, `ir::verify`, and `binary_repr` all
read those `String`s, re-comparing and re-allocating type names repeatedly. The IR
has **no** generic/`Var`/template notion (verified: `rg 'template|generic|Var\b|
type_param' src/ir/types.rs` → 0 matches) — it is purely concrete, which is why
retyping it does not require any of the HIR machinery.

### Measured populations

| What | Count | Command |
|---|---|---|
| String-typed type fields in `ir/types.rs` | 8 | `rg -n 'type_: String\|returns: String\|kind: String\|type_name: String' src/ir/types.rs \| wc -l` |
| `type_: String` fields in `ir/value.rs` (`IrValue` variants) | 17 | `rg -c 'type_: String' src/ir/value.rs` |
| `.type_` field accesses across `src/` | 676 | `rg -c '\.type_\b' src/ \| awk -F: '{s+=$2} END{print s}'` |
| scalar-name `== "Integer"`-style compares across `src/` | 123 | `rg -n '== "(Integer\|String\|Boolean\|Float\|Fixed\|Byte\|Money\|Nothing\|AttributeString)"' src/ \| wc -l` |
| structural prefix matches (`strip_prefix("List OF ")` etc.) | 166 | `rg -n 'strip_prefix\("(List OF \|Set OF \|Map OF \|RES \|MapEntry OF \|Result OF \|Thread OF \|ISOLATED )\|starts_with\("(List OF \|Set OF \|Map OF \|RES )' src/ \| wc -l` |
| `type_:/returns: <...>to_string()` alloc sites | 710 | `rg -n 'type_: .*to_string\(\)\|returns: .*to_string\(\)' src/ \| wc -l` |
| `format!("List OF …")`-style type constructions | 52 | `rg -n 'format!\("(List OF\|Set OF\|Map OF\|Result OF\|MapEntry OF)' src/ \| wc -l` |
| `ir/lower.rs` size | 3825 | `wc -l src/ir/lower.rs` |
| wire serializers (`ir/binary.rs`+`ir/json.rs`) | 2668 | `wc -l src/ir/binary.rs src/ir/json.rs` |

The 676 `.type_` accesses are the full retype blast radius, but most are *reads*
that behave identically on a `ParameterType` (equality, passing along); the churn
concentrates in the sites that build/compare type *strings* (the 123 + 166 + 710 +
52 above), which become enum matches / structural comparisons.

### Verified properties

- **The IR is post-monomorph and concrete.** `src/ir/lower.rs:141` documents it;
  `src/ir/lower.rs:2841` relies on it ("the monomorphizer already rewrote them to a
  concrete symbol"). VERIFIED.
- **`ir/types.rs` has no generic notion.** `rg 'template|generic|Var\b|type_param'
  src/ir/types.rs` → 0. So retyping is String→`ParameterType`, not
  adding-polymorphism. VERIFIED.

## 3. Design Overview

Two moves, then a consumer sweep:

1. **Boundary conversion.** In `ir::lower_augmented_project`, every place that today
   stores a source type `String` on an IR node instead stores
   `ParameterType::parse(&s)`. Because the input is concrete (post-monomorph), no
   `Var` arises — every parse yields a concrete `ParameterType`.
2. **Field flip.** Change the 8 `ir/types.rs` fields + 17 `ir/value.rs` `type_`
   fields from `String` to `ParameterType`.
3. **Consumer sweep.** Repoint codegen / `ir::verify` / `binary_repr` reads:
   - string `==`/`match` on type names → `ParameterType` match/`==`;
   - `strip_prefix("List OF ")` structural tests → match on `ParameterType::ListOf`;
   - `format!("List OF {}", x)` constructions → `ParameterType::list_of(...)`;
   - wire serializers call `t.name()` at the emit point (bytes unchanged).

Risk concentrates in the consumer sweep (676 sites) — but it is gated by
byte-identity: if the emitted machine code is unchanged, the retype is faithful.

**Re-measure and split at kickoff.** Likely parts: B1 = boundary conversion +
field flip in `ir/types.rs`/`ir/value.rs` + make it compile (consumers use
`.name()` shims at first); B2 = replace the `.name()` shims in codegen with native
`ParameterType` matches; B3 = `ir::verify` + `binary_repr` + wire seams. Each part
byte-identical.

### Rejected alternatives

- **Convert per-consumer instead of at the boundary.** Rejected: that keeps the IR
  string-typed and re-parses at every consumer — the opposite of the goal.

## Compatibility / Format Impact

IR JSON/binary artifact bytes unchanged (serializers render `name()`). No other
external contract touched.

## Phases

> Re-measure and split into B1/B2/B3 at kickoff. The phases below are the shape.

### Phase 1 — boundary conversion + field flip (compiles via `.name()` shims)

Split into **B1a** (6 `ir/types.rs` struct fields) and **B1b** (19 `ir/value.rs`
fields) — see Corrections. Each lands byte-identical.

**B1a — flip the 6 `ir/types.rs` struct type fields:**
- [x] Flip `IrBinding.type_`, `IrField.type_`, `IrParam.type_`,
      `ExternalFunctionParam.type_`, `EntryPoint.returns`, `IrFunction.returns` to
      `ParameterType` (`src/ir/types.rs`).
- [x] Boundary: `ir::lower` (+ `manifest`, `cli/build`) wrap source strings with
      `ParameterType::parse` at each construction site.
- [x] Consumers compile via `.name()`/`.name().into_owned()` shims (wire seams
      `ir/binary.rs`/`ir/json.rs`/`binary_repr/writer.rs` render `.name()` at the
      emit point — the *final* form, not a temporary shim; `ir/verify` shims reads,
      with a few native `== ParameterType::Unknown/Nothing` comparisons).
- [x] Test fixtures updated (subagent): constructions → `ParameterType::parse`,
      assertions → `.name()`. (production build 0 errors/0 warnings.)

**B1b — flip the 17 `ir/value.rs` `type_` + `UnionWrap.union_type`/`.member_type`:**
- [x] Flip the 19 `ir/value.rs` type-name fields to `ParameterType`.
- [x] Seam: `IrValue::annotated_type()` returns `Option<Cow<'_, str>>` (from
      `.name()`); added `annotated_parameter_type() -> Option<ParameterType>`.
      Production `usable_type(...annotated_type())` callers use `.as_deref()`;
      direct destructuring reads shimmed with `.name()` (+ native
      `matches!(type_, ParameterType::…)` for scalars).
- [x] Boundary: `ir::lower` value-lowering wraps ~44 IrValue constructions with
      `ParameterType::parse`; wire seams (`ir/binary.rs`/`ir/json.rs`/
      `binary_repr/writer.rs`/`nir/lower.rs`) render `.name()` at emit.
- [x] Test fixtures updated (~305 sites across `ir/tests.rs`,
      `variant_corpus_tests.rs`, `verify/tests.rs`, `binary_repr/tests/*`; note
      `IrOp::{For,ForEach,Bind}.type_` stay `String` — IrOp is out of B's scope).

Acceptance: `cargo test` green; `artifact-gate all` no NEW diff vs the plan-102-A
baseline. **B1a VERIFIED**: production 0/0, test build 0/0, full suite's sole
failure is the `artifact_gate_all` baseline, gate `diff` IDENTICAL to baseline.
B1b pending.
Commit (B1a): 1a8fbc5cb

### Phase 2 — native `ParameterType` in codegen consumers

- [x] ~~Replace the temporary `.name()` shims in codegen with native `ParameterType`
      matches~~ — **moot: codegen consumes NIR, not IR** (see Corrections). Codegen's
      type-compare/`strip_prefix`/`format!` sites read `NirValue.type_` (`String`),
      produced at the IR→NIR boundary (`nir/lower.rs:397 lower_value(&IrValue) ->
      NirValue`). Evidence: zero `src/codegen/` files in the B1a (78) or B1b (150)
      flip error sets; `NirValue.type_` is `String` (20 fields). There are NO codegen
      IR-type `.name()` shims to replace. Typing NIR is out of plan-102-B's IR scope.
- [x] ~~Tests: codegen unit + `rt-behavior` suite~~ — moot with the above.

Acceptance: N/A (phase moot — codegen does not consume IR type fields; the IR→NIR
`.name()` render is the correct final seam). The IR-consumer string-op reduction
that IS in scope (`ir::verify`) moves to Phase 3.
Commit: — (moot)

### Phase 3 — `ir::verify` native retype (wire seams already final)

The wire serializers (`ir/binary.rs`/`ir/json.rs`) and `binary_repr` already render
`name()` at the byte-emit point (landed in B1a/B1b) — the intended final form, byte-
identical. The genuine remaining IR-layer work is converting `ir::verify`'s read
shims to native `ParameterType`.

- [ ] Convert `ir::verify`'s `.name()` read-shims (added in B1a/B1b) to native
      `ParameterType`: make the verify env maps (`FnSig.params`/`.returns`,
      `field_types`, `record_field_lists`, `globals`) and the shared helpers
      (`resource_base_type`, `parse_map`, `is_defaultable`, …) operate on
      `ParameterType` where it removes a re-parse, OR document each residual `.name()`
      as a deliberate string boundary (diagnostics that quote a type). No behavior
      change — diagnostics identical.
- [ ] Tests: the full `*-invalid` diagnostic golden corpus (accept/reject + wording +
      order unchanged); `.mfp` round-trip + IR JSON golden tests.

Acceptance: IR JSON/binary artifact bytes byte-identical (golden diff empty);
`artifact-gate all` no NEW diff vs baseline; `test-accept` no NEW mismatch; `cargo
test` green; diagnostic goldens byte-identical.
Commit: —

## Validation Plan

- Tests: IR/codegen unit suite, `rt-behavior`, `.mfp` round-trip, IR JSON goldens.
- Coverage check: the retyped `ir::lower`/codegen paths are exercised by the full
  fixture suite (every compiled fixture flows through them).
- Runtime proof: `artifact-gate all` byte-identical (modulo baseline).
- Doc sync: if `.ai/codegen-invariants.md` describes IR type fields as `String`,
  update it. Spec `architecture/04_ir.md` / `20_ir-json-artifact.md` describe the
  IR — confirm they don't assert `String`-typed fields (the JSON bytes are
  unchanged, so likely no spec change).
- Acceptance: `cargo test`; `artifact-gate all` (no NEW diff); `test-accept` (no
  NEW mismatch); fmt both crates.

## Open Decisions

- **Keep a `type_name(): Cow` accessor on IR nodes for the handful of consumers
  that genuinely want the string?** Recommend yes, a thin `name()` wrapper, to
  keep the wire seams and any diagnostic that quotes a type readable. (§3)

## Corrections

- **Field census refined at kickoff (2026-08-23).** The §2 grep `type_: String|
  returns: String|kind: String|type_name: String` counted **8** fields in
  `ir/types.rs`, but 2 of those (`IrType.kind` line 5, `IrFunction.kind` line 159)
  are the record/union/enum and function/sub *kind* discriminants — NOT type
  spellings — and must stay `String`. The genuine type-spelling fields in
  `ir/types.rs` are **6**: `IrBinding.type_`, `IrField.type_`, `IrParam.type_`,
  `ExternalFunctionParam.type_`, `EntryPoint.returns`, `IrFunction.returns`
  (commands: `rg -n 'type_: String|returns: String' src/ir/types.rs`).
- **Additional `IrValue` type-name fields beyond the 17 `type_`.** `IrValue` also
  carries type spellings under other field names the `type_: String` grep missed:
  `UnionWrap.union_type`/`.member_type` (`src/ir/value.rs:76-77`). These are genuine
  type spellings and are flipped too, so the IR is fully typed (else a re-parse
  survives at the union-wrap seam). Total `IrValue` type-name fields flipped: 19.
- **Split refined to B1a/B1b/B2/B3** (finer than the doc's B1/B2/B3) to keep each
  atomic-compiling step smaller: B1a = flip the 6 `ir/types.rs` struct fields +
  boundary + shims; B1b = flip the 19 `ir/value.rs` fields + boundary + shims;
  B2 = native `ParameterType` in codegen consumers (replace shims); B3 =
  `ir::verify`/`binary_repr`/wire seams. Same seams and acceptance as the doc.

- **CODEGEN CONSUMES NIR, NOT IR — Phase 2 (codegen native) is MOOT for the IR
  retype (2026-08-23).** The plan's §2 assumed codegen reads IR type fields
  directly (the "676 `.type_` sites, all of codegen" blast radius). It does not:
  there is a **NIR** (native-IR) layer between IR and codegen. `nir/lower.rs:397`
  `fn lower_value(value: &IrValue) -> NirValue` (and the sibling `lower_type`/
  binding/param/field lowerings) is the **IR→NIR boundary**, and `NirValue.type_`
  is `String` (`rg -c 'type_: String' src/target/shared/nir/mod.rs` → 20). Codegen's
  ~80 scalar type-compares / 18 `strip_prefix` / 23 `format!` sites all operate on
  **`NirValue.type_` (String)**, unaffected by the IrValue flip — which is exactly
  why **zero `src/codegen/` files appeared in the B1a or B1b error sets**. Evidence:
  the B1a flip (78 errors) and B1b flip (150 errors) touched `ir/`, `verify/`,
  `binary_repr/`, `manifest/`, `cli/build`, `nir/lower` — never `src/codegen/`.
  Consequence:
  - The `.name().into_owned()` renders at the IR→NIR boundary (`nir/lower.rs:132,
    159,198,232`) ARE the correct **final** form for plan-102-B — NIR stays
    string-typed; typing NIR is a distinct, larger effort **outside** plan-102-B's
    stated IR scope (Goal §1 lists only IR fields, never NIR).
  - **Phase 2 (codegen native) is moot**: there are no codegen IR-type `.name()`
    shims to replace. The codegen Q3 string-op win the feature envisioned lives in
    the NIR layer, and belongs to a separate plan, not plan-102-B.
  - **Phase 3** is re-scoped to its genuinely-remaining IR-layer content: convert
    `ir::verify`'s `.name()` read-shims (added in B1a/B1b) to native `ParameterType`
    so the one real IR consumer that does string type-logic stops re-deriving from
    strings. The wire seams (`ir/binary.rs`/`ir/json.rs`) and `binary_repr` already
    render `.name()` at the byte-emit point — that IS the intended final form, so
    they need no further change beyond what B1a/B1b landed. This becomes **B2**
    (renumbered), and the old "codegen native" B2 is dropped as moot.

## Summary

Largest blast radius of the feature (676 `.type_` sites, all of codegen), but
lowest design uncertainty: the IR is already concrete, so this is a pure
String→`ParameterType` swap gated by byte-identity. It delivers a typed IR and the
codegen-side Q3 win independently of the HIR work above it.
