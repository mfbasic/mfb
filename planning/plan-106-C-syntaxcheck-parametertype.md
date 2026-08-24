# plan-106-C: Replace syntaxcheck's private Type enum with ParameterType

Last updated: 2026-08-24
Effort: large (3h–1d)
Depends on: plan-106-B (verify is typed; syntaxcheck is the last engine on a
private representation).

Delete the compiler's **sixth type representation**: `syntaxcheck`'s private
`enum Type` (`src/syntaxcheck/mod.rs:27` — scalars, `List`/`Set`/`Map`/`Res`/
`Function`/`Result`/`Thread`/`ThreadWorker`/`User(String)` + `Error`/`ErrorLoc`)
and its private 1,077-line parser (`src/syntaxcheck/types.rs`), replacing both
with `crate::types::ParameterType` and the canonical `parse`. Syntaxcheck is
already structurally typed *internally* — this is an enum **swap** plus a
parser deletion, not a 14k-line stringly rewrite; the discovery that makes
plan-106 tractable.

See plan-106-A for the roadmap, shared prerequisites, and the terminal
no-strings invariant.

References:

- `src/syntaxcheck/mod.rs:27-…` — the private `Type` enum (read the whole
  definition; note `Error`/`ErrorLoc` variants and
  `Thread(msg, res, res_state, out)`'s four-slot shape vs `ParameterType`'s
  `ThreadHandle`).
- `src/syntaxcheck/types.rs` (1,077 lines) — `parse_type`/
  `parse_collection_element_type`/`parse_function_type` + their unit tests: the
  private grammar this letter deletes.
- `src/syntaxcheck/inference.rs` (2,831 lines, 153 `Type` references) — the
  canonical AST checker engine, already enum-driven.
- `src/syntaxcheck/helpers.rs:272` — `numeric_binary_result_type(op, &Type,
  &Type) -> Type` (the sixth promotion copy; E deletes it onto numeric.rs once
  the enum matches).
- `src/types.rs` — `ParameterType` (post-105-B: includes `UserOf`).

## Prerequisites

See plan-106-A §Prerequisites (shared). Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-106-B complete | verify typed; B's boxes ticked | NOT MET until B lands |
| plan-105-B's `UserOf` exists | `rg -n 'UserOf' src/types.rs` → hit | NOT MET until 105 lands (already gated at 106 level) |

## 1. Goal

- `enum Type` and `src/syntaxcheck/types.rs`'s parser **do not exist**;
  syntaxcheck's rule modules and `inference.rs` operate on `ParameterType`.
- Diagnostics are byte-identical (codes, wording, order) across the full
  `*-invalid` corpus — the checker's accept/reject set is unchanged.
- Where syntaxcheck's enum is richer or differently shaped than
  `ParameterType`, the mapping is explicit and recorded (see §3): `Error`/
  `ErrorLoc` map to the same `Named` spellings the rest of the compiler uses;
  the thread shape's `res_state` slot maps onto `ThreadHandle`'s existing
  RES/STATE modelling (the plane's STATE rides the res type exactly as
  `parse`/`name` already round-trip it).

### Non-goals (explicit constraints)

- No behavior change: same programs accepted/rejected, same diagnostics.
- Syntaxcheck still consumes the **AST** in this letter (parsing source-string
  type fields via the canonical `ParameterType::parse` where its private parser
  did) — the HIR input switch is D's scope. One representation change at a
  time.
- No rule relocation between syntaxcheck and `ir::verify`
  (`RELOCATED_TO_IR_VERIFY` untouched).

## 2. Current State

Syntaxcheck parses AST type strings ONCE at its own boundary
(`types.rs::parse_type`, scope-aware — it resolves user names against its
symbol tables, which is why it has `User(String)`), then runs its 124-rule
engine over its private enum. The enum is near-isomorphic to `ParameterType`;
the differences are enumerable and small.

### Measured populations

| What | Count | Command |
|---|---|---|
| private parser to delete | 1,077 lines | `wc -l src/syntaxcheck/types.rs` |
| `Type` references in the engine | 153 (inference.rs) | `rg -c '\bType\b' src/syntaxcheck/inference.rs` → 153; whole-module count at kickoff: `rg -c '\bType\b' src/syntaxcheck/` |
| syntaxcheck total | 14,441 lines | `find src/syntaxcheck -name '*.rs' \| xargs wc -l` |
| enum variant delta vs `ParameterType` | to enumerate | Phase 1 task: read both enums side-by-side; record the full mapping table here |
| `HashMap<String, String>` in syntaxcheck | 2 | `rg -c 'HashMap<String, String>' src/syntaxcheck/` → 2 (triage in Phase 2: type-valued vs name-valued) |
| diagnostic goldens guarding this | 124 rules / every `*-invalid` fixture | plan-102-F census |

### Verified properties

- **The enum is near-isomorphic** — read side-by-side at plan-writing:
  scalars match; `List/Set/Map/Res/Function/Result` match
  (`Function{params,return,isolated}` ≡ `Func(Vec,Box,bool)`); `User(String)` ≡
  post-105 `Named`/`UserOf`; `Thread(msg, res, res_state, out)` carries
  `res_state` separately where `ThreadHandle{worker,msg,res,out}` folds STATE
  into the res type's spelling. UNVERIFIED remainder: whether any syntaxcheck
  rule DISTINGUISHES `res` from `res_state` in a way the folded spelling
  cannot — Phase 1 reads every `Thread(` match arm and records the answer
  before the swap. If a genuine expressiveness gap exists, the fix is a
  `ParameterType` accessor (splitting the folded STATE back out
  structurally), never a parallel enum.
- **`User(String)` is scope-resolved at parse time** (its parser consults
  symbol tables). The swap must preserve WHERE resolution happens: syntaxcheck
  keeps resolving names, then constructs `Named`/`UserOf` — the canonical
  parse handles grammar, syntaxcheck handles scope, same as `elaborate`'s
  split. Phase 1 verifies this boundary by reading `parse_type`'s
  symbol-table touches.

## 3. Design Overview

A mechanical enum swap executed like plan-102's ports: alias first, then
migrate, then delete.

1. **Mapping table** (Phase 1): every `Type` variant → its `ParameterType`
   form, with the two known deltas resolved (`Error`/`ErrorLoc` → `Named`;
   thread `res_state` → folded spelling + accessor if any rule needs the
   split).
2. **Swap** (Phase 2): `type Type = ParameterType` is NOT enough (variant
   names/shapes differ) — convert module-by-module (`types.rs` callers first,
   then `inference.rs`, then the rule modules), compile-driven, with the
   private parser reduced to scope-resolution + canonical `parse` and finally
   deleted.
3. `helpers.rs`'s promotion copy converts to the numeric.rs typed source
   (E deletes the last copy once codegen's falls in 104).

**Correctness risk:** the highest of plan-106 — 124 rules' worth of
comparisons changing representation. Held by the strongest corpus in the
repo: every `*-invalid` fixture byte-compares the full diagnostic stream, and
accepted programs byte-compare through the gate. Convert incrementally
(module-per-commit) so a corpus failure localizes.

### Rejected alternatives

- **Keep the private enum, add a converter at the edges.** Rejected: that is a
  SEVENTH representation's worth of conversion code and none of the drift
  protection; the review's complaint is the multiplicity itself.
- **Do the HIR input switch simultaneously.** Rejected: two representation
  changes in one diff makes corpus failures unattributable; D does the input
  switch against an already-`ParameterType` checker.

## Compatibility / Format Impact

None. Diagnostics byte-identical.

## Phases

### Phase 1 — mapping table + thread-shape verification

- [ ] Read both enums + every `Thread(`/`ThreadWorker(` match arm in
      syntaxcheck; record the complete variant mapping table here, including
      the `res_state` resolution (folded spelling or new accessor).
- [ ] Read `types.rs::parse_type`'s symbol-table touches; record the
      grammar-vs-scope split.

Acceptance: the mapping table exists in this section with no UNVERIFIED rows.
Commit: —

### Phase 2 — the swap, module by module

- [ ] Convert `types.rs` callers → canonical parse + scope resolution;
      convert `inference.rs`; convert the rule modules (`checking.rs`,
      `resources.rs`, `builtins.rs`, `link.rs`, `helpers.rs`); delete
      `enum Type` and the private parser (+ its 1,077 lines of tests, ported
      to `ParameterType` where they cover grammar the canonical tests lack).
- [ ] `helpers.rs` promotion copy → the numeric.rs typed source.
- [ ] Tests: the full `*-invalid` diagnostic corpus after EVERY module commit.

Acceptance: `cargo test --no-fail-fast` green; diagnostic corpus
byte-identical; `artifact-gate all` no NEW diff; `rg -n 'enum Type' src/syntaxcheck/`
→ 0; `wc -l src/syntaxcheck/types.rs` → file deleted or reduced to
scope-resolution only (record which).
Commit: —

## Validation Plan

- Tests: full diagnostic corpus per module commit; ported grammar tests.
- Coverage check: 124/124 rules golden-guarded (measured, plan-102-F).
- Runtime proof: `artifact-gate all`; `test-accept` no NEW mismatch.
- Doc sync: none (E owns docs).
- Acceptance: full suite; gate; test-accept; fmt both crates.

## Open Decisions

- **Where scope-resolution lives after the parser dies:** a thin
  `syntaxcheck::resolve_type_name(&str, scope) -> ParameterType` that calls
  canonical `parse` then classifies user names, vs inlining at call sites.
  Recommend the thin fn — one seam, testable.

## Corrections

<Filled in during execution.>

## Summary

The sixth representation dies. The engine is already enum-shaped, so this is a
swap with a 124-rule golden corpus holding every comparison steady; the one
real design question (the thread `res_state` slot) is Phase 1's first read,
recorded before any code moves.
