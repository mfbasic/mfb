# Native Executable Generation

The native executable back end: lowering IR through NIR, plans, AArch64 encoding, and OS linking.

Native executable generation is implemented under `src/target`,
`src/target/shared`, `src/arch`, and `src/os`.

The active native backend registry is in `src/target.rs`:[[src/target.rs:NativeBackend]]

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
  -> target/shared/code/ (directory module: mod.rs + builder_*.rs submodules)
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
which helper families are needed (the `RuntimeHelper` enum in `runtime.rs`):[[src/target/shared/runtime.rs:RuntimeHelper]]

- `datetime`
- `fs`
- `general`
- `io`
- `math`
- `net`
- `strings`
- `term`
- `thread`
- `tls`

`validate_capabilities` rejects native builds that require runtime calls not
listed in the backend capability set.[[src/target/shared/validate.rs:validate_capabilities]]
Both `macos-aarch64` and `linux-aarch64` currently declare the same set of
supported native runtime calls:

- All `io.*` calls: `io.print`, `io.write`, `io.flush`, `io.printError`,
  `io.writeError`, `io.flushError`, `io.input`, `io.readLine`, `io.readChar`,
  `io.readByte`, `io.pollInput`, `io.isInputTerminal`, `io.isOutputTerminal`,
  `io.isErrorTerminal`
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
  `thread.isCancelled`, `thread.transferResource`, `thread.acceptResource`
- All `datetime.*` calls: `datetime.nowNanos`, `datetime.monotonicNanos`,
  `datetime.localOffset`
- All `term.*` calls: `term.on`, `term.off`, `term.isOn`, `term.clear`,
  `term.moveTo`, `term.hideCursor`, `term.showCursor`, `term.setForeground`,
  `term.getForeground`, `term.setBackground`, `term.getBackground`,
  `term.setBold`, `term.getBold`, `term.setUnderline`, `term.getUnderline`,
  `term.terminalSize`
- All `net.*` calls: `net.lookup`, `net.connectTcp`, `net.listenTcp`,
  `net.accept`, `net.bindUdp`, `net.read`, `net.readText`, `net.write`,
  `net.writeText`, `net.sendTo`, `net.sendTextTo`, `net.receiveFrom`,
  `net.receiveTextFrom`, `net.localAddress`, `net.remoteAddress`, `net.close`,
  `net.poll`, `net.setReadTimeout`, `net.setWriteTimeout`
- All `tls.*` calls: `tls.connect`, `tls.read`, `tls.readText`, `tls.write`,
  `tls.writeText`, `tls.close`

`math`, `strings`, and `general` operations are not listed as runtime helper
calls because they are code-generated inline rather than dispatched through
external runtime helpers. The `RuntimeHelper::General` variant exists, but
neither backend's `runtime_calls` contains any `general.*` call — `general.*`
built-ins (like `math`/`strings`) are inline-codegen'd, not a gated runtime-call
family.[[src/builtins/general.rs:is_general_call]] The complete, authoritative
capability set is the `runtime_calls` declaration in each backend
(`src/target/macos_aarch64/mod.rs`, `src/target/linux_aarch64/mod.rs`); both
backends currently declare the same set.[[src/target/macos_aarch64/mod.rs:runtime_calls]]

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

macOS object plans target a Mach-O layout; Linux object plans target an ELF
layout. The concrete segment/section regions are owned by
`./mfb spec linker object-plan`.

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

The final OS-specific executable writers are `src/os/macos/link.rs` and
`src/os/linux/link.rs`. Both patch relocations in the encoded text, resolve the
entry symbol to a text offset, encode the OS executable container, and write the
output. macOS emits a single Mach-O `<project>.out`; Linux emits one ELF per
flavor (`<project>-glibc.out`, `<project>-musl.out`) and chooses static vs.
dynamic by whether external imports are present.

The container byte details — Mach-O segments and the ad hoc code signature,
`libSystem` imports and import stubs; the ELF static/dynamic split, PLT/GOT,
per-flavor `DT_NEEDED`/interpreter selection, and `0755` permissions — are owned
by the linker spec: `./mfb spec linker macos-aarch64`,
`./mfb spec linker linux-aarch64`, `./mfb spec linker static-and-dynamic-output`.

## Runtime Value Memory Model

Native code generation realizes the language's value semantics over a per-arena
heap of flat, pointer-free blocks: copies are a single `arena_alloc` + byte copy,
ownership is established by copy-insertion at long-lived store sites, and owned
non-escaping locals are freed at scope exit. The `memory` spec is the authority
for the value layout, arena mechanism, and scope-drop frees —
`./mfb spec memory heap-values` and `./mfb spec memory arenas`.

## See Also

* ./mfb spec memory heap-values — the flat-block value layout
* ./mfb spec memory arenas — the arena allocator and scope-drop frees
* ./mfb spec linker object-plan — Mach-O/ELF object layout planning
* ./mfb spec linker macos-aarch64 — Mach-O executable encoding
* ./mfb spec linker linux-aarch64 — ELF flavors and dynamic linking
* ./mfb spec linker static-and-dynamic-output — static vs. dynamic output selection
* ./mfb spec language memory-semantics — the source-level ownership model
