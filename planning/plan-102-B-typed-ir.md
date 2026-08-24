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

- [ ] Flip the 8 `ir/types.rs` and 17 `ir/value.rs` type fields to `ParameterType`.
- [ ] In `ir::lower_augmented_project`, convert source strings via
      `ParameterType::parse` at each IR-node construction site.
- [ ] Make consumers compile by inserting `.name()` at read sites (temporary; a
      mechanical shim so the tree builds before the real sweep).
- [ ] Tests: existing IR/codegen suite compiles and passes.

Acceptance: `cargo test` green; `artifact-gate all` no NEW diff vs the plan-102-A
baseline.
Commit: —

### Phase 2 — native `ParameterType` in codegen consumers

- [ ] Replace the temporary `.name()` shims in codegen with native `ParameterType`
      matches (`strip_prefix`→`ListOf` match; scalar `==`→variant `==`;
      `format!`→constructor).
- [ ] Tests: codegen unit + `rt-behavior` suite.

Acceptance: `artifact-gate all` no NEW diff; `cargo test` green; the codegen string
type-compare/alloc counts drop (re-run the §2 census over `src/codegen/` and record
the delta).
Commit: —

### Phase 3 — `ir::verify`, `binary_repr`, wire seams

- [ ] Retype `ir::verify`'s type reads to `ParameterType`.
- [ ] `binary_repr` reads `ParameterType`; `ir/binary.rs`/`ir/json.rs` call
      `name()` only at the byte-emit point.
- [ ] Tests: `.mfp` round-trip + IR JSON golden tests.

Acceptance: IR JSON/binary artifact bytes byte-identical (golden diff empty);
`artifact-gate all` no NEW diff; `test-accept` no NEW mismatch; `cargo test` green.
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

<Filled in during execution.>

## Summary

Largest blast radius of the feature (676 `.type_` sites, all of codegen), but
lowest design uncertainty: the IR is already concrete, so this is a pure
String→`ParameterType` swap gated by byte-identity. It delivers a typed IR and the
codegen-side Q3 win independently of the HIR work above it.
