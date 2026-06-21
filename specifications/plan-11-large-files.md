# Plan: Split Large Source Files into Module Folders

Several source files have grown past ~1,000 lines and mix multiple
responsibilities. This document is the plan for breaking each one into a
folder of focused submodules.

## Goal

For each oversized `src/foo.rs`, convert it into a `src/foo/` directory:

- `src/foo.rs` becomes `src/foo/mod.rs` (the parent module).
- Logical groupings move into sibling files: `src/foo/types.rs`,
  `src/foo/lower.rs`, etc.
- The parent `mod.rs` keeps the shared type definitions, declares the
  submodules (`mod lower;`), and `pub use`-re-exports the public surface so
  that **no callers outside the module need to change**.

Target: no single file over ~1,000 lines where a clean seam exists; a few
cohesive files may stay larger when splitting would only create false
boundaries.

## Conventions & guardrails

- **Reserved names.** `type`, `match`, `loop`, `mut`, `move` are Rust
  keywords and cannot be module file names. Use `types.rs`, `match_.rs` (or
  fold match types into `value.rs`), etc. The original ask said
  `src/ir/type.rs` — this plan uses `src/ir/types.rs`.
- **Keep shared types in `mod.rs`.** Structs/enums referenced across many
  submodules (e.g. `IrProject`, `TypeChecker`, `NativeCodePlan`) stay in the
  parent and are imported by submodules via `use super::*;` or specific paths.
- **Preserve the public API.** Re-export everything that's currently `pub`
  from `mod.rs` so external `use crate::ir::Foo` paths keep working. Verify
  with `cargo build` after each file move (zero behavior change).
- **Move tests with their code**, or into a `tests.rs` submodule under the
  folder when they're broad.
- **One file at a time**, compiling between moves, so a mistake is easy to
  bisect. Pure code relocation — no logic changes in this effort.
- **Line ranges below are approximate** (snapshot at planning time) and exist
  to identify the seams; they will drift as the move proceeds.

## Suggested order

Work roughly largest-first / highest-pain-first. The `target/shared/code/`
hub is the biggest win but also the riskiest, so the front-end files
(`ir`, `ast`, `typecheck`) are good warm-ups that establish the pattern.

1. `src/ir.rs`
2. `src/ast.rs`
3. `src/typecheck.rs`
4. `src/binary_repr.rs`
5. `src/main.rs`
6. `src/target/shared/{plan,runtime,nir}.rs`
7. `src/target/shared/code/mod.rs` + `builder_misc.rs` + `net.rs` + `builder_collection_updates.rs`
8. `src/arch/aarch64/encode.rs`
9. `src/audit/collect.rs`
10. `src/resolver.rs`, `src/monomorph.rs` (lighter splits)

Files left as-is (already cohesive; revisit only if they grow):
`validate.rs`, `builder_strings.rs`, `builder_values.rs`, `builder_numeric.rs`,
`builder_collection_queries.rs`, `builder_collection_layout.rs`.

---

# Inventory (files > 1000 lines)

| Lines | File | Plan |
|------:|------|------|
| 14,899 | `src/target/shared/code/mod.rs` | split → `code/` submodules |
| 6,568 | `src/typecheck.rs` | split → `typecheck/` |
| 5,807 | `src/ir.rs` | split → `ir/` |
| 5,103 | `src/ast.rs` | split → `ast/` |
| 3,928 | `src/binary_repr.rs` | split → `binary_repr/` |
| 3,008 | `src/target/shared/code/builder_misc.rs` | split → 4 files |
| 2,585 | `src/main.rs` | split → `cli/` + `manifest/` |
| 2,126 | `src/target/shared/plan.rs` | split → `plan/` |
| 1,827 | `src/target/shared/runtime.rs` | split → `runtime/` (per category) |
| 1,674 | `src/monomorph.rs` | light split → `monomorph/` |
| 1,645 | `src/target/shared/code/net.rs` | split → 2 files |
| 1,629 | `src/target/shared/nir.rs` | split → `nir/` |
| 1,621 | `src/target/shared/code/builder_collection_updates.rs` | split → mutate/query |
| 1,605 | `src/target/shared/validate.rs` | **keep as-is** |
| 1,511 | `src/resolver.rs` | light split → `resolver/` |
| 1,441 | `src/target/shared/code/builder_strings_package.rs` | optional split |
| 1,394 | `src/target/shared/code/builder_strings.rs` | **keep as-is** |
| 1,256 | `src/target/shared/code/builder_values.rs` | **keep as-is** |
| 1,174 | `src/target/shared/code/builder_numeric.rs` | **keep as-is** |
| 1,129 | `src/target/shared/code/builder_collection_queries.rs` | **keep as-is** |
| 1,121 | `src/arch/aarch64/encode.rs` | split → `encode/` |
| 1,112 | `src/target/shared/code/builder_collection_layout.rs` | **keep as-is** |
| 1,073 | `src/audit/collect.rs` | split → `collect/` |

---

# Per-file split plans

## `src/ir.rs` → `src/ir/` (5,807 lines)

Defines the IR plus its lowering, JSON, and binary-repr encoding. Natural
seams between type definitions, the lowering pass, and the two serializers.

| New file | Lines | Responsibility |
|----------|------:|----------------|
| `ir/mod.rs` | — | Re-export public API; keep the small shared structs and the `IrProject`/`EntryPoint`/`IrFunction` definitions if convenient. |
| `ir/types.rs` | ~360 | Core type defs: `IrType`, `IrBinding`, `IrField`, `IrVariant`, `IrEnumMember`, `IrParam`, `IrSourceLoc`, `IrRecordUpdate`, `ExternalFunctionParam`. |
| `ir/op.rs` | ~80 | `IrOp` (statement variants). |
| `ir/value.rs` | ~100 | `IrValue` (expression variants) + `IrMatchCase`/`IrMatchPattern`. |
| `ir/link.rs` | ~70 | Native `LINK` types: `IrLinkFunction`, `IrNativeResource`, `IrAbiSlot`, `IrLinkExpr`. |
| `ir/lower.rs` | ~2,600 | AST→IR lowering: `lower_project_with_external_functions` and all `lower_*` helpers. |
| `ir/json.rs` | ~720 | `ToIrJson` trait and impls. |
| `ir/binary.rs` | ~1,440 | `encode_binary_repr`/`decode_binary_repr`, verification, identity prefixing. |
| `ir/package.rs` | ~280 | `prefix_package_symbols`, `apply_package_identity`, `merge_package`. (A `repository/src/package.rs` already exists for repo side — keep names distinct.) |
| `ir/tests.rs` | ~445 | The `#[cfg(test)]` block. |

`lower.rs` and `binary.rs` stay large but cohesive; acceptable for a first
pass. Revisit `lower.rs` only if it keeps growing.

## `src/ast.rs` → `src/ast/` (5,103 lines)

Type definitions + recursive-descent parser + manifest discovery + JSON. Split
the parser by grammar level.

| New file | Lines | Responsibility |
|----------|------:|----------------|
| `ast/mod.rs` | ~100 | Re-exports; public `parse_source`/`write_ast`. |
| `ast/types.rs` | ~500 | All AST node type definitions + trivial accessor impls. |
| `ast/manifest.rs` | ~275 | `parse_project`, source-file discovery, glob matching, path validation. |
| `ast/parser.rs` | ~250 | `FileParser` struct, token stream, lookahead, error reporting, `synchronize`. |
| `ast/items.rs` | ~1,200 | File entry + top-level item parsing (function/type/binding/resource/link/abi). |
| `ast/stmt.rs` | ~700 | Statement & block parsing (if/match/for/while/do/trap). |
| `ast/expr.rs` | ~700 | Expression precedence climbing, calls, literals, type annotations, lambdas. |
| `ast/lexical.rs` | ~150 | Identifier/keyword consumption helpers. |
| `ast/serialize.rs` | ~550 | `to_json` / `ToAstJson` impls. |

## `src/typecheck.rs` → `src/typecheck/` (6,568 lines)

| New file | Lines | Responsibility |
|----------|------:|----------------|
| `typecheck/mod.rs` | ~700 | Public API, the `TypeChecker` struct + the shared enums/structs (`Type`, `OwnershipState`, `FunctionSig`, `TypeInfo`, …), init/collect, top-level checking, visibility. |
| `typecheck/types.rs` | ~500 | `parse_type`/`parse_function_type`, compatibility (`compatible`, `is_numeric`, `is_comparable`). |
| `typecheck/checking.rs` | ~1,100 | Statement & block checking, control flow, ownership tracking. |
| `typecheck/inference.rs` | ~1,600 | Expression inference (the big match), pattern matching, literals. |
| `typecheck/builtins.rs` | ~700 | Per-package builtin call checkers (fs/net/json/io/thread/strings/math/general). |
| `typecheck/resources.rs` | ~600 | Resource/ownership rules, copyability, thread sendability + boundary checks. |
| `typecheck/helpers.rs` | ~450 | Standalone utilities, literal parsing, error formatting. |

Keep `inference.rs` as one file even at ~1,600 lines — it's a single dispatch
match that doesn't split cleanly.

## `src/binary_repr.rs` → `src/binary_repr/` (3,928 lines)

Split by data-flow direction: read, interpret, write, encode.

| New file | Lines | Responsibility |
|----------|------:|----------------|
| `binary_repr/mod.rs` | ~330 | Public API, all `BinaryRepr*` type defs, section/type-id constants. |
| `binary_repr/reader.rs` | ~850 | MFP container + per-section decoders, type-name decoding, validation. |
| `binary_repr/builder.rs` | ~180 | Convert decoded structures → public export/info types. |
| `binary_repr/writer.rs` | ~1,200 | IR → BinaryRepr lowering, resource/type collection, sig hashing, final serialize. |
| `binary_repr/sections.rs` | ~800 | `StringPool`/`TypeTable`/`ConstPool`/`ResourceTable`/`ImportTable`/`AbiIndex` encoders. |
| `binary_repr/util.rs` | ~100 | Cursor/checked readers, `put_*` writers, hash/hex helpers. |

## `src/main.rs` → thin `main.rs` + `src/cli/` + `src/manifest/` (2,585 lines)

Keep `main.rs` as a thin dispatcher; move command handlers and manifest logic
into modules.

| New file | Lines | Responsibility |
|----------|------:|----------------|
| `main.rs` | ~100 | `main()` dispatcher + shared `json_string` util. |
| `cli/build.rs` | ~425 | `BuildOptions`/`BuildOutput`, `parse_build_options`, `build_project` pipeline, signing. |
| `cli/init.rs` | ~150 | `init`/`init-pkg` scaffolding, manifest/source templates, name sanitization. |
| `cli/pkg.rs` | ~400 | `pkg add/info/verify/publish` dispatch + verification helpers. |
| `cli/repo.rs` | ~50 | `repo register/auth`. |
| `cli/man.rs` | ~130 | `man` index/package/function/page rendering. |
| `manifest/mod.rs` | ~550 | `parse_project_json`, manifest validation, field validators, accessors. |
| `manifest/package.rs` | ~700 | `MfpHeader` + `.mfp` reading, installed-package discovery, external function types, dependency JSON editing. |
| `manifest/entry.rs` | ~125 | `validate_entry_point`. |

## `src/target/shared/plan.rs` → `plan/` (2,126 lines)

| New file | Lines | Responsibility |
|----------|------:|----------------|
| `plan/mod.rs` / `types.rs` | ~150 | `NativePlan`, `PlannedFunction`, `StorageType`, … + `NativePlanPlatform` trait. |
| `plan/lower.rs` | ~350 | `lower_module_for_platform`, type-storage mapping. |
| `plan/symbols.rs` | ~250 | Runtime/platform symbol collection, native constant folding, net→libc mapping. |
| `plan/function_builder.rs` | ~350 | `FunctionPlanBuilder` (per-function lowering). |
| `plan/json.rs` | ~150 | `ToPlanJson` impls. |

## `src/target/shared/runtime.rs` → `runtime/` (1,827 lines)

Split the helper catalog by category (this matches the `RuntimeHelper` enum).

| New file | Lines | Responsibility |
|----------|------:|----------------|
| `runtime/mod.rs` / `types.rs` | ~100 | `RuntimeHelper`, `RuntimeHelperSpec`, `RuntimeHelperAbi`, symbol generation. |
| `runtime/fs_specs.rs` | ~350 | Filesystem helper ABI specs. |
| `runtime/io_specs.rs` | ~150 | IO helper specs. |
| `runtime/strings_specs.rs` | ~100 | Strings helper specs. |
| `runtime/thread_specs.rs` | ~200 | Threading helper specs. |
| `runtime/net_specs.rs` | ~200 | Networking helper specs. |
| `runtime/catalog.rs` | ~100 | `supported_helper_specs`, `spec_for_symbol`/`spec_for_call` lookup. |
| `runtime/usage.rs` | ~150 | `required_helpers` IR analysis + `is_native_direct_call`. |

## `src/target/shared/nir.rs` → `nir/` (1,629 lines)

| New file | Lines | Responsibility |
|----------|------:|----------------|
| `nir/mod.rs` / `types.rs` | ~300 | NIR type defs, `LINK_INIT_SYMBOL`/`link_thunk_symbol`. |
| `nir/lower.rs` | ~450 | IR→NIR lowering pass, `merge_packages`, link routing. |
| `nir/symbols.rs` | ~50 | Symbol-name generation. |
| `nir/json.rs` | ~450 | `ToNirJson` impls. |

## `src/target/shared/code/mod.rs` → `code/` submodules (14,899 lines)

This is the hub. The `builder_*` siblings already hold much of
`CodeBuilder`; this plan breaks up what's left in `mod.rs`. Biggest payoff,
do it carefully and incrementally.

| New file | Lines | Responsibility |
|----------|------:|----------------|
| `code/mod.rs` | — | Re-exports + module wiring; keep `lower_module_for_platform` orchestrator (~330) here or in `lower_module.rs`. |
| `code/error_constants.rs` | ~150 | Result tags, error code/message/symbol constants, register/collection magic constants. |
| `code/types.rs` | ~275 | `NativeCodePlan`, `CodeFunction/Instruction/Relocation/Import/...`, `CodegenPlatform` trait. |
| `code/builder_state.rs` | ~150 | `CodeBuilder` struct + lowering state structs (`LocalValue`, `TrapState`, `LoopLabels`, …). |
| `code/validation.rs` | ~375 | `validate`/`to_code_json` impls for plan/function/type-model. |
| `code/entry_and_arena.rs` | ~585 | Program-entry lowering, arena alloc/destroy, error diagnostics. |
| `code/function_lowering.rs` | ~280 | `lower_function`, builtin wrappers. |
| `code/runtime_helpers.rs` | ~3,056 | Runtime dispatcher + threading helpers. **Split further** (e.g. `runtime_helpers/thread.rs`). |
| `code/io_helpers.rs` | ~1,304 | IO helpers (read/write/poll/terminal). |
| `code/fs_helpers_paths.rs` | ~866 | Path/directory ops. |
| `code/fs_helpers_io.rs` | ~1,853 | FD-based file IO. **Consider splitting** open/close vs read/write. |
| `code/fs_helpers_atomic.rs` | ~2,092 | Atomic writes + path-level text/bytes ops. **Consider splitting.** |
| `code/codegen_utils.rs` | ~609 | String sort, UTF-8 validation, errno mapping, frame finalization. |
| `code/code_impl.rs` | ~228 | `impl`/`ToCodeJson` for `Code*` types. |
| `code/module_analysis.rs` | ~2,117 | Feature-detection walkers (`module_uses_*`, type-name/unicode collection). |
| `code/data_objects.rs` | ~411 | Unicode tables, string/type-name constant collection. |
| `code/type_utils.rs` | ~546 | Type analysis & numeric/format utilities. |
| `code/serialization_utils.rs` | ~489 | JSON/text serialization helpers. |
| `code/tests.rs` | ~48 | Arena unit tests. |

Note: three modules (`runtime_helpers`, `fs_helpers_io`, `fs_helpers_atomic`)
remain >1,500 lines after the first cut. Land the first split, confirm the
build, then sub-split those in a follow-up.

### `code/builder_misc.rs` → 4 files (3,008 lines)

| New file | Lines | Responsibility |
|----------|------:|----------------|
| `builder_emit_helpers.rs` | ~450 | Call emission/dispatch (`emit_symbol_call`, `emit_raw_call`, string loading, runtime-helper calls). |
| `builder_value_semantics.rs` | ~650 | Field access, record updates, string concat, constant/type resolution. |
| `builder_arena_transfer.rs` | ~820 | Arena-aware `copy_*`/`fix_*` value transfer + result materialization. |
| `builder_codegen_primitives.rs` | ~1,080 | Register/stack/label management, instruction emission, error-return protocol. |

### `code/net.rs` → 2 files (1,645 lines)

| New file | Lines | Responsibility |
|----------|------:|----------------|
| `net_helpers.rs` | ~350 | Shared emitters + TCP connect/listen establishment. |
| `net_io_poll.rs` | ~900 | Socket IO: accept/read/write/poll/timeout/DNS lookup. |

### `code/builder_collection_updates.rs` → 2 files (1,621 lines)

The file currently mixes mutation and query ops. Split along that line:

| New file | Lines | Responsibility |
|----------|------:|----------------|
| `builder_collection_mutate.rs` | ~850 | append/prepend/insert/remove/set/concat + COW. |
| `builder_collection_query.rs` | ~770 | `get`/`getOr` read ops with error handling. |

### Optional: `code/builder_strings_package.rs` (1,441 lines)

Extract the dominant `lower_strings_package_call` (~900 lines) into
`builder_strings_builtins.rs`, leaving helpers in a thinner
`builder_strings_package.rs`. Lower priority than the above.

## `src/arch/aarch64/encode.rs` → `encode/` (1,121 lines)

| New file | Lines | Responsibility |
|----------|------:|----------------|
| `encode/mod.rs` | ~150 | Public API (`EncodedImage`, …), `encode()` orchestration. |
| `encode/emitter.rs` | ~680 | `Encoder` + all `emit_*` instruction encoders + label patching. |
| `encode/sizing.rs` | ~180 | Instruction-size pre-pass, large-immediate chunking. |
| `encode/operand.rs` | ~100 | Field/register/immediate/shift parsing. |
| `encode/data.rs` | ~70 | Data-section layout, hex decoding. |

## `src/audit/collect.rs` → `collect/` (1,073 lines)

| New file | Lines | Responsibility |
|----------|------:|----------------|
| `collect/mod.rs` | ~35 | Public `collect()` orchestrator + `AuditInputs`. |
| `collect/project.rs` | ~67 | Project metadata + native resource collection. |
| `collect/dependencies.rs` | ~138 | Declared + installed package metadata extraction. |
| `collect/lockfile.rs` | ~79 | Lockfile parsing + project-hash computation. |
| `collect/source.rs` | ~487 | AST walks: fallible functions, traps, resource allocs, permissions + builtin tables. |
| `collect/findings.rs` | ~218 | Audit rule checks + sorting. |

(`manifest_string` helper, ~10 lines, can fold into `mod.rs`.)

## Lighter splits

### `src/resolver.rs` → `resolver/` (1,511 lines)

| New file | Lines | Responsibility |
|----------|------:|----------------|
| `resolver/mod.rs` | ~450 | `Resolver` struct + shared types, symbol registration, visibility. |
| `resolver/resolution.rs` | ~750 | Main resolve pipeline (decls/types/functions/statements/expressions). |
| `resolver/packages.rs` | ~280 | Package loading/validation, manifest parsing, dependency resolution. |

### `src/monomorph.rs` → `monomorph/` (1,674 lines)

Cohesive; light split only.

| New file | Lines | Responsibility |
|----------|------:|----------------|
| `monomorph/mod.rs` | ~200 | `monomorphize_project` entry + `Monomorphizer`/`ImportedOverload`/`FunctionContext`. |
| `monomorph/lower.rs` | ~1,140 | The `impl Monomorphizer` lowering methods. |
| `monomorph/helpers.rs` | ~330 | `unify_type`, `substitute_type_params`, `mangle_name`, overload collection. |

---

# Execution checklist (per file)

1. Create `src/foo/` and move `foo.rs` → `foo/mod.rs`.
2. Cut one logical group into a new `foo/<group>.rs`; add `mod <group>;` and
   any `use super::*;` / re-exports needed.
3. `cargo build` (and `cargo test` for the module) — expect zero behavior
   change. Fix visibility (`pub(crate)`, `pub(super)`) as the compiler flags.
4. Repeat for the next group; commit per file or per coherent step.
5. After the file is fully split, re-run the full test suite
   (`tests/repo_acceptance.rs`, golden audit tests, etc.) before moving on.
