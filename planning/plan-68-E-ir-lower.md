# plan-68-E: IR lowering / binary / link

Last updated: 2026-07-27
Overall Effort (AI): large (3h–1d)   (whole plan-68 feature)
Effort (Human): large (3h–1d)
Effort (AI): medium (1h–3h)
Depends on: plan-68-A
Produces: nothing — the six files below reach ≥95% and drop off
`sh scripts/coverage-check.sh`; no source-behavior or exception-list change is
intended. (If a coverage test surfaces a real defect, it is fixed on its own
RED-first commit per `AGENTS.md`, not worked around — that is the only
production edit this letter may make.)

Part **E** of plan-68. Shared goal, prerequisites, dependency graph, the
except-vs-backfill rule, and the standing test-authoring requirements live in the
overview: [plan-68-coverage-gate.md](plan-68-coverage-gate.md). The worklist and
the fresh `target/coverage/coverage.json` this letter reads for exact uncovered
lines come from [plan-68-A-triage-exceptions.md](plan-68-A-triage-exceptions.md).
Re-run the prerequisites before starting; do not restate them here.

## Scope (the six files, covered/total from `sh scripts/coverage-check.sh`)

| File | covered/total | pct | uncov | Phase |
|---|---|---|---|---|
| src/ir/lower.rs | 2692/2911 | 92.48 | 219 | E1 |
| src/ir/binary.rs | 1180/1253 | 94.17 | 73 | E2 |
| src/ir/link.rs | 456/490 | 93.06 | 34 | E3 |
| src/ir/lower_link.rs | 246/275 | 89.45 | 29 | E4 |
| src/ir/docs.rs | 128/138 | 92.75 | 10 | E4 |
| src/ir/package.rs | 197/208 | 94.71 | 11 | E4 |

All six are pure transform / encode / decode / validate logic → almost entirely
unit-coverable. The uncovered remainder is un-exercised lowering variants,
encode/decode branches, and error paths — not integration boundaries.

## Test harness this letter reuses (do NOT reinvent)

Everything lands in the existing shared test file `src/ir/tests.rs` (5368 lines)
and the inline `#[cfg(test)]` module in `src/ir/link.rs:899`. Precedent to
follow:

- **Source-driven lowering fixtures** — `tests.rs` `helpers::lower_src(src) ->
  IrProject` / `try_lower_src` run the real front-end (parse → resolve →
  monomorph → **lower**); `helpers::function(ir, name) -> &IrFunction` fetches a
  lowered function so a test asserts on `.body` ops / `link_functions` /
  `link_cstructs` / `native_resources` / docs decls. Existing test modules:
  `lowering_totality_tests` (138), `lower_tests` (574), `lower_pipeline_tests`
  (3145).
- **Encode/decode round-trip fixtures** — `binary_repr_tests` (235) round-trips
  `variant_corpus_tests::variant_corpus()` (defined in
  `src/ir/variant_corpus_tests.rs`), the "strict superset of every IrType / IrOp
  / IrValue / IrMatchPattern kind." `binary_repr_round_trip_is_identity` asserts
  `project.to_json() == decoded.to_json()`; error paths use a valid buffer with
  one byte corrupted (`binary_repr_rejects_bad_magic` / `…_bad_version`).
- **LINK-layout unit fixtures** — `link.rs` inline tests use a `fields(&[(name,
  ctype)])` helper + `check_cstruct` / `compute_c_layout` assertions.

Prefer extending `variant_corpus` (so one added variant covers both its encode
and its decode arm) over hand-rolling parallel IR; prefer `lower_src(<snippet>)`
over hand-building AST.

## Unreachable-arm candidates (hand to A; do NOT contrive invalid-IR tests)

Reading the source found three lines that a *valid* input cannot reach — they are
defensive arms, not coverable transforms. They are single lines inside otherwise
coverable files, so each file still clears 95% around them; leave them uncovered.
Only if A's fresh report shows one of them is what keeps its file under 95% does
it become a line-level exception note for A (not a test here):

- `src/ir/lower.rs:3108` — `unreachable!("inline TRAP must be lowered as a
  statement value")` (the value-position lowering of an inline TRAP; the desugar
  routes it through the statement path first).
- `src/ir/link.rs:585` — `AbiDirection::Out => unreachable!("guarded above")` in
  `check_buffer_slots`.
- `src/ir/lower.rs:3614` `write_ir` **Err arm only** — the `std::fs` write
  failure. Its happy path IS covered (E1); the error return is an fs boundary,
  not a transform. Do not simulate a filesystem failure to chase one line.

Everything else in the six files is backfill.

## Phases

### Phase E1 — src/ir/lower.rs (219 uncov, the big one)

Read A's fresh `coverage.json` for `src/ir/lower.rs` to get exact uncovered
lines, then map them to the construct groups below (the file's lowering is two
large match dispatchers — `lower_statement` at :460 and
`lower_expression_with_expected` at :2378 — plus type-inference helpers). Add a
test per un-exercised construct in a new `mod lower_construct_tests` beside the
existing `lower_tests`. Fixtures are `lower_src(<snippet>)` +
`function(ir,"main").body` shape assertions.

- [ ] **Statement arms** (`lower_statement`, :460–932): cover the arms the
      existing suite under-exercises — `Recover` (both `(Some(slot),Some(value))`
      and the no-slot branch, :606ff), `StateAssign` (:678), `Exit` with a target
      vs. without (:570), `DoUntil` (:917), `Propagate` (:600), `Continue { kind
      }`. Fixture: a `sub`/`func` body using each; assert the emitted `IrOp`
      kind(s).
- [ ] **Expression arms** (`lower_expression_with_expected`, :2378–3230):
      `WithUpdate` (:3009), `SetLiteral` (:3057), `MapLiteral` (:3075),
      `Lambda` with captured locals incl. a `by_ref` capture (:2834),
      `Constructor` that wraps a union variant via `wrap_union_value` (:2991 /
      :3258), `MemberAccess` (:3096), and the `Unary` fold of `-<Number>` vs. a
      non-literal operand (:3131, :773). Fixture: `lower_src` of an expression
      using each; assert the `IrValue` shape.
- [ ] **Type / argument inference helpers**: `expression_type` (:1776) over the
      less-common shapes it dispatches, `builtin_argument_types` (:2107) for a
      builtin whose signature isn't already hit,
      `normalize_overloaded_builtin_call_arguments` (:2207),
      `function_param_types_from_type` / `function_return_from_type` (:2039),
      `promote_loop_numeric_type_name` (:1420), `parse_map_type` /
      `parse_map_entry_type` (:1432/:1438) `None` paths,
      `filter_predicate_arg_type` / `builtin_predicate_ref_type` (:2343/:2357).
      Fixture: a lowered program whose numeric/loop/map/predicate types force each
      branch; assert on the inferred `type_` in the emitted binding/op.
- [ ] **`write_ir` happy path** (:3614): `write_ir(temp_dir, &lower_src(...))`
      returns `Ok(path)` and the file exists on disk. (Err arm: see
      unreachable-arm list — leave.)
- [ ] Do NOT add a test for the `unreachable!` at :3108.

Acceptance: `sh scripts/coverage.sh` (fresh), then
`sh scripts/coverage-check.sh src/ir/lower.rs` shows ≥95%. `cargo test` → `0
failed`.
Commit: —

### Phase E2 — src/ir/binary.rs (73 uncov)

Encode/decode round-trip module. Read A's report; the uncovered lines split into
(a) encode/decode variant arms not present in `variant_corpus`, and (b) the
`decode_*` malformed-input error branches. Extend `binary_repr_tests`.

- [ ] **Complete the round-trip corpus**: for any `encode_op` / `encode_value` /
      `encode_type` / `encode_match_pattern` / `encode_link_expr` arm A's report
      shows uncovered, add that variant to `variant_corpus_tests::variant_corpus`
      so `binary_repr_round_trip_is_identity` exercises both its encode and its
      matching `decode_*` arm in one shot.
- [ ] **Decode error branches** — feed hand-crafted / truncated byte buffers to
      `decode_binary_repr` (and the sub-decoders it reaches) and assert
      `.is_err()`, one test per branch: unknown `IrOp` tag (:1227), unknown
      `IrValue` tag (:1587), unknown `IrMatchPattern` tag (:1281), unknown loop
      kind (`decode_loop_kind` :1231), `decode_vec_capped` cap-exceeded (:572 —
      craft a length header past the cap), `decode_cstructs` bad count / bad field
      (:589/:600), `decode_resource_owners` error (:954), bad-utf8 in a `put_str`
      field, and a mid-record truncation of `decode_project` /
      `decode_link_function`. Reuse the "encode corpus, corrupt N bytes" pattern
      of `binary_repr_rejects_bad_magic`.
- [ ] **`verify_package` error branches** (:1616–): the `Err` arms not already
      covered by `verify_package_rejects_duplicate_function` — bad op depth
      (`verify_ops` :1655), any remaining reject reason. Fixture: an `IrProject`
      built to violate each rule; assert the `Err` message.

Acceptance: `sh scripts/coverage.sh` (fresh), then
`sh scripts/coverage-check.sh src/ir/binary.rs` shows ≥95%. `cargo test` → `0
failed`.
Commit: —

### Phase E3 — src/ir/link.rs (34 uncov)

C-ABI layout + CSTRUCT validation. Extend the existing inline `#[cfg(test)]`
module (:899) using its `fields()` helper.

- [ ] **`check_cstruct` fault branches** (:305): each `CStructFault` kind — unknown
      field ctype (extends existing `rejects_unknown_field_ctype`), `Cvoid` field
      (extends `rejects_cvoid_field`), plus any misalignment / bad-slot fault A's
      report shows uncovered. Fixture: a `StructSlotView` built from `fields(...)`
      that trips each fault; assert the returned `Vec<CStructFault>`.
- [ ] **`check_buffer_slots`** (:549): the `AbiDirection::In` / `InOut` branches
      and the missing-length / `None` fault (:641). Fixture: a `BufferSlotsView`
      per direction. Do NOT test the `Out => unreachable!` arm (:585, see
      unreachable list).
- [ ] **ctype predicate + size helpers**: `abi_ctype_valid_as_argument` (:48) /
      `abi_ctype_valid_as_return` (:65) for the reject-cases, `ctype_size_align`
      `None` path (:117), `cstruct_field_mfb_type` `None` path (:202),
      `abi_slot_ctype_is_known` (:19) for a false result.
- [ ] **Direction enum**: `from_code` (:836) for each valid code and one invalid
      (`None`), `code` round-trip (:828), `writes_back` (:846) for each variant.

Acceptance: `sh scripts/coverage.sh` (fresh), then
`sh scripts/coverage-check.sh src/ir/link.rs` shows ≥95%. `cargo test` → `0
failed`.
Commit: —

### Phase E4 — src/ir/lower_link.rs (29) + src/ir/docs.rs (10) + src/ir/package.rs (11)

Cluster of three small files, all source-driven via `lower_src` / a merged
`IrProject`. Add a `mod lower_link_tests`, `mod docs_tests`, `mod package_tests`
in `tests.rs` (or extend the nearest existing block — `collect_project_docs` is
already touched near `tests.rs:2146`, package prefix near :350).

- [ ] **lower_link.rs** — `lower_src` of a LINK block, assert
      `ir.link_functions` / `link_cstructs` / `native_resources` / `link_aliases`:
      `eval_link_const_opt` arms (`SIZEOF` :184, unary `-` :200, unary `+` :203,
      `NOTHING` :181, the `None` default :204), `lower_link_expr` operator arms
      (`NOT`, `AND`/`OR`, the compare operators, `*`/`+`/`-`, the `_ => Int(0)`
      fallback :320/:323), `link_const_bits` for hex / binary / decimal text
      (:228), `lower_bind_in_field` param-vs-literal-vs-negative (:243),
      `native_resources` visibility arms (`export`/`public`/`private` :370–373)
      and `close_may_fail` true/false, `link_functions` resource-param formatting
      with a `STATE` type (:62).
- [ ] **docs.rs** — `lower_src` of a program carrying doc-comment headers, assert
      `collect_project_docs` decls: `DocHeaderKind` arms `Package` (:157),
      `Func`/`Sub` (:166), `Type`/`Union`/`Enum` (:182, each mapping to the right
      `IrDocKind` :189), `Resource` requiring `Export` visibility (:196 — cover
      both the emitted and the skipped-because-non-export branch), and
      `header_params` `Some(wanted)` vs `None` matching (:99–103).
- [ ] **package.rs** — build two `IrProject`s via `lower_src` and exercise the
      rewrite/merge path: `prefix_package_symbols` (:10) name-qualification of
      bindings/entry/link tables, `rewrite_op_targets` arms not already hit
      (`Match` with `OneOf` pattern :218, `ForEach` :262, `Trap` :268,
      `ExitProgram` :200, `AssignGlobal` :188), `rewrite_value_targets`
      `FunctionRef`/`Closure`/`Global` (:291/:296), `push_unique` dedup (:165),
      `merge_package` (:115), `package_qualified_reference_names` (:69),
      `apply_package_identity` (:89). Assert on the rewritten names / merged
      vectors.

Acceptance: `sh scripts/coverage.sh` (fresh), then `sh scripts/coverage-check.sh
src/ir/lower_link.rs src/ir/docs.rs src/ir/package.rs` shows all three ≥95%.
`cargo test` → `0 failed`.
Commit: —

## Validation Plan

- **Per phase**: run `sh scripts/coverage.sh` once (rebuilds the profile the
  checker reads), then `sh scripts/coverage-check.sh <path>` for the phase's
  file(s) → ≥95%, and `cargo test` → `0 failed` (the whole suite, never a single
  module — a green targeted run does not prove the suite green).
- **Letter done**: `sh scripts/coverage-check.sh src/ir/lower.rs src/ir/binary.rs
  src/ir/link.rs src/ir/lower_link.rs src/ir/docs.rs src/ir/package.rs` lists none
  of the six as a GATE FAILURE.
- **No behavior drift**: `git diff --stat src/` shows only `src/ir/tests.rs`,
  `src/ir/link.rs` (its inline test module), and `src/ir/variant_corpus_tests.rs`
  changed — unless a coverage test surfaced a real bug, which appears as its own
  RED-first fix commit with a note in Corrections.
- **Unreachable arms**: if any of the three listed defensive lines is what holds
  its file under 95% after the backfill, record it in Corrections and hand it to A
  as a line-level exception note — do NOT author an invalid-IR test to reach it.

## Corrections

<Filled in during execution.>
