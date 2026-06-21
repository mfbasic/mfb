# Large Source Files (>1000 lines)

| Lines | File |
|------:|------|
| 14,899 | `src/target/shared/code/mod.rs` |
| 6,568 | `src/typecheck.rs` |
| 5,807 | `src/ir.rs` |
| 5,103 | `src/ast.rs` |
| 3,928 | `src/binary_repr.rs` |
| 3,008 | `src/target/shared/code/builder_misc.rs` |
| 2,585 | `src/main.rs` |
| 2,126 | `src/target/shared/plan.rs` |
| 1,827 | `src/target/shared/runtime.rs` |
| 1,674 | `src/monomorph.rs` |
| 1,645 | `src/target/shared/code/net.rs` |
| 1,629 | `src/target/shared/nir.rs` |
| 1,621 | `src/target/shared/code/builder_collection_updates.rs` |
| 1,605 | `src/target/shared/validate.rs` |
| 1,511 | `src/resolver.rs` |
| 1,441 | `src/target/shared/code/builder_strings_package.rs` |
| 1,394 | `src/target/shared/code/builder_strings.rs` |
| 1,256 | `src/target/shared/code/builder_values.rs` |
| 1,174 | `src/target/shared/code/builder_numeric.rs` |
| 1,129 | `src/target/shared/code/builder_collection_queries.rs` |
| 1,121 | `src/arch/aarch64/encode.rs` |
| 1,112 | `src/target/shared/code/builder_collection_layout.rs` |
| 1,073 | `src/audit/collect.rs` |

---

## `src/target/shared/code/mod.rs` — 14,899 lines

The main AArch64 native code generation module. Houses the `CodeBuilder` struct and the top-level lowering pass that translates NIR operations into AArch64 machine instructions. Defines all error-string constants (overflow, underflow, float domain, allocation, index-out-of-range, etc.), the code plan types (`CodeInstruction`, `CodeRelocation`, `NativeCodePlan`), and the `CodeBuilder` implementation split across the `builder_*` submodules. This is the largest file because it is the hub that re-exports and ties together every facet of native code generation.

---

## `src/typecheck.rs` — 6,568 lines

The type checker and ownership checker. Walks the AST and infers or verifies types for every expression, statement, and function. Maintains a `Type` enum covering all MFBasic types (scalar, composite, thread, resource, function), tracks `OwnershipState` (available/moved) and borrow rules for `RES` parameters, validates thread transfer and accept signatures, and reports all type errors with source locations.

---

## `src/ir.rs` — 5,807 lines

Defines the Intermediate Representation (IR) used between the AST front-end and the native back-end. Contains `IrProject`, `IrFunction`, `IrOp`, `IrType`, `IrValue`, and related structs that model every language construct at a lower level. Also carries `IrLinkFunction` and `IrNativeResource` for native `LINK` declarations, and includes the IR-to-binary-repr encoding logic (`encode_binary_repr`).

---

## `src/ast.rs` — 5,103 lines

Defines the Abstract Syntax Tree and contains the parser. Declares all node types — `AstProject`, `AstFile`, `Item`, `TopLevelBinding`, `Function`, `TypeDecl`, `Expression`, `Statement`, `MatchPattern`, and so on — and implements the recursive-descent parser that converts the token stream into this tree. Also handles project-level manifest parsing and import resolution setup.

---

## `src/binary_repr.rs` — 3,928 lines

Implements the MFPC binary package format (version 2). Writes and reads the structured binary container with named sections: manifest, string pool, type table, constant pool, import/export tables, global table, function table, resource table, ABI index, and the binary-repr function-body payload. Computes SHA-256 ABI hashes for interface stability checking and produces the `.mfp` and `.hex` output artifacts.

---

## `src/target/shared/code/builder_misc.rs` — 3,008 lines

Miscellaneous helpers shared across all `CodeBuilder` impl blocks. Contains `emit_symbol_call`, `emit_raw_call`, call-argument preparation (spill-to-stack then reload into argument registers), string/constant loading, arena allocation wrappers, result-register conventions, stack-object allocation, label generation, and other plumbing used by every other builder submodule.

---

## `src/main.rs` — 2,585 lines

The compiler entry point and CLI driver. Parses the command line (`help`, `init`, `init-pkg`, `build`, `audit`, `man`, `pkg add/info/verify`) and drives the full compilation pipeline: manifest loading, lexing, parsing, monomorphization, resolution, type checking, IR lowering, NIR lowering, validation, native plan generation, code generation, encoding, and final object/binary linking. Also handles project scaffolding and package management commands.

---

## `src/target/shared/plan.rs` — 2,126 lines

Generates the `NativePlan` — the high-level native compilation plan. Walks the NIR module to enumerate all functions, runtime helper dependencies, platform-import requirements (libc symbols per helper call), external symbols, and native `LINK` symbols. The plan drives what the object-plan and linker scripts need to include, and captures `libc_symbols` tables for each runtime helper.

---

## `src/target/shared/runtime.rs` — 1,827 lines

The runtime helper catalog. Defines the `RuntimeHelper` enum (Fs, General, Io, Math, Net, Strings, Thread) and `RuntimeHelperSpec` structs that encode each helper function's ABI: parameter types, return type, and clobbered registers. Used by `CodeBuilder` to emit correct call sequences into the MFB runtime library and by the plan generator to determine platform symbol requirements.

---

## `src/monomorph.rs` — 1,674 lines

The monomorphizer. Expands generic type and function templates into concrete instantiations before IR lowering. Resolves overloaded functions (including those imported from packages), rewrites call sites to use the mangled monomorphic name, and normalizes package-qualified type names so the binary merge can identity-prefix them correctly.

---

## `src/target/shared/code/net.rs` — 1,645 lines

Native code generation for the built-in `net` package runtime helpers. Emits self-contained AArch64 functions for DNS lookup (`getaddrinfo`/`inet_ntop`), TCP connect, TCP listen, accept, poll, read/readText, write/writeText, close, and address query. Each helper marshals libc socket calls and returns the standard `(tag, value)` result convention used throughout the native backend.

---

## `src/target/shared/nir.rs` — 1,629 lines

Defines the Native IR (NIR), the intermediate tier between the high-level IR and machine code generation. Contains `NirModule`, `NirFunction`, `NirOp`, `NirValue`, `NirType`, and related types. Also defines the IR-to-NIR lowering pass that flattens complex IR operations into the flat, register-oriented NIR form the code builder consumes. Declares the `LINK_INIT_SYMBOL` and `link_thunk_symbol` conventions for native binding codegen.

---

## `src/target/shared/code/builder_collection_updates.rs` — 1,621 lines

Codegen for collection mutation operations. Implements `append`, `prepend`, `insert`, `remove`, and `replace` for both lists and maps, including COW (copy-on-write) semantics, capacity management, and the specialized list-slot helper that packs inline record/union payloads alongside scalar and pointer elements.

---

## `src/target/shared/validate.rs` — 1,605 lines

The NIR validator. Verifies structural correctness of a `NirModule` before code generation begins: unique function/global/import names, valid entry-point signatures, resource-ownership rules (no duplicate close, no use after close), consistent runtime helper declarations, and correct use of builtin operations. Catches contract violations early so the code builder can assume a well-formed input.

---

## `src/resolver.rs` — 1,511 lines

The name resolver. Walks the AST after parsing to verify that every identifier refers to a known binding, type, or import; that constructor arguments match their type declarations; that pattern exhaustiveness is consistent; and that package-qualified names resolve through the import table. Reports all resolution errors with source locations before the type checker runs.

---

## `src/target/shared/code/builder_strings_package.rs` — 1,441 lines

Codegen for the `strings.*` built-in package. Implements `trim`, `trimStart`, `trimEnd`, `upper`, `lower`, `caseFold`, `normalizeNfc`, `byteLen`, `startsWith`, `endsWith`, `contains`, `graphemes`, `split`, `join`, `format`, and related functions by emitting AArch64 instructions and calls to the strings runtime helper or inline Unicode tables.

---

## `src/target/shared/code/builder_strings.rs` — 1,394 lines

Lower-level string value operations shared by several builder modules. Implements `replace` for both string values and lists-of-strings, string comparison helpers, and the core byte-sequence manipulation emit patterns used by the strings package builder.

---

## `src/target/shared/code/builder_values.rs` — 1,256 lines

The central NIR-value lowering dispatcher. Implements `lower_value`, which routes each `NirValue` variant (constant, local, call, constructor, field access, match, etc.) to the appropriate emit logic and maintains the `current_loc` source-location cursor so that runtime errors stamp a real `ErrorLoc`.

---

## `src/target/shared/code/builder_numeric.rs` — 1,174 lines

Numeric and Boolean operation codegen. Implements short-circuit `AND`/`OR`, `XOR`, and all integer, fixed-point, and floating-point arithmetic operators including overflow/underflow detection, divide-by-zero, float domain errors, and calls to external math symbols for transcendental functions.

---

## `src/target/shared/code/builder_collection_queries.rs` — 1,129 lines

Codegen for collection read operations. Implements `get`, `contains`, `count`/`len`, `keys`, `values`, `first`, `last`, `slice`, and iteration-related helpers for both lists and maps, dispatching to the correct slot-width path based on element type.

---

## `src/arch/aarch64/encode.rs` — 1,121 lines

The AArch64 binary instruction encoder. Converts the high-level `CodeOp` instruction stream (from `NativeCodePlan`) into raw 32-bit machine words, resolves forward-reference labels by back-patching branch offsets, lays out the data section, and produces the `EncodedImage` (text bytes, data bytes, symbol table, relocations, imports) consumed by the object-file writer.

---

## `src/target/shared/code/builder_collection_layout.rs` — 1,112 lines

Collection memory layout helpers. Computes inline payload sizes and alignment requirements for each element type (scalars, pointers, inline records/unions), emits the `emit_align_offset_slot` helper for padding to alignment boundaries, and manages the physical data-region layout for packed list and map storage.

---

## `src/audit/collect.rs` — 1,073 lines

Assembles the `AuditReport` from project metadata and source. Collects declared dependencies, installed packages, source-level I/O flow, permission usage, native `LINK` resource declarations, and lockfile status entirely offline (no build required). Runs finding checks (missing lockfile, outdated packages, undeclared permissions, resource lifecycle issues) and sorts results for consistent output.
