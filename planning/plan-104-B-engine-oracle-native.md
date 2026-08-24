# plan-104-B: Native ParameterType in the codegen engine (oracle + builder)

Last updated: 2026-08-24
Effort: large (3h–1d)
Depends on: plan-104-A (NIR fields are `ParameterType`; consumers are shimmed —
this sub-plan replaces the engine's shims with native operations).

Convert the codegen **engine**'s type machinery from strings to native
`ParameterType`: the central static type oracle (`static_nir_value_type`), the
builder's per-local type store (`LocalValue.type_`), the `FieldTypes` field-type
map, and the engine's read sites (123 `.type_` reads, 8 scalar-compare files).
After this sub-plan the engine tracks and compares types as `ParameterType`
end-to-end; strings survive only at the engine's outward seams (registry calls
until C, error-message formatting, symbol mangling).

See plan-104-A §3 for the full layering, the shared byte-identity gate, and the
A→D roadmap. See plan-104-A §Prerequisites for the shared gate.

References:

- `src/codegen/engine/types/type_utils.rs` — `static_nir_value_type` (`:19`),
  `FieldTypes` (`:16`), `numeric_binary_result_type` (`:414`).
- `src/codegen/engine/builder/mod.rs` — `LocalValue` (`:414`), the builder's
  maps (`package_return_types` `:118` is type-valued; `function_symbols`/
  `string_symbols`/`platform_imports` are symbol maps and stay `String`).
- `src/codegen/engine/analysis/module_analysis.rs` — the module-level walks
  threading `locals: &HashMap<String, String>` (`:398`).
- `.ai/codegen-invariants.md` — engine invariants (regalloc/lifetimes).

## Prerequisites

See plan-104-A §Prerequisites (shared). Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-104-A complete | `rg -c 'type_: String' src/target/shared/nir/mod.rs` → 0; A's phases all `[x]` with Commit hashes | NOT MET until A lands |

## 1. Goal

- `static_nir_value_type` returns `Option<ParameterType>`; its 12 callers
  (`rg -c 'static_nir_value_type' src/` → 12) consume the typed result; its
  internal recursion is structural (no `format!`/`strip_prefix` on the represented
  shapes).
- `LocalValue.type_` is `ParameterType`; the 11 construction sites
  (`rg -c 'LocalValue \{' src/codegen/` → 11) store the NIR node's typed field
  directly; the engine's 123 `.type_` reads
  (`rg -c '\.type_\b' src/codegen/engine/` → 123) match natively.
- `FieldTypes` values are `ParameterType`
  (`HashMap<(String, String), ParameterType>`); the module-analysis walks thread
  `HashMap<String, ParameterType>` locals.
- The engine's 8 scalar-compare files (see A's census) use variant matches, not
  string equality; `numeric_binary_result_type` gains a `ParameterType` form.

### Non-goals (explicit constraints)

- No change to compiled output — byte-identical vs the plan-104 baseline.
- Symbol/name maps stay `String` (`function_symbols`, `string_symbols`,
  `platform_imports`, and any map whose values are *names*, not type
  spellings — the Phase-1 triage census decides each).
- Error/diagnostic message text renders `name()` at the format site — wording
  unchanged.
- Builtins (`src/codegen/builtins/`) keep their A-phase shims — that sweep is C.
  Where an engine seam calls into builtins helpers taking `&str` types
  (`resolve_call_return_type`, `element_accepts_item`, …), render `.name()` at
  the call — C retypes those boundaries.

## 2. Current State

Post-A, the engine reads `ParameterType` NIR fields through `.name()` shims and
still stores/threads types as `String`: `LocalValue.type_: String`
(`builder/mod.rs:415`), `FieldTypes = HashMap<(String,String),String>`
(`type_utils.rs:16`), `static_nir_value_type(… ) -> Option<String>`
(`type_utils.rs:19` — synthesizing container strings via `format!` and calling
the registry return-type oracle with `Vec<String>` arg types), and
module-analysis walks over `HashMap<String, String>` locals
(`module_analysis.rs:398`).

### Measured populations

| What | Count | Command |
|---|---|---|
| engine `.type_` reads | 123 | `rg -c '\.type_\b' src/codegen/engine/ \| awk -F: '{s+=$2} END{print s}'` → 123 |
| `static_nir_value_type` callers | 12 | `rg -c 'static_nir_value_type' src/ \| awk -F: '{s+=$2} END{print s}'` → 12 |
| `LocalValue` construction sites | 11 | `rg -c 'LocalValue \{' src/codegen/ \| awk -F: '{s+=$2} END{print s}'` → 11 |
| engine scalar-compare files | 8 | `rg -l '== "(Integer\|…\|Scalar)"' src/codegen/engine/` → 8 files (convert/control/builder×2/function/value/analysis/operators) |
| engine `HashMap<String, String>` occurrences (type-valued + symbol mixed) | UNMEASURED per-map | Phase-1 triage: `rg -n 'HashMap<String, String>' src/codegen/engine/` and READ each declaration — only type-valued ones convert |

### Verified properties

- **`package_return_types` is type-valued; `function_symbols`/`string_symbols`/
  `platform_imports` are symbol maps.** Read `builder/mod.rs:84-129`. The
  remaining engine maps (`float_residents`, `promoted_float_locals`,
  `len_of_local`, `provable_index_locals`, …) are UNVERIFIED as to which side
  they fall on — Phase 1's triage census reads each declaration and its
  writers before converting anything.
- **`static_nir_value_type`'s registry fallback passes `Vec<String>` arg
  types** into `builtins::resolve_call_return_type` (`type_utils.rs:41-60`) —
  in B this seam renders `.name()` per arg; C retypes it. VERIFIED (read).

## 3. Design Overview

Convert inside-out: the stores first (`LocalValue`, `FieldTypes`, locals maps),
then the oracle, then the read sites — each step compile-driven, all landed
together as one byte-identical sub-plan (the stores and readers are too
interleaved to gate separately).

- `static_nir_value_type(value, locals: &HashMap<String, ParameterType>,
  fields: &FieldTypes) -> Option<ParameterType>`: structural rebuild —
  containers via `ParameterType::list_of(...)` etc. instead of `format!`;
  `numeric_binary_result_type` gets a typed twin mapping scalar variants
  (byte-identical result by the parse↔name bijection); the registry fallback
  renders `.name()` per argument (until C).
- `LocalValue.type_: ParameterType`; binds clone the NIR node's field (the A
  shim `.name().into_owned()` at the 11 construction sites is deleted).
- Scalar compares → `matches!(t, ParameterType::Integer | …)`; structural tests
  → variant matches (`ParameterType::ListOf(elem)` instead of
  `strip_prefix("List OF ")`).

**Correctness risk:** the builder's local types feed layout/width decisions
(float promotion, inline sizing) — a wrong variant match here changes emitted
code, which is exactly what the gate catches. Root-cause any diff against ONE
fixture (objdump/`.ncode` diff) before touching anything else.

### Rejected alternatives

- **Convert readers file-by-file across multiple sub-plans while stores stay
  String.** Rejected: every intermediate state doubles conversions at the
  store/reader boundary; the interleaving makes one sub-plan cheaper than three.

## Compatibility / Format Impact

None externally observable.

## Phases

### Phase 1 — map triage census

- [x] Enumerate every `HashMap<String, String>` in `src/codegen/engine/`
      (`rg -n 'HashMap<String, String>' src/codegen/engine/`), read each
      declaration + writers, and record in this file which are **type-valued**
      (convert) vs **symbol/name-valued** (stay). This bounds Phase 2's scope.

**Census (2026-08-24; declaration + writers read for each):**

Type-valued — CONVERT in Phase 2:

| Map | Where | Value is |
|---|---|---|
| `package_return_types` | `builder/mod.rs:118` (built `:741`) | link-function/import return-type spelling |
| module-analysis `locals`/`types` walks | `analysis/module_analysis.rs:302,403,713,927,999` | local name → type spelling |
| oracle `locals` | `types/type_utils.rs:20` (`static_nir_value_type`) | local name → type spelling |
| helper `types` maps | `types/type_utils.rs:135,194,225` | local name → type spelling |
| `FieldTypes` | `types/type_utils.rs:16` (`HashMap<(String,String),String>`) | field type spelling |
| `TypeModel.record_fields` / `union_variant_fields` | `builder/mod.rs:617,622` (`Vec<(String,String)>` values) | per-field type spellings |

Symbol/name-valued — STAY `String`:

| Map | Where | Value is |
|---|---|---|
| `platform_imports` | `builder/mod.rs:84,119` (+ threaded everywhere) | platform import symbol |
| `function_symbols` | `builder/mod.rs:116` | text symbol |
| `string_symbols` | `builder/mod.rs:129` | data-object symbol |
| `float_residents` | `builder/mod.rs:158` (writers: `builder_values.rs:412`, `builder_numeric.rs:213,216`) | FP vreg render (`d`-register name) |
| `promoted_float_locals` | `builder/mod.rs:164` (writer `builder_control.rs:1812`) | `%fN` register render |
| `len_of_local` | `builder/mod.rs:401` (writer `builder_control.rs:425`) | container **local name** (`n → L`) |
| `union_variants` | `builder/mod.rs:619` | union **nominal name** (bare identifier, no structure; keys into name-keyed model maps) |
| `resource_closers` | `builder/mod.rs:659` (reader `:986`) | close **function name** |
| `get_container`/`copy_src` | `function_lowering.rs:229,230` | local names |
| `err_binding` | `function_lowering.rs:401` | local name |
| `tests/test_support.rs` platform_imports params | test twins | follow production |

Acceptance: the tagged map list is recorded in this section.
Commit: 7c0ecf8fa

### Phase 2 — stores + oracle + engine read sites native

- [x] `LocalValue.type_` → `ParameterType` (`builder/mod.rs`); repoint the 11
      construction sites and the engine's `.type_` reads. **Also
      `GlobalValue.type_`** (LocalValue's global twin, missed by the
      HashMap-shaped census — same store family, converted with it), and the
      FUNC-callable local/global call arms went structural
      (`ParameterType::Func(_, _, false)` match preserving the plain-`FUNC(`
      isolation exclusion) with the return type destructured, not string-split.
- [x] `FieldTypes` values → `ParameterType`; `module_field_types` builder and
      the module-analysis walks thread `HashMap<String, ParameterType>` (the
      data_objects pre-pass walks sharing those maps flipped their `types`
      params with it).
- [x] `static_nir_value_type` → `Option<ParameterType>` (typed
      `typed_numeric_binary_result_type` twin — renders operand names, runs the
      one `numeric` algorithm, maps the closed scalar result set back with a
      static match, no parse; `ResultOf`/`MapEntryOf`/`ThreadHandle` arms
      structural; registry fallback renders `.name()` per arg + parses the
      resolved return — the plan-104-C boundary, commented as such); the 12
      callers consume the typed result.
- [x] Convert the type-valued maps from Phase 1's census; leave symbol maps.
      (`package_return_types` parses the LINK block's C-boundary strings at
      build and renders at its two readers — the reader chains merge `&str`
      params from builtins, C's seam. `TypeModel.record_fields`/
      `union_variant_fields` values typed; `link_thunk`'s three signatures
      followed — it reads only field names.)
- [x] Native matches for the engine's scalar compares and structural tests
      **where the operand is a typed store**; the survivors are annotated
      below.
- [x] Tests: engine unit tests compile untouched (`cargo check --all-targets`
      0 errors/warnings); the one link_thunk fixture builds typed field lists.

**Survivor annotation (measured post-conversion):** engine scalar-compare
census `rg -n '== "(Integer|…|Scalar)"' src/codegen/engine/` → **24**, and
structural-test census → **15**; every one operates on a `String`/`&str`
carrier, none on a typed store:

- `ValueResult.type_` reads (13 compares + 3 structural): the lowering
  interchange struct stays `String` in B — it is the builtins boundary, typed
  no earlier than C/D.
- `type_utils`' shared string-vocabulary helpers (11 structural sites:
  `is_collection_type`/`list_element_type`/`map_type_parts`/…): serve every
  still-shimmed tree; C converts their codegen call sites (plan-104-C §3
  Rejected alternatives keeps the helpers).
- `emit_call`/`emit_call_result` `result_type` chains (3 compares): merge an
  `Option<&str>` parameter passed by builtins callers — C's seam.
- `ProgramEntrySpec.language_entry_returns: &str` (2 compares): consumed by the
  per-arch backends — D's scope.
- `builder_numeric` string-algorithm internals (3 compares incl.
  `numeric_binary_result_type(...) == "Float"` on `static_type_name` strings),
  `is_freeable_flat_value`/`type_requires_empty_string_constant` `&str`-param
  helpers (2), `IrLinkFunction.return_type` wire string (1),
  `validation.rs` `&str`-param walk (1 structural).

Acceptance: `cargo test --no-fail-fast` green; `artifact-gate all` no NEW diff
vs `planning/plan-104-baseline-diffs.txt`; engine scalar-compare census
(`rg -n '== "(Integer|…)"' src/codegen/engine/`) drops to 0 or each survivor is
annotated as a deliberate string boundary (record the number).
Commit: —

## Validation Plan

- Tests: engine unit suite + full integration suite.
- Coverage check: every fixture lowers through the builder/oracle — the gate
  corpus is the coverage.
- Runtime proof: `artifact-gate all` byte-identical vs baseline.
- Doc sync: none in B (D owns the `.ai` updates).
- Acceptance: `cargo test --no-fail-fast`; `artifact-gate all`; fmt both crates.

## Open Decisions

- **`numeric_binary_result_type`: replace vs add a typed twin.** Recommend a
  typed twin and delete the string form when its last caller converts (C/D) —
  keeps each sub-plan's diff minimal. (§3)

## Corrections

<Filled in during execution.>

## Summary

The engine's type plumbing goes native behind the strongest gate available.
Risk concentrates where local types feed layout/width decisions; the mechanics
are the proven plan-102 pattern. Builtins/memory/backends stay shimmed until
C/D.
