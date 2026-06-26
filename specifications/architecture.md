# MFBASIC Compiler Architecture

Last updated: 2026-06-19 HST

This document describes how the current compiler implementation moves an
MFBASIC project from source files to either a native executable or a compiled
`.mfp` package.

It is an implementation architecture document, not a language reference. The
language syntax and package/container formats are specified separately in:

- `specifications/mfbasic.md`
- `specifications/project.md`
- `specifications/package_format.md`
- `specifications/standard_package.md`

## 1. High-Level Shape

The compiler is a single Rust binary named `mfb`. The command-line entry point
is `src/main.rs`. It owns project-level orchestration, manifest validation,
package-management commands, build-mode selection, and high-level error
handling.

The build pipeline has a shared source front end:

```text
project.json
  -> source discovery
  -> lexing
  -> parsing
  -> AST
  -> name resolution
  -> monomorphization
  -> name resolution again
  -> entry-point validation
  -> type checking
  -> IR
```

After IR, the pipeline splits:

```text
Executable build:
  IR
    -> native IR
    -> native plan
    -> native code plan
    -> encoded aarch64 image
    -> OS executable container/link step
    -> <project>.out

Package build:
  IR
    -> MFPC architecture-independent binary representation
    -> unsigned MFP container
    -> <package>.mfp
```

Diagnostic and validation output is emitted during the front-end passes. Build
artifacts are written into the project directory.

## 2. Commands And Build Modes

The CLI supports these build-related commands:

- `mfb init <location>` creates an executable project with `project.json` and
  `src/main.mfb`.
- `mfb init-pkg <location>` creates a package project with `project.json` and
  `src/lib.mfb`.
- `mfb build [location]` validates and emits the primary artifact for the
  project kind.
- `mfb build -ast [location]` writes `<name>.ast`.
- `mfb build -ir [location]` writes `<name>.ir`.
- `mfb build -br [location]` writes `<name>.hex`, a hexadecimal dump of MFPC
  binary representation.
- `mfb build -nir [location]` writes `<name>.nir`.
- `mfb build -nplan [location]` writes `<name>.nplan`.
- `mfb build -nobj [location]` writes `<name>.nobj`.
- `mfb build -ncode [location]` writes `<name>.ncode`.
- `mfb build -target os-arch [location]` selects a native target instead of
  the host target.
- `mfb build -app [location]` selects GUI app mode: the executable and native
  intermediate outputs target a windowing app runtime instead of the console
  runtime — AppKit on macOS, GTK4 on Linux. Shared lowering treats both uniformly
  (`NativeBuildMode::is_app`); the target OS selects the toolkit. `-app` is valid
  only for executable projects and only when `-target` resolves to a native target
  that supports app mode (`macos-aarch64` or `linux-aarch64`); it is rejected
  otherwise. App mode is recorded as the `buildMode` field in `-nir`, `-nplan`,
  and `-ncode` output (`"console"`, `"macos-app"`, or `"linux-app"`).

The output flags are mutually exclusive. If no output flag is supplied,
`mfb build` emits:

- `<name>.out` for `kind = "executable"`.
- `<name>.mfp` for `kind = "package"`.

Native intermediate outputs are rejected for package projects. Package projects
are emitted through the package binary representation path instead.

- `mfb fmt [--check] [--indent N] [location]` formats every `.mfb` file selected
  by the project manifest (or a single `.mfb` file) in place, normalizing block
  indentation and keyword capitalization. The formatter is purely lexical
  (`src/fmt.rs`): it re-tokenizes raw text to preserve comments, blank lines,
  string contents, and `DOC`/`LINK` block bodies verbatim. `--indent` sets the
  indent width (default `2`); `--check` writes nothing and exits non-zero with an
  `FMT_CHECK_FAILED` diagnostic when any file is not already formatted.

## 3. Project Manifest Loading

The project manifest is `project.json` in the build location. The manifest is
read and validated by `validate_project_manifest` in `src/main.rs`.

The current implementation requires these string fields:

- `name`
- `version`
- `mfb`

It also requires `sources` to be a non-empty array of objects, each with a
string `root` field. Optional `entry`, `author`, and `url` fields must be
strings when present. Optional `kind` must be a string and is expected to be
`"executable"` or `"package"`. Unknown kinds are diagnosed, but the current
validator continues after that diagnostic.

The current implementation does not enforce every field described in
`specifications/project.md`. In particular, it primarily consumes:

- `name`
- `version`
- `mfb`
- `kind`
- `sources[*].root`
- `entry`
- `author`
- `url`
- `packages`

Fields such as `include`, `exclude`, `role`, `targets`, and richer repository
metadata are documented for the project format but are not the active source of
build behavior in the compiler code reviewed here.

## 4. Source Discovery And Parsing

Source parsing is implemented in `src/ast.rs`.

`ast::parse_project` receives the validated project name, project directory,
and manifest. It reads `sources[*].root`, joins each root to the project
directory, recursively collects `.mfb` files, sorts them, and parses each file.

For each source file:

1. The file is read as text.
2. `lexer::lex` tokenizes the source.
3. `FileParser` parses tokens into an `AstFile`.
4. The file path is stored relative to the project directory.

The AST keeps imports, type declarations, function declarations, statements,
expressions, visibility, parameters, default values, and source line positions.
`mfb build -ast` serializes this structure to `<project>.ast`.

Current discovery behavior:

- If a source root is a file, it is included only when its extension is `.mfb`.
- If a source root is a directory, all nested `.mfb` files are included.
- Empty source roots are compile-time errors.
- `include` and `exclude` manifest patterns are not currently applied by the
  source collector.

## 5. Name Resolution

Name resolution is implemented in `src/resolver.rs`.

The resolver has two jobs:

1. Collect top-level symbols from the project.
2. Validate references inside imports, type declarations, function bodies, and
   expressions.

The resolver knows built-in type names such as `Boolean`, `Byte`, `Error`,
`Fixed`, `Float`, `Integer`, `Json`, `Nothing`, `Result`, `String`,
`TerminalSize`, and `FileHandle`.

Before resolving, the resolver calls `builtins::json::augmented_project` to
expand `Json`-typed declarations into the augmented AST used for the rest of
resolution.

It also reads declared package dependencies from the manifest and uses those to
validate imported package roots. For source imports, it detects duplicate
imports in a file, duplicate top-level names, duplicate function overloads with
the same parameter type shape, unknown types, unknown functions, invalid
constructors, invalid member references, and related symbol errors.

Resolution runs twice:

```text
parsed AST
  -> resolver::resolve_project
  -> monomorph::monomorphize_project
  -> resolver::resolve_project again
```

The second resolution pass is important because monomorphization rewrites
generic/template code into concrete declarations that must also obey normal
symbol rules.

## 6. Monomorphization

Monomorphization is implemented in `src/monomorph.rs`.

This pass takes the parsed, initially resolved AST and produces a concrete AST.
Template/generic declarations are expanded into concrete forms based on use
sites. The rest of the pipeline consumes the concrete AST, not the original AST.

Because the concrete AST introduces generated declarations and names, the build
pipeline immediately runs the resolver again after monomorphization.

## 7. Entry-Point Validation

Entry-point validation is implemented in `validate_entry_point` in
`src/main.rs`.

Package projects have no executable entry point and return `None` for the IR
entry.

Executable projects use the manifest `entry` field, defaulting to `main`.
The selected function must be a top-level `SUB` or `FUNC` with one of these
effective signatures:

```basic
SUB main
END SUB

SUB main(args AS List OF String)
END SUB

FUNC main AS Integer
END FUNC

FUNC main(args AS List OF String) AS Integer
END FUNC
```

Rules enforced by the implementation:

- A `FUNC` executable entry must return `Integer`.
- The entry may have zero parameters or one `List OF String` parameter.
- The args parameter must not declare a default value.
- Missing or invalid executable entries are compile-time errors.

The resulting IR entry records the entry name, return type, and whether the
program accepts command-line arguments.

## 8. Type Checking

Type checking is implemented in `src/typecheck.rs`.

The type checker builds indices for:

- Local project functions.
- Exported package functions.
- User-defined types.
- Type kinds.
- Type fields.
- Union member types.
- Enum members.

It then validates declarations, statement flow, expression types, mutability,
constructor usage, member access, function calls, built-in calls, package calls,
return/fail behavior, isolated-function restrictions, and default values.

The type model includes primitive and compound forms:

- `Boolean`
- `Byte`
- `Error`
- `Fixed`
- `Float`
- `Integer`
- `List<T>`
- `Map<K, V>`
- function values (with parameter types, return type, and isolated flag)
- `Nothing`
- `Result<T>`
- `String`
- `Thread<T, E>`
- `ThreadWorker<T, E>`
- user-defined types

Type checking is the last front-end validation pass before lowering to IR.

## 9. Package Dependencies

Package dependency handling is split between `src/main.rs`, `src/binary_repr.rs`,
and `src/target/shared/nir.rs`.

### 9.1 Installing Packages

`mfb pkg add <url>` currently supports `file://` URLs that point to absolute
`.mfp` files. The command:

1. Reads and validates the MFP header.
2. Copies the package to `packages/<name>.mfp`.
3. Adds a dependency entry to `project.json`.
4. Pins the dependency to the installed package version.

The package entry written to `project.json` includes:

- `name`
- `version`, as an exact `=<version>` requirement
- `pin`, as the concrete package version
- `source`, as the original URL

### 9.2 Verifying Packages

`mfb pkg verify` reads the manifest `packages` array and checks that each
declared package has a matching installed file under `packages/<name>.mfp`.
Pinned dependencies must match the installed package header version.

### 9.3 Using Packages During Compilation

Executable builds load installed package files before IR lowering. The compiler
reads each package header and exported binary representation ABI metadata, then creates
external function signatures under qualified names such as:

```text
packageName.exportName
```

These signatures are passed into `ir::lower_project_with_external_functions`
so calls to package functions survive lowering with proper function types.

For native executable builds, package exports also become NIR imports with
generated symbols:

```text
_mfb_pkg_<package>_<export>
```

For binary representation merging, package binary representation is decoded and appended to the
application binary representation function/type/constant/import/export structures.

## 10. IR Lowering

IR lowering is implemented in `src/ir.rs`.

The IR is a typed, architecture-independent representation of the concrete AST.
It contains:

- Project name.
- Optional executable entry point.
- Top-level bindings (program globals).
- User-defined types.
- Functions.
- Parameters and defaults.
- Structured operations.
- Structured expression values.

The main IR operation forms are:

- `Bind`
- `Assign`
- `AssignGlobal` — assignment to a top-level binding
- `Return`
- `ExitLoop` — structured `EXIT FOR/DO/WHILE` (carries the `LoopKind`)
- `ContinueLoop` — structured `CONTINUE FOR/DO/WHILE` (carries the `LoopKind`)
- `ExitProgram` — `EXIT PROGRAM` with a status code and full RAII unwind
- `Fail`
- `Eval`
- `If`
- `Match`
- `While` — `WHILE`/`DO WHILE` loops (carries the `LoopKind`)
- `For` — counted `FOR` loops with start/end/step
- `DoUntil` — bottom-tested `DO ... UNTIL` loops
- `ForEach`
- `Trap` — inline `TRAP` block; `RECOVER` is lowered into the trap body via
  recover targets rather than a dedicated op

The main IR value forms are:

- `Const` — typed literal constants
- `Local` — local variable references
- `Global` — top-level binding references
- `FunctionRef` — references to named functions
- `Closure` — closure values with a name, type, and captured variable list
- `Capture` — reference to a captured variable by index inside a closure body
- `Call` — calls to functions that return a plain value
- `CallResult` — calls to functions that return `Result<T>`
- `Constructor` — record/struct constructors
- `UnionWrap` — wrapping a value as a specific union member
- `UnionExtract` — extracting the inner value from a union member
- `ResultIsOk` — testing whether a `Result` value is the success case
- `ResultValue` — extracting the success value from a `Result`
- `ResultError` — extracting the error value from a `Result`
- `WithUpdate` — record update expressions
- `ListLiteral` — list literal values
- `MapLiteral` — map literal values
- `MemberAccess` — member field access
- `Binary` — binary operator expressions
- `Unary` — unary operator expressions

`mfb build -ir` serializes this representation to `<project>.ir`.

IR is intentionally shared by both downstream products:

- Native executable generation lowers IR to target-specific native structures.
- Package generation lowers IR to architecture-independent MFPC binary representation.

## 11. Binary Representation And Package Generation

Binary Representation generation is implemented in `src/binary_repr.rs`.
MFP package wrapping is implemented in `src/target/package_mfp/mod.rs`.

### 11.1 What the Binary Representation Is

**The Binary Representation is the compiler's IR, exposed as a versioned external
interface.** The in-memory IR of §10 (`IrProject` / `IrFunction` / `IrOp` /
`IrValue` / `IrType`) is the compiler's private, in-process model and is free to
change between builds. The Binary Representation is a defined, versioned binary
*serialization* of that model: control flow stays nested, expressions stay as
trees, and the structure is preserved faithfully — there is no lowering to a flat
opcode/register machine. `src/binary_repr.rs` encodes IR → Binary Representation
and decodes Binary Representation → IR.

The two are related but **not the same thing**, and the distinction is the whole
point of the boundary:

- The **IR** is an unstable in-memory data structure. Nothing outside the
  compiler process may depend on its layout.
- The **Binary Representation** is the stable on-disk contract. It carries its own
  format version (`MFBR` payload magic, `MFPC` container major `2`), so a future
  compiler can change the IR freely as long as it can still encode/decode this
  versioned format. Because the encoding is a faithful, structure-preserving
  serialization, a consumer **decodes it straight back into IR** and lowers it
  through the single `IR → NIR → native` codegen used for the executable's own
  code — no second, package-only code path.

The binary representation layer lowers IR into an architecture-independent package
image that starts with `MFPC` magic and contains sectioned data. The implemented
sections include:

- manifest
- string pool
- type table
- constant pool
- import table
- export table
- global table
- function table
- binary representation (the structured function bodies, `MFBR` payload)
- resource table
- ABI index

The binary representation writer builds:

- A string pool for names, literals, package metadata, and version data.
- A type table with primitive and user-defined types.
- A constant pool for literal values.
- Import and dependency metadata.
- Export metadata for non-private functions.
- Function tables with parameters and cleanup metadata; each function body is the
  structured Binary Representation node tree in the `MFBR` payload section.
- ABI hashes used by package readers and dependency checks.

`mfb build -br` writes a hexadecimal dump of the binary representation to `<project>.hex`.
When the executable project has package dependencies, the binary representation path
decodes installed packages back into IR and merges them, so every function flows
through the one codegen.

### 11.2 MFP Package Container

Package projects emit a `.mfp` file through `target::write_package`.

The package path is:

```text
IR
  -> binary_repr::build_binary_repr_bytes
  -> package_mfp::build_package_bytes
  -> <package>.mfp
```

Package metadata is derived from `project.json`:

- `name`
- `version`
- `author`
- `url`
- dependency constraints from `packages`

The current package writer emits an unsigned MFP container:

- container major/minor: `1.0`
- binary representation major/minor: `1.0`
- signature type: unsigned
- signature length: zero
- pre-release flag set when the version contains `-`

*NOTE: `package_format.md` specifies a `signatureType` field that allows
signed containers. The current writer always emits `signatureType = 0` and
`signatureLength = 0`. The reader's `validate_signature_header` already accepts
an ed25519 signature header (`signatureType = 1`, `signatureLength = 64`) in
addition to the unsigned form, but no signed containers are produced yet.*

The package payload must start with `MFPC`. Metadata string lengths are checked
before writing.

### 11.3 Error Source Locations

Every user-visible `Error` carries an `ErrorLoc source` recording where it
originated. The location flows through every layer:

- **AST** (`src/ast.rs`): `Expression::Call`/`Binary`/`Unary` and `Statement::For`
  carry an internal `(line, column)`; the source file is the enclosing `AstFile`.
  These are not serialized to the `.ast` JSON.
- **IR** (`src/ir.rs`): `IrValue::Call`/`CallResult`/`Binary`/`Unary` and
  `IrOp::For` carry an `IrSourceLoc { line, column }`; each `IrFunction` carries
  its source `file`. The `error(code, message)` built-in lowers to nested record
  constructors — `Error[code, message, ErrorLoc[file, line, char]]` — so
  `Error`/`ErrorLoc` are ordinary records for the rest of the pipeline. These
  fields are not serialized to the `.ir` JSON but are encoded into the Binary
  Representation, so an imported package's functions retain their own source
  locations.
- **NIR** (`src/target/shared/nir.rs`): mirrors the IR fields (`NirSourceLoc`,
  `NirFunction::file`).
- **Native runtime** (`src/target/shared/code`): the code generator tracks the
  current function file and the current node location, builds a real `ErrorLoc`
  at every error origin (user `error(...)`, arithmetic overflow/divide-by-zero,
  failing built-in/helper calls), and threads the origin through the four-register
  result ABI (see `memory_layouts.md`). Propagation preserves the origin; trapping
  materializes the 3-field `Error`.

## 12. Native Executable Generation

Native executable generation is implemented under `src/target`,
`src/target/shared`, `src/arch`, and `src/os`.

The active native backend registry is in `src/target.rs`:

- `macos-aarch64`
- `linux-aarch64`

Each backend implements the `NativeBackend` trait. The trait exposes
capabilities and methods for executable and intermediate artifact emission.

The native executable pipeline is:

```text
IR
  -> target/shared/lower.rs
  -> target/shared/nir.rs
  -> target/shared/validate.rs
  -> target/<os>_aarch64/plan.rs
  -> target/shared/plan.rs
  -> os/<os>/object.rs
  -> target/<os>_aarch64/code.rs
  -> target/shared/code.rs
  -> arch/aarch64/encode.rs
  -> os/<os>/link.rs
  -> <project>.out
```

### 12.1 Native IR

Native IR, or NIR, is defined in `src/target/shared/nir.rs`.

NIR is close to the shared IR but adds native build concerns:

- Target name.
- Imported package functions with platform symbols.
- Runtime helper declarations.
- Native call forms for built-ins that require runtime support.

The NIR lowerer reads installed package exports and produces NIR imports. It
also rewrites supported built-in calls into runtime-call forms where needed.

`mfb build -nir` writes `<project>.nir`.

### 12.2 Runtime Helper Selection

Runtime-helper detection is implemented in `src/target/shared/runtime.rs`.

The compiler scans IR values for calls into built-in packages. It records
which helper families are needed:

- `fs`
- `general`
- `io`
- `math`
- `strings`
- `thread`

`validate_capabilities` rejects native builds that require runtime calls not
listed in the backend capability set. Both `macos-aarch64` and
`linux-aarch64` currently declare the same set of supported native runtime
calls:

- All `io.*` calls: `io.print`, `io.write`, `io.flush`, `io.printError`,
  `io.writeError`, `io.flushError`, `io.input`, `io.readLine`, `io.readChar`,
  `io.readByte`, `io.pollInput`, `io.isInputTerminal`, `io.isOutputTerminal`,
  `io.isErrorTerminal`, `io.terminalSize`
- Most `fs.*` calls: `fs.open`, `fs.openFile`, `fs.openFileNoFollow`,
  `fs.createTempFile`, `fs.close`, `fs.readLine`, `fs.readAll`,
  `fs.readAllBytes`, `fs.writeAll`, `fs.writeAllBytes`, `fs.readText`,
  `fs.readBytes`, `fs.writeText`, `fs.writeTextAtomic`, `fs.writeBytes`,
  `fs.writeBytesAtomic`, `fs.appendText`, `fs.appendBytes`, `fs.eof`,
  `fs.fileExists`, `fs.directoryExists`, `fs.exists`, `fs.canonicalPath`,
  `fs.isWithin`, `fs.deleteFile`, `fs.createDirectory`,
  `fs.createDirectories`, `fs.deleteDirectory`, `fs.listDirectory`,
  `fs.currentDirectory`, `fs.tempDirectory`, `fs.setCurrentDirectory`
- All `thread.*` calls: `thread.start`, `thread.isRunning`, `thread.waitFor`,
  `thread.cancel`, `thread.send`, `thread.poll`, `thread.receive`,
  `thread.isCancelled`

`math` and `strings` operations are not listed as runtime helper calls because
they are code-generated inline rather than dispatched through external runtime
helpers.

### 12.3 Native Validation

Native validation is implemented in `src/target/shared/validate.rs`.

It validates:

- Non-empty target fields.
- NIR project and function shape.
- Unique function and import names.
- Entry resolution.
- Runtime helper consistency.
- Backend runtime-call capability support.
- Native-plan and native-code-plan structural invariants.

Important current limitation: `validate_project` in this module is currently a
no-op. Validation is therefore distributed across the front-end passes, NIR
validation, plan validation, code-plan validation, and OS/linker checks rather
than centralized in target project validation.

### 12.4 Native Plan

Native planning is implemented by platform-specific wrappers in:

- `src/target/macos_aarch64/plan.rs`
- `src/target/linux_aarch64/plan.rs`

Both use the shared planner in `src/target/shared/plan.rs`.

The native plan records:

- Target and project.
- Entry symbol.
- Required runtime symbols.
- External package symbols.
- Platform imports.
- Planned functions.
- Parameter storage.
- Stack slots.
- Labels.
- Planned operation descriptions.
- Calls and call kinds.

`mfb build -nplan` writes `<project>.nplan`.

### 12.5 Native Object Plan

OS object/container planning is implemented in:

- `src/os/macos/object.rs`
- `src/os/linux/object.rs`

The object plan is still a JSON planning artifact, not the final executable
container. It describes how the already planned native code will be arranged in
Mach-O or ELF terms:

- image base
- load commands or program headers
- segments
- sections
- code units
- data units
- defined symbols
- imported symbols
- symbol/string tables
- relocations

macOS object plans target a Mach-O layout with `__TEXT`, `__cstring`, and
`__LINKEDIT` regions. Linux object plans target an ELF layout with a loadable
text/rodata image.

`mfb build -nobj` writes `<project>.nobj`.

### 12.6 Native Code Plan

Native code planning is implemented by platform-specific wrappers in:

- `src/target/macos_aarch64/code.rs`
- `src/target/linux_aarch64/code.rs`

Both use the shared code generator in `src/target/shared/code.rs`.

The native code plan records:

- Target and architecture.
- Project.
- Entry symbol.
- Imports.
- Data objects.
- Functions.
- Stack frames.
- Parameters and locations.
- AArch64 instruction operations.
- Relocations.
- Stack slots.

The code generator also adds:

- A program entry wrapper when an executable entry exists.
- Arena allocation and destruction helpers.
- Required runtime helper implementations.
- String data objects.
- Error string data used by entry/error paths.

`mfb build -ncode` writes `<project>.ncode`.

### 12.7 AArch64 Encoding

Architecture-specific instruction encoding is under `src/arch/aarch64`.

The encoder consumes the native code plan and produces an `EncodedImage` with:

- text bytes
- data bytes
- symbols
- relocations
- imports
- entry symbol

The encoder handles AArch64 instruction forms and ABI details used by the
native code plan.

### 12.8 Linking And Executable Writing

The final OS-specific executable writers are:

- `src/os/macos/link.rs`
- `src/os/linux/link.rs`

Both writers:

1. Patch relocations in encoded text.
2. Resolve the entry symbol to a text offset.
3. Encode the OS executable container.
4. Write `<project>.out`.
5. Set executable permissions to `0755`.

macOS output:

- Encodes a Mach-O executable.
- Supports imports from `libSystem`.
- Emits import stubs when platform imports are present.
- Adds an ad hoc code signature.
- Writes a single `<project>.out`.

Linux output:

- Emits two output files, one per flavor: `<project>-glibc.out` and
  `<project>-musl.out`.
- When external imports are present, encodes a dynamic ELF executable with
  import stubs and a PLT/GOT; when there are no imports, encodes a static ELF.
- The glibc flavor links against `libc.so.6`, `libm.so.6`, and
  `libpthread.so.0`. The musl flavor links against `libc.musl-aarch64.so.1`
  (which bundles pthread).
- `LinuxFlavor` (`src/os/linux/flavor.rs`) selects interpreter path and
  `DT_NEEDED` entries per flavor.

### 12.9 Runtime Value Memory Model

Native code generation realizes the language's value semantics over a per-arena
heap. `specifications/memory_layouts.md` is the authority; the architectural shape:

- **Flat values.** Every non-resource value (`String`, `Record`, `Union`, `List`,
  `Map`, `Error`, `Result`) is a single self-describing, pointer-free arena block —
  all composite sub-values are inlined by block-relative offset, not pointers. A
  resource is the one exception: an opaque move-only handle to its single instance.
- **Copy = `memcpy`.** Because a flat block has no internal pointers, copying any
  value is one `arena_alloc` + one `memcpy` (`copy_flat_block`); there is no
  per-type deep-copy glue. `thread::transfer`/`send` use the same routine to copy
  into the receiver's arena.
- **Ownership tree via copy-insertion.** Values are shared by pointer at most read
  sites, so the lowering inserts a deep copy (`lower_value_owned`) at every site
  that hands a value to a longer-lived owner — `Bind`/`Assign`, global store,
  closure capture, and `Return` — whenever the source is an alias/borrow (a local,
  global, capture, field/`MemberAccess` read, union/`Result` extract) or a static
  `String` constant (rodata, not arena). After this every owned local owns an
  independent block. Constructors, collection inserts, and `WITH` already inline
  (copy) their flat payloads.
- **Deterministic scope-drop frees.** Each owned, non-escaping flat local is freed
  by one `arena_free(ptr, size)` at scope exit (normal drain, `EXIT`/`CONTINUE`,
  `RETURN`, `TRAP`), reusing the resource-cleanup machinery. A returned local is
  moved out (its free suppressed); resources, runtime-managed thread results
  (`thread::receive`/`waitFor`), and recursive/non-flat composites are excluded.
- **Arena reuse + entropy fill.** Freed blocks go onto a per-arena coalescing
  free-list for reuse (never returned to the OS until bulk `arena_destroy` at
  teardown), and freed/freshly-mapped memory is filled with PRNG bytes, always on,
  so a use-after-free or uninitialized read fails loudly instead of silently.

## 13. Source-To-Executable End-To-End Flow

For an executable project, `mfb build` performs this sequence:

1. Parse command-line options and select target.
2. Read and validate `project.json`.
3. Determine project kind, defaulting to `executable`.
4. Parse all `.mfb` source files from manifest roots.
5. Resolve the parsed AST.
6. Monomorphize the AST.
7. Resolve the concrete AST.
8. Validate the executable entry point.
9. Type-check the concrete AST.
10. Read installed package files from `packages/<name>.mfp`.
11. Read package export signatures.
12. Lower the concrete AST to IR with external package function types.
13. Select the native backend for the requested target.
14. Validate backend support.
15. Lower IR to NIR.
16. Validate NIR and backend runtime capabilities.
17. Lower NIR to a native plan.
18. Validate the native plan.
19. Lower the native plan to an OS object plan for validation or `-nobj`.
20. Lower NIR and the native plan to a native code plan.
21. Validate the native code plan.
22. Encode AArch64 text, data, symbols, relocations, and imports.
23. Link/write the OS executable container.
24. Mark the output executable.
25. Print the output path(s).

The default output file is:

```text
<project>/<project-name>.out          (macOS)
<project>/<project-name>-glibc.out    (Linux)
<project>/<project-name>-musl.out     (Linux)
```

Linux builds always emit both flavor outputs in a single `mfb build` run.

## 14. Source-To-Package End-To-End Flow

For a package project, `mfb build` performs this sequence:

1. Parse command-line options.
2. Read and validate `project.json`.
3. Determine project kind as `package`.
4. Parse all `.mfb` source files from manifest roots.
5. Resolve the parsed AST.
6. Monomorphize the AST.
7. Resolve the concrete AST.
8. Skip executable entry-point selection.
9. Type-check the concrete AST.
10. Lower the concrete AST to IR.
11. Build binary representation metadata from the manifest.
12. Lower IR to MFPC package binary representation.
13. Validate package metadata and MFPC payload magic.
14. Wrap binary representation in an unsigned MFP container.
15. Write the package file.
16. Print the output path.

The default output file is:

```text
<project>/<package-name>.mfp
```

Package projects do not support native intermediate outputs. Use plain
`mfb build` for `.mfp` emission or `-ast`, `-ir`, and `-br` for front-end and
binary representation inspection.

## 15. Artifact Summary

| Artifact | Command | Producer | Meaning |
| --- | --- | --- | --- |
| `<name>.ast` | `mfb build -ast` | `src/ast.rs` | Parsed source tree before monomorphization. |
| `<name>.ir` | `mfb build -ir` | `src/ir.rs` | Typed, architecture-independent compiler IR. |
| `<name>.hex` | `mfb build -br` | `src/binary_repr.rs` | Hex dump of MFPC binary representation. |
| `<name>.nir` | `mfb build -nir` | `src/target/shared/nir.rs` | Native IR for the selected target. |
| `<name>.nplan` | `mfb build -nplan` | `src/target/shared/plan.rs` | Native function/storage/call plan. |
| `<name>.nobj` | `mfb build -nobj` | `src/os/*/object.rs` | OS object/container layout plan. |
| `<name>.ncode` | `mfb build -ncode` | `src/target/shared/code.rs` | AArch64 code-generation plan. |
| `<name>.out` | `mfb build` executable (macOS) | `src/os/macos/link.rs` | Native executable (Mach-O). |
| `<name>-glibc.out` | `mfb build` executable (Linux) | `src/os/linux/link.rs` | Native executable (ELF, glibc). |
| `<name>-musl.out` | `mfb build` executable (Linux) | `src/os/linux/link.rs` | Native executable (ELF, musl). |
| `<name>.mfp` | `mfb build` package | `src/target/package_mfp` | Compiled MFB package. |

## 16. Module Map

| Module | Responsibility |
| --- | --- |
| `src/main.rs` | CLI, manifest validation, project orchestration, package commands. |
| `src/lexer.rs` | Source tokenization. |
| `src/ast.rs` | Parser, AST model, AST JSON output. |
| `src/resolver.rs` | Name resolution and import/package symbol checks. |
| `src/monomorph.rs` | Template/generic expansion into concrete AST. |
| `src/typecheck.rs` | Type system, expression checking, flow validation. |
| `src/ir.rs` | Shared compiler IR and AST-to-IR lowering. |
| `src/binary_repr.rs` | MFPC binary representation lowering, encoding, decoding, package ABI inspection. |
| `src/builtins/mod.rs` | Built-in package dispatch and parameter name tables. |
| `src/builtins/fs.rs` | Filesystem built-in signatures and validation. |
| `src/builtins/general.rs` | General and collection built-in signatures. |
| `src/builtins/io.rs` | IO built-in signatures and validation. |
| `src/builtins/json.rs` | JSON built-in type and call signatures. |
| `src/builtins/math.rs` | Math built-in signatures and constants. |
| `src/builtins/strings.rs` | String built-in signatures. |
| `src/builtins/thread.rs` | Thread built-in type and call signatures. |
| `src/unicode_backend.rs` | Unicode normalization and grapheme code generation. |
| `src/unicode_runtime_tables.rs` | Compile-time Unicode lookup tables embedded in generated code. |
| `src/target.rs` | Target parsing, backend registry, backend dispatch. |
| `src/target/shared/lower.rs` | Shared IR-to-NIR lowering pass. |
| `src/target/shared/nir.rs` | Native IR and import/runtime-call lowering. |
| `src/target/shared/runtime.rs` | Runtime helper discovery and helper ABI metadata. |
| `src/target/shared/validate.rs` | Native target, NIR, capability, and plan validation. |
| `src/target/shared/plan.rs` | Shared native plan lowering. |
| `src/target/shared/code/` | Shared native code-plan lowering (directory module with builder submodules). |
| `src/target/macos_aarch64/*` | macOS aarch64 backend wrappers and platform behavior. |
| `src/target/linux_aarch64/*` | Linux aarch64 backend wrappers and platform behavior. |
| `src/target/package_mfp` | MFP package container writer. |
| `src/arch/aarch64/*` | AArch64 ABI, operations, and binary instruction encoding. |
| `src/os/macos/*` | Mach-O object planning and executable writing. |
| `src/os/linux/flavor.rs` | Linux flavor enumeration (glibc/musl) and suffix/interpreter selection. |
| `src/os/linux/link.rs` | ELF object planning and executable writing. |
| `src/os/linux/object.rs` | ELF container layout planning. |
| `src/man/*` | Built-in package/function help text. |
| `src/rules.rs` | Diagnostic display support. |
| `src/numeric.rs` | Numeric parsing and representation helpers. |

## 17. Current Implementation Boundaries

The following boundaries are important when extending the compiler:

- Native executable support is target-limited to `macos-aarch64` and
  `linux-aarch64`.
- Native runtime-call support covers `io.*`, `fs.*`, and `thread.*` built-ins.
  `math` and `strings` operations are code-generated inline and do not go
  through the runtime-helper capability gate. `json` built-ins have no native
  backend support and are binary representation-only.
- `target/shared/validate.rs::validate_project` is currently a no-op, so target
  project-level checks must be implemented elsewhere or added there.
- Manifest source `include` and `exclude` patterns are not currently enforced
  by source discovery.
- *NOTE: `package_format.md` specifies that packages may carry a cryptographic
  signature. The current package writer always emits unsigned containers with
  `signatureType = 0` and `signatureLength = 0`.*
- `mfb pkg add` currently supports only absolute `file://` package URLs.
- Linux builds emit two output files per build (`-glibc.out` and `-musl.out`).
  The glibc flavor links `libpthread.so.0` separately; musl bundles pthread in
  `libc.musl-aarch64.so.1`.
- macOS executable writing supports `libSystem` imports only.

These boundaries should be treated as implementation facts, not necessarily
language or package-format design goals.

## 18. Extension Checklist

When adding a language feature or built-in that must work end to end, update
every layer that observes or emits that behavior:

1. Lexer/parser support, if syntax changes.
2. AST model and AST serialization.
3. Resolver rules for names, imports, constructors, and members.
4. Monomorphization rules for generic forms.
5. Type-checking rules and overload behavior.
6. IR lowering.
7. MFPC binary representation lowering, encoding, decoding, exports, and ABI hashing.
8. NIR lowering for native builds.
9. Runtime helper detection and native backend capabilities.
10. Native plan and native code-plan lowering.
11. AArch64 encoding, if new instruction forms are needed.
12. OS linker/container support, if relocations/imports/layout change.
13. Package dependency merge/import behavior, if packages are affected.
14. Valid and invalid function tests for every changed public function.
15. Acceptance suite updates only after proving mismatches are expected.
16. Runtime validation for executable behavior, not just generated artifacts.

This checklist follows the repository's completion rule: compiler output alone
does not prove a runtime feature works. Executable behavior must be validated by
running the generated program or by another observable runtime result.
