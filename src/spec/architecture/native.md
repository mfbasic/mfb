# Native Executable Generation

The native executable back end: lowering IR through NIR, plans, AArch64 encoding, and OS linking.

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

## Native IR

Native IR, or NIR, is defined in `src/target/shared/nir.rs`.

NIR is close to the shared IR but adds native build concerns:

- Target name.
- Imported package functions with platform symbols.
- Runtime helper declarations.
- Native call forms for built-ins that require runtime support.

The NIR lowerer reads installed package exports and produces NIR imports. It
also rewrites supported built-in calls into runtime-call forms where needed.

`mfb build -nir` writes `<project>.nir`.

## Runtime Helper Selection

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

## Native Validation

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

## Native Plan

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

## Native Object Plan

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

## Native Code Plan

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

## AArch64 Encoding

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

## Linking and Executable Writing

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

## Runtime Value Memory Model

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
