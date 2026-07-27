# plan-68-D: IR verifier + small IR/arch

Last updated: 2026-07-27
Overall Effort (AI): large (3h–1d)   (whole plan-68 feature)
Effort (Human): large (3h–1d)
Effort (AI): medium (1h–2h)
Depends on: plan-68-A
Produces: nothing new — these 8 files reach ≥95% line coverage and drop off the
`scripts/coverage-check.sh` failing list. No production `src/**` behavior change
(test-only), unless a coverage test surfaces a real bug (then fixed on its own
commit per `AGENTS.md`, not worked around).

Part **D** of plan-68. Shared goal, prerequisites, dependency graph, the design
rationale (except-vs-backfill), and the standing requirements live in the
overview: [plan-68-coverage-gate.md](plan-68-coverage-gate.md). Re-run its
Prerequisites gate before starting. The authoritative worklist + the fresh
`target/coverage/coverage.json` this sub-plan reads to pin exact uncovered lines
are produced by [plan-68-A-triage-exceptions.md](plan-68-A-triage-exceptions.md).

## Scope (measured)

Eight files, `covered/total` per the overview's population table (from
`sh scripts/coverage-check.sh`):

| File | covered/total | pct | uncov | Phase |
|---|---|---|---|---|
| src/arch/aarch64/backend.rs | 3/9 | 33.33 | 6 | D1 |
| src/ir/types.rs | 5/8 | 62.50 | 3 | D1 |
| src/ir/verify/calls.rs | 139/207 | 67.15 | 68 | D2 |
| src/ir/verify/link.rs | 434/571 | 76.01 | 137 | D3 |
| src/ir/verify/values.rs | 479/580 | 82.59 | 101 | D4 |
| src/ir/verify/resources.rs | 262/294 | 89.12 | 32 | D5 |
| src/ir/verify/compat.rs | 479/512 | 93.55 | 33 | D6 |
| src/ir/verify/mod.rs | 705/744 | 94.76 | 39 | D6 |
| src/arch/x86_64/encode/emitter.rs | 1471/1554 | 94.66 | 83 | D7 (added by A) |

(D1's `src/arch/aarch64/backend.rs` measured 6/9 on A1's fresh report, not 3/9 —
3 uncov lines, still its three trivial method bodies.)

The IR verifier is pure logic — it validates an in-memory `IrProject` and emits
diagnostics (`TypeEnv::emit(rule, detail)`). Nearly every uncovered line is an
un-exercised **error arm**: a diagnostic branch no existing test feeds malformed
IR to. This makes the work almost entirely backfill, not exceptions. The two
non-verifier files (D1) are a trivial dispatch shim and a small helper, both
directly unit-coverable — my read found **no** exception candidate in the set
(see D1's aarch64 note, which the overview's Open Decision flagged for A).

### Test infrastructure to reuse (do not rebuild)

The verifier test module is `src/ir/verify/tests.rs` (306 `#[test]` fns; count:
`grep -c '#\[test\]' src/ir/verify/tests.rs`). It already carries every fixture
builder these phases need — extend it, matching the nearest existing test's
style:

- `project(functions, types)` → `crate::ir::test_support::project_fixture(...)`.
- `collect_diagnostics(&project)`, `rules(&project)`, `accept(&project)`,
  `expect_rule(&project, "RULE")` — the assertion helpers.
- IR-value builders: `const_of`, `binary`, `unary`, `bind`, `ret`, `ret_none`,
  `record_typed`, `enum_type`.
- **LINK builders (D3):** `link_fn()` (`tests.rs:2730`), `cstruct(name, fields)`
  (`:2770`), `project_with_cstructs(...)` (`:2785`), and the `project_with_link`
  pattern (`p.link_functions = vec![lf]`). Set the `IrLinkFunction` fields
  (`abi_slots`, `bind_in`, `bind_state`, `bind_state_resource`, `consts`,
  `params`, `return_resource`, `return_state_type`) to craft each malformed
  shape.

### How each phase names its targets

The on-disk `target/coverage/coverage.json` is stale (mtime Jul 21; overview §2).
A regenerates it. **Before writing tests for a phase, open A's fresh report for
that file and confirm the exact uncovered line set** — the diagnostic arms named
below are the candidates I found by reading the source + diffing the rule-id set
against `tests.rs` (`grep -oE '"[A-Z][A-Z0-9_]{3,}"'` on each), but only A's
fresh line data proves which arms are truly uncovered and whether any residual is
an unreachable defensive arm needing an exception rather than a test.

## Phases

### Phase D1 — arch backend shim + `ir/types.rs` helper

Two tiny files, clustered. Both are directly unit-coverable; neither is an
exception.

**src/arch/aarch64/backend.rs (3/9).** The whole file is a zero-sized `Backend`
impl (`Aarch64Backend`, `src/arch/aarch64/backend.rs:19-37`). The 3 uncovered
lines are its three method bodies, exercised today only through full codegen
integration — but each is trivially callable in isolation:

- [x] Add a `#[cfg(test)] mod tests` to `backend.rs` (the file has none). Assert
      `AARCH64_BACKEND.is_aarch64()` is `true` (covers `backend.rs:34-36`).
- [x] Assert `AARCH64_BACKEND.register_model()` returns a model whose behavior
      identifies as AArch64 — call one `RegisterModel` method on the returned
      `&AARCH64_MODEL` (covers `:30-31`).
- [x] Assert `AARCH64_BACKEND.select(&[])` returns an empty `Vec` (covers `:26-28`;
      `select_aarch64` loops over the input, so the empty slice is safe — verified
      `src/arch/aarch64/select.rs:20-22`). This closes the overview's Open
      Decision in favor of **backfill**: no codegen-integration boundary is needed;
      an empty/`AddrOf` MIR fixture reaches every line. Only if a subagent finds
      `select` panics on empty input (it will not, per the read) does this become
      an A exception with a named codegen-integration boundary — flag it back to A,
      do not weaken the gate silently.

**src/ir/types.rs (5/8).** The only executable code in the file is
`IrProject::link_library_names` (`src/ir/types.rs:135-143`); everything else is
`struct`/`impl` declarations (not in the denominator). The 3 uncovered lines are
that function's body, including the `if !names.contains(...)` dedup arm.

- [x] In `src/ir/verify/tests.rs` (or a new `#[cfg(test)]` in `types.rs`),
      construct an `IrProject` (via `project_fixture`) whose `link_functions`
      declare libraries in the order `["a", "b", "a"]`; assert
      `link_library_names()` returns `["a", "b"]` — covers the loop, the push, and
      the dedup `contains` guard in one fixture.

Acceptance: after a fresh `sh scripts/coverage.sh`,
`sh scripts/coverage-check.sh src/arch/aarch64/backend.rs src/ir/types.rs` shows
both ≥95%.
Commit: 15f206a71

### Phase D2 — `ir/verify/calls.rs` STATE-agreement family (68 uncov)

Every rule the file's operator/arity arms emit (`TYPE_UNARY_OPERATOR_MISMATCH`,
`TYPE_UNARY_OPERATOR_UNKNOWN`, `SYMBOL_NOT_CALLABLE`, `TYPE_CALL_ARITY_MISMATCH`,
`TYPE_CALL_ARGUMENT_MISMATCH`, `TYPE_UNION_STATE_FORBIDDEN`) already has a test.
The 68 uncovered lines are concentrated in the **resource-STATE agreement**
functions — dense multi-arm matches with no `tests.rs` reference to their
`TYPE_STATE_MISMATCH` / `TYPE_STATE_OPAQUE_NARROWING` messages:

- [x] `check_argument_state_agreement` (`calls.rs:238-267`): fixture a call whose
      callee parameter is declared `RES p AS File STATE Cursor` and pass an
      argument carrying `STATE Label` → `TYPE_STATE_MISMATCH` (the "carries STATE
      T2" arm, `:252-255`); and pass a stateless argument → the "carries no STATE"
      arm (`:256-258`). A bare param (`state_type_name(param_type)` = `None`) must
      **accept** anything — add the accepting case too (guards `:245`).
- [x] `check_thread_state_agreement` (`calls.rs:~180-205`): fixture a
      `thread::transfer` whose plane resource and transferred resource disagree —
      cover all three detail arms: `(Some,Some)` mismatch, `(Some,None)`
      plane-declares-but-carries-none, `(None,Some)` bare-plane-but-carries-state
      → `TYPE_STATE_MISMATCH` each (`:187-204`).
- [x] `check_return_state_declaration` (`calls.rs:281-304`): a FUNC returning a
      resource **union** with a STATE → `TYPE_UNION_STATE_FORBIDDEN` (`:287`); a
      FUNC whose return STATE type is non-defaultable → `TYPE_STATE_INVALID`
      (`:296`).
- [x] `check_binding_state_agreement` (`calls.rs:351-400`): bind a bare `RES`
      parameter (opaque state) under a concrete `STATE T` →
      `TYPE_STATE_OPAQUE_NARROWING` (`:369`, requires seeding
      `current_opaque_params` — mirror how a bare-`RES`-param function body is set
      up); a binding declaring `STATE T` whose initializer carries `T2` →
      `TYPE_STATE_MISMATCH` (`:387`); a bare binding whose initializer carries a
      state → the bare-binding arm (`:393`). Also the agreeing `declared ==
      value_state` accept case (`:386`).

Acceptance: fresh `sh scripts/coverage.sh`, then
`sh scripts/coverage-check.sh src/ir/verify/calls.rs` shows ≥95%.
Commit: e95d0ed31 (calls.rs:199 (None,None) transfer arm unreachable — see Corrections)

### Phase D3 — `ir/verify/link.rs` native-LINK arms (137 uncov)

The largest file. `check_link_cstructs` and `check_link_functions` re-run every
native-ABI marshaling rule on the decoded-package path. The `NATIVE_ABI_*`,
`NATIVE_CPTR_ESCAPE`, `NATIVE_CSTRUCT_*`, `NATIVE_CONST_*`, `NATIVE_FREE_INVALID`,
`NATIVE_STRUCT_FIELD_MISMATCH` rules have tests (`tests.rs:2730+`). The 137
uncovered lines are the arms with **no** `tests.rs` reference — build each with
the existing `link_fn()`/`cstruct()`/`project_with_cstructs()` helpers:

- [x] **`NATIVE_BIND_IN_INVALID`** — all four arms in `check_link_cstructs`
      (`link.rs:104-160`): a `BIND IN` naming a nonexistent ABI slot (`:106`); a
      slot that is not a CSTRUCT (`:120`); a field the CSTRUCT does not declare
      (`:131`); a field binding neither/both of param+literal (`:140`); a field
      binding an unknown parameter (`:151`). Set `lf.bind_in` with an
      `IrBindIn`/field shape for each.
- [x] **`NATIVE_STRUCT_FIELD_MISMATCH` maps-to-not-a-record arm** (`link.rs:74`):
      a CSTRUCT whose `maps_to` names a type absent from `project.types` (or a
      non-`type`/`record` kind), referenced by an `abi_slot`'s `ctype`.
- [x] **`NATIVE_CSTRUCT_INVALID` duplicate-name arm** (`link.rs:38`): two CSTRUCTs
      with the same `alias` + `name`.
- [x] **`NATIVE_CSTRUCT_ESCAPE` via `check_link_cstructs`** (`link.rs:181,190`): a
      link function whose param/return type names a sibling CSTRUCT (distinct from
      the `check_link_functions` C-ABI-escape path if A's report shows this one
      uncovered).
- [x] **`NATIVE_BIND_STATE_INVALID`** — all four arms (`link.rs:505-568`): a
      `bind_state` naming a slot that is not an OUT CSTRUCT slot (`:515`); a
      `bind_state` present but the function does not return a stateful resource
      (`:523`); the CSTRUCT's `maps_to` disagreeing with `return_state_type`
      (`:533`); `bind_state_resource` naming a slot other than the returned one
      (`:559`).
- [x] **`TYPE_STATE_MISMATCH` native cross-declaration arm** (`link.rs:582`): two
      link functions declaring the same native resource base type with **different**
      STATE types (one as a producer `return_state_type`, one as a param's
      `state_type_name`).
- [x] **Helper branches** — if A's report shows them uncovered: the collection
      recursion in `contains_resource_or_thread` (`link.rs:619-635`, `List OF`/
      `Map OF`/record-field/cycle arms), the union/collection arms of
      `provably_data_type` (`:642-659`), and the `consumed_resource`
      `Eval`/`Assign`/`Return` op-shape arms (`:714-732`).

Acceptance: fresh `sh scripts/coverage.sh`, then
`sh scripts/coverage-check.sh src/ir/verify/link.rs` shows ≥95%.
Commit: ebc83a5fd

### Phase D4 — `ir/verify/values.rs` money + set/field arms (101 uncov)

The literal-range, collection-element, member-access, and binary-operator rules
are largely tested. The uncovered cluster (rule-ids absent from `tests.rs`) is the
**Money** algebra plus a few member/collection arms:

- [x] **Money literals — `check_const_literal` Money arm** (`values.rs:373-385`):
      a `Money` const with >5 fractional digits → `TYPE_MONEY_LITERAL_PRECISION`
      (`:374`); a `Money` const outside range (converter `Err`) →
      `TYPE_MONEY_LITERAL_OVERFLOW` (`:381`).
- [x] **Money literals — `check_negated_const_literal` Money arm**
      (`values.rs:430-442`): a negated Money with >5 fractional digits →
      `TYPE_MONEY_LITERAL_PRECISION` (`:431`); a negated Money below range →
      `TYPE_MONEY_LITERAL_UNDERFLOW` (`:438`). Feed via a `Unary { op: "-", .. }`
      over a `Const { type_: "Money", .. }` (the `check_literal_range` dispatch at
      `:302-308`).
- [x] **`check_money_operands`** (`values.rs:681-715`): a Money-vs-non-Money
      comparison (`=`,`<`,… with `l_money != r_money`) → `TYPE_MONEY_OPERATION_INVALID`
      (`:687`); each invalid-arithmetic reason arm — `+`/`-`/`MOD` with a
      non-Money operand, `Money * Money`, non-Money `/` Money, `Money ^ x`
      (`:700-714`). Accept-cases: same-dimension add, `M/M`, scalar scale (guard
      `:696`).
- [x] **`TYPE_UNKNOWN_FIELD`** in `check_member_access` (`values.rs:543`): a
      `MemberAccess` on a known record whose complete field set does **not**
      contain the member. (`TYPE_MEMBER_NOT_VISIBLE` `:547` may already be tested —
      confirm against A's report; if uncovered, add a private-field-cross-file
      fixture via `hidden_from_here`.)
- [x] **`TYPE_REQUIRES_COMPARABLE` Set-element arm + `TYPE_COLLECTION_OWNERSHIP_VIOLATION`
      Set arm** in `check_map_key_comparable` (`values.rs:734-756`): a `Set OF T`
      whose element is incomparable → `TYPE_REQUIRES_COMPARABLE` (`:749`); a
      `Set OF T` whose element contains a resource/thread →
      `TYPE_COLLECTION_OWNERSHIP_VIOLATION` (`:742`). (The overview lists this file
      under D; the source rule is `TYPE_SET_ELEMENT_MISMATCH` at the collection
      literal path — check A's report for whether the set-literal-element arm or
      the set-type-key arm is the uncovered one, and cover whichever it is.)
- [x] **`is_comparable_seen` / `is_comparable_defaultable` recursion arms**
      (`values.rs:784-819`) and the `check_binary_operands` equality
      compatible-but-not-comparable vs incompatible split (`:646-673`): cover per
      A's report — an `=` over two incompatible types (operator-mismatch arm) vs
      two compatible-but-incomparable types (comparability arm).

Acceptance: fresh `sh scripts/coverage.sh`, then
`sh scripts/coverage-check.sh src/ir/verify/values.rs` shows ≥95%.
Commit: 34218c19d

### Phase D5 — `ir/verify/resources.rs` (32 uncov)

Small, three rule-ids. Concentrated in the RES-ownership axis and the
use-after-move dataflow:

- [x] **`collection_axis_element`** (`resources.rs:455-477`): a collection whose
      element is a bare resource type (`List OF File`, not `RES File`) →
      `TYPE_RESOURCE_REQUIRES_RES` (`:461`); a collection element marked `RES` over
      a provably-data type (`List OF RES Integer`) → `TYPE_RES_REQUIRES_RESOURCE`
      (`:468`); and the nested-collection recursion (`List OF List OF RES File`,
      `:476`) plus the `Map OF … TO …` value arm (`check_collection_res_axis`,
      `:450`).
- [x] **`TYPE_USE_AFTER_MOVE`** in `check_resource_moves` (`resources.rs:110-116`):
      a function body that closes/returns a resource binding and then reads it
      again (double-close / read-after-move). Also cover the alias-tracking arm
      (`:90-98`) if A's report shows it uncovered.

Acceptance: fresh `sh scripts/coverage.sh`, then
`sh scripts/coverage-check.sh src/ir/verify/resources.rs` shows ≥95%.
Commit: —

### Phase D6 — `ir/verify/compat.rs` (33 uncov) + `ir/verify/mod.rs` (39 uncov)

Both are already >93%. Every top-level rule-id in each file already has a test,
so the uncovered residual is **deep arms inside covered rules** — specific
type-compatibility branches in `compat.rs`'s `compatible` / `expression_compatible`
helpers, and scattered driver edges in `mod.rs`. These are the two files where
guessing from rule-ids is insufficient: **open A's fresh report and cover the
named uncovered lines directly.**

- [ ] **compat.rs**: from A's report, enumerate the uncovered arms of the
      compatibility matrix (`compat.rs` emits `TYPE_ASSIGNMENT_MISMATCH`,
      `TYPE_RETURN_MISMATCH`, `TYPE_BINDING_MISMATCH`, `TYPE_CONSTRUCTOR_*`,
      `TYPE_CONDITION_REQUIRES_BOOLEAN`, `TYPE_RESULT_IS_IMPLICIT`,
      `TYPE_READ_ONLY_RECORD_CONSTRUCTOR`, `TYPE_MEMBER_NOT_VISIBLE` — all tested).
      Add fixtures for the untested *type pairings* (e.g. numeric-widening arms,
      union-member compatibility, Result-implicit-unwrap) each uncovered line
      guards. For any arm A's report shows unreachable-by-construction (a
      required-for-exhaustiveness match arm the front-end can never emit), do NOT
      contort a fixture — flag it to A as a candidate line-level exception with the
      reason, per the overview's except-vs-backfill rule.
- [ ] **mod.rs**: cover the driver edges A's report flags. Known candidate:
      **`TYPE_RESOURCE_RETURN_ORDER`** (`mod.rs:378`), reached only when a
      function's `resource_owners` map carries a `ResOwner::FloatBlocked(collection)`
      — a resource returned inside a collection declared after it. Also confirm the
      `emit` sites at `mod.rs:258/280/299/309/332` (the `TYPE_FUNC_REQUIRES_RETURN_TYPE`
      / `TYPE_FUNC_MISSING_RETURN` / `TYPE_PARAM_REQUIRES_TYPE` / `TYPE_DEFAULT_ARG_ORDER`
      / `TYPE_DEFAULT_VALUE_MISMATCH` arms) are covered; if A's report shows a
      specific arm uncovered, add its fixture.

Acceptance: fresh `sh scripts/coverage.sh`, then
`sh scripts/coverage-check.sh src/ir/verify/compat.rs src/ir/verify/mod.rs`
shows both ≥95%.
Commit: —

### Phase D7 — `src/arch/x86_64/encode/emitter.rs` (1471/1554, 83 uncov) — added by A

A1 found this file below the floor and in no plan-68 doc (overview Corrections);
assigned **backfill:D** for arch cohesion with D1. It is a pure in-memory x86-64
instruction encoder — no I/O, no integration boundary. Read A's fresh
`coverage.json` (or `awk '/^SF:.*x86_64\/encode\/emitter.rs$/{f=1} f&&/^DA:[0-9]+,0$/
{print} /^end_of_record/{if(f)exit;f=0}' target/coverage/lcov.info`) for the exact
uncovered lines, then cover them by emitting the un-exercised instruction forms /
error arms directly against the emitter's public/`pub(crate)` surface.

- [ ] From A's fresh report, enumerate emitter.rs's uncovered lines and group them
      by the encode helper they sit in (ModRM/SIB forms, REX-prefix arms, immediate
      widths, displacement sizes, the rejected/`unreachable`-guard arms).
- [ ] Add a `#[cfg(test)] mod tests` (or extend the nearest existing arch test
      module) that drives each uncovered encode arm and asserts the exact emitted
      byte sequence; for any genuinely-unreachable defensive/`unreachable!` arm,
      flag it to A as a line-level exception with the reason (do NOT fabricate an
      invalid instruction to reach it).

Acceptance: fresh `sh scripts/coverage.sh`, then
`sh scripts/coverage-check.sh src/arch/x86_64/encode/emitter.rs` shows ≥95%.
Commit: —

## Validation Plan

- **Per-phase:** a fresh `sh scripts/coverage.sh` (the profile the checker reads),
  then `sh scripts/coverage-check.sh <path…>` for the phase's files → each ≥95%.
- **Whole sub-plan:** `sh scripts/coverage-check.sh src/arch/aarch64/backend.rs
  src/ir/types.rs src/ir/verify/` → all eight files ≥95% (none appears as a GATE
  FAILURE).
- **Suite:** `cargo test` → `0 failed` (run the FULL suite, never one module —
  `AGENTS.md`; new tests must not regress it).
- **No behavior change:** the diff is confined to `#[cfg(test)]` modules
  (`src/ir/verify/tests.rs`, a new `mod tests` in `backend.rs`, and possibly
  `types.rs`). Any non-test `src/**` change must be a bug fix on its own commit
  with a RED-first test — not a workaround to make coverage pass.
- **Exceptions audit:** if any arm is proven unreachable-by-construction and
  handed to A for a line-level exception, that exception names the concrete reason
  a unit test cannot reach it. No file in this set is expected to need one (the
  aarch64 shim, the Open Decision's flagged candidate, is coverable — D1).

## Corrections

<Filled in during execution — including any arm A's fresh report proves
unreachable (→ A exception) rather than backfill, and any delta between the
covered/total figures above and A1's regenerated report.>
