# Native Executable Generation

The native executable back end: lowering IR through NIR, plans, AArch64 encoding, and OS linking.

Native executable generation is implemented across the target layer, the shared
target support, the architecture backends, and the OS layer.[[src/target]][[src/target/shared]][[src/arch]][[src/os]]

The active native backend registry lists:[[src/target.rs:NativeBackend]]

- `macos-aarch64`
- `linux-aarch64`
- `linux-x86_64`
- `linux-riscv64`

Each backend implements the `NativeBackend` trait. The trait exposes
capabilities and methods for executable and intermediate artifact emission.

The native executable pipeline is:

```text
IR
  -> shared IR-to-native lowering
  -> native IR (NIR)
  -> native validation
  -> per-target native plan
  -> shared planner
  -> OS object/container plan
  -> per-target native code plan
  -> shared code generator
  -> architecture encoding
  -> OS linking
  -> build/<project>.out
```

## Native IR

Native IR, or NIR, is defined by the native-IR layer.[[src/target/shared/nir/]]

NIR is close to the shared IR but adds native build concerns:

- Target name.
- Imported package functions with platform symbols.
- Runtime helper declarations.
- Native call forms for built-ins that require runtime support.

The NIR lowerer reads installed package exports and produces NIR imports. It
also rewrites supported built-in calls into runtime-call forms where needed.

`mfb build --nir` writes `<project>.nir`.

## Runtime Helper Selection

Runtime-helper detection is handled by the runtime-helper layer.[[src/target/shared/runtime/]]

The compiler scans IR values for calls into built-in packages. It records
which helper families are needed (the `RuntimeHelper` enum):[[src/target/shared/runtime/mod.rs:RuntimeHelper]]

- `crypto`
- `datetime`
- `fs`
- `general`
- `io`
- `math`
- `net`
- `os`
- `strings`
- `term`
- `thread`
- `tls`

`validate_capabilities` rejects native builds that require runtime calls not
listed in the backend capability set.[[src/target/shared/validate.rs:validate_capabilities]]
All four backends declare the same set of supported native runtime calls:

- `crypto.*` calls: `crypto.randomBytes` plus the P-256/384/521 key-generation,
  signing, and verification calls
- All `os.*` calls: `os.getEnv`, `os.getEnvOr`, `os.hasEnv`, `os.setEnv`,
  `os.unsetEnv`, `os.environ`, `os.args`, `os.pid`, `os.executablePath`,
  `os.name`, `os.arch`, `os.hostName`, `os.userName`, `os.cpuCount`
- All `io.*` calls: `io.print`, `io.write`, `io.flush`, `io.isBuffered`,
  `io.setBuffered`, `io.printError`, `io.writeError`, `io.input`, `io.readLine`,
  `io.readChar`, `io.readByte`, `io.pollInput`, `io.isInputTerminal`,
  `io.isOutputTerminal`, `io.isErrorTerminal`
- Most `fs.*` calls: `fs.open`, `fs.openFile`, `fs.openFileNoFollow`,
  `fs.createTempFile`, `fs.close`, `fs.readLine`, `fs.readAll`,
  `fs.readAllBytes`, `fs.writeAll`, `fs.writeAllBytes`, `fs.setBuffered`,
  `fs.isBuffered`, `fs.flush`, `fs.readText`,
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
  `tls.writeText`, `tls.close`, `tls.listen`, `tls.accept`, `tls.closeListener`

`math`, `strings`, and `general` operations are not listed as runtime helper
calls because they are code-generated inline rather than dispatched through
external runtime helpers. The `RuntimeHelper::General` variant exists, but
neither backend's `runtime_calls` contains any `general.*` call — `general.*`
built-ins (like `math`/`strings`) are inline-codegen'd, not a gated runtime-call
family.[[src/builtins/general.rs:is_general_call]] The complete, authoritative
capability set is the `runtime_calls` declaration in each backend; all four
backends declare the same set.[[src/target/macos_aarch64/mod.rs:runtime_calls]]

### Helper Requirement Analysis

`required_helpers` computes the exact set of `RuntimeHelper` families an IR
project needs, walking every function body and value recursively. Two cases are
not visible from plain runtime-call dispatch and are handled specially:[[src/target/shared/runtime/usage.rs:required_helpers]]

- **Resource-union binds.** A `Bind` of a resource type pulls in the helper for
  that type's close op. A `Bind` of a *resource-union* type drops by dispatching
  to each variant's close op (codegen-emitted, not a runtime call), so it pulls
  in the close helper for *every* variant of the union. The variant-close map is
  built once over all `union` types whose variants all map to a
  `resource_close_function`.
- **Thread `.result` member access.** A `MemberAccess` whose member is `result`
  pulls in `RuntimeHelper::Thread`, because reading a thread handle's result is
  serviced by the thread runtime even though no `thread.*` call appears in the
  IR.

Otherwise, helpers come from `Call`/`CallResult` targets via `helper_for_call`,
skipping native-direct calls (`is_native_direct_call`).

The declared==used invariant is enforced by `validate_nir`, which is the
authoritative gate on NIR shape. It first rejects any runtime helper declared
more than once, then accumulates the set of *used* helpers while validating each
function, and finally adds the variant-close helpers for any resource-union type
that is the subject of a `Bind` (mirroring `required_helpers`). It then enforces
both directions as hard errors:[[src/target/shared/validate.rs:validate_nir]]

- a used helper that is not in `module.runtime_helpers` is an
  `"NIR runtime call requires undeclared helper"` error;
- a declared helper that is not used is an
  `"NIR declares unused runtime helper"` error.

Capability gating is a separate check. `validate_capabilities` collects every
runtime call reached from the function bodies, and for each non-native-direct
call rejects the build if the call is outside the backend's
`capabilities.runtime_calls` set (`"native backend does not support runtime
call"`). It additionally rejects any declared helper that is actually used by an
emitted call but lacks a complete `supported_helper_specs` ABI entry (non-empty
params, returns, and clobbers) with `"native backend does not implement runtime
helper"`.[[src/target/shared/validate.rs:validate_capabilities]]

The concrete ABI (registers, clobbers, fallibility) for each helper family is
owned by `./mfb spec memory runtime-helper-abi`.

## Native Validation

Native validation is handled by the native validator.[[src/target/shared/validate.rs]]

It validates:

- Non-empty target fields.
- NIR project and function shape.
- Unique function and import names.
- Entry resolution.
- Runtime helper consistency.
- Backend runtime-call capability support.
- Native-plan and native-code-plan structural invariants.

Target-level `validate_project` is a no-op. Validation is therefore distributed
across the front-end passes, NIR validation, plan validation, code-plan
validation, and OS/linker checks rather than centralized in target project
validation.

## Native Plan

Native planning is implemented by platform-specific wrappers, one per target,
that both delegate to a shared planner.[[src/target/macos_aarch64/plan.rs]][[src/target/linux_aarch64/plan.rs]][[src/target/shared/plan/]]

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

`mfb build --nplan` writes `<project>.nplan`.

## Native Object Plan

OS object/container planning is implemented per OS.[[src/os/macos/object.rs]][[src/os/linux/object.rs]]

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

`mfb build --nobj` writes `<project>.nobj`.

## Native Code Plan

Native code planning is implemented by platform-specific wrappers, one per
target, that both delegate to a shared code generator.[[src/target/macos_aarch64/code.rs]][[src/target/linux_aarch64/code.rs]][[src/target/shared/code/]]

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

`mfb build --ncode` writes `<project>.ncode`.

### Register Allocation

Lowerings do not name physical temporary registers directly. `allocate_register`
mints an integer **virtual register**, carried in the instruction stream as the
sentinel `%vN`; `allocate_fp_register` mints a floating-point virtual register
`%fN`. After a function is fully lowered, a coloring pass rewrites every virtual
register to a physical
register, before the peephole pass and `finalize_frame` (which expect physical
names).[[src/target/shared/code/regalloc/mod.rs:allocate]]

The integer and FP/SIMD classes have separate physical files that never
interfere, so each is colored by an independent linear-scan pass over its own
operands.

A `Float` value is **`d`-register-native**: its canonical
home is a `d`-register, not a general-purpose register holding the bit pattern. A
float arithmetic result, a unary negation, and a `Float` local/global load all
carry their FP virtual register (`%fN`) directly in the value, so chained
arithmetic (`fadd d, d, d`), `fcmp` comparisons, the FP-domain finiteness check
and a `Float` local store/load (`str d`/`ldr d`) all stay in the FP
domain with no `fmov`-to-GPR shuttle and no stack traffic as integer bits. A GPR
copy is materialized **lazily**, through a single choke point, only at the
consumers that genuinely need the raw bits: integer/bitwise reinterpretation,
`toString`/print formatting, `toInt`/`toByte`/`toFixed` conversion, map-key
hashing, comparison spill, collection/record/closure-env element stores, call
arguments and returns (the `Float` arg/return ABI still travels in a GPR), and
thread marshalling. Storing a `d`-native float writes the same 8 bytes a `str x`
would (`str d`), so copy/transfer/golden output is unchanged — only the in-flight
register class differs. (The carrier is sound only under liveness-based coloring,
so the `bump` oracle keeps the legacy GP-native round-trip; `MOD`/`^` run
dedicated scalar kernels — their FP working set is allocator-placed virtual
registers plus the fixed `d0`–`d7` input/scratch bank — and stay GP-native too.)

A single-use `Float` `a*b±c` chain is **fused** into one single-rounded fused
multiply-add before register allocation: `a*b+c`→`fmadd_d`,
`a*b-c`→`fmsub_d`, `c-a*b`→`fnmsub_d`. The rewrite is decided on the neutral MIR
stream, so it fires identically on every backend, and applies only when the
product's FP virtual register is used exactly once — a product the program
observes (a named binding, a store, a second reader) keeps its own `fmul_d` and is
checked at that boundary like any other value. Fusion rounds the product
once instead of twice, so a fused result is `≤1` ULP and no worse than the discrete
form; the intermediate product is not a named `Float` and does not independently
trap, so `a*b+c` can be finite even where `a*b` alone would overflow.

A value live across a call stays in a callee-saved `d8`–`d15` rather than
spilling. A loop-carried float accumulator — a non-escaping `Float` local
assigned in a loop body — is **promoted** to a `d`-register held across the whole
loop: loaded from its slot once on entry, read and updated in the register each
iteration, and stored back once on exit, so the per-iteration slot round-trip
disappears. A local whose address is taken, or that is touched inside a nested
loop, is never promoted (its slot stays authoritative).

The allocator is split into two layers so every backend reuses the
core:

- **ISA-neutral core**: the virtual-register
  representation, the rewrite pass, and the pluggable `AllocationStrategy`
  interface. It names no physical registers.[[src/target/shared/code/regalloc]]
- **Per-ISA register model**: the `RegisterModel`
  trait answers every register question — the allocatable banks and their class
  (integer `x0`–`x30` vs FP/SIMD `d0`–`d31`, where `d_n` aliases the low 64 bits
  of the NEON `v_n`), the caller/callee-saved partition per class, ABI-pinned and
  scratch registers, and the per-class spill/reload emitters. FP spills are
  128-bit `str q`/`ldr q` into 16-byte slots — an FP virtual register can carry a
  SIMD vector (the `math::` kernels' working file), and a 64-bit `str d` would
  drop its high lane. All three ISAs — AArch64, x86-64, and RISC-V 64 —
  implement the trait.[[src/target/shared/regmodel.rs:RegisterModel]] [[src/arch/aarch64/regmodel.rs:Aarch64RegisterModel]]

The allocation method is a swappable `AllocationStrategy`, selected by the
`--regalloc <name>` build flag. The default, `linear-scan`, computes liveness over
the lowered stream and colors the integer class by live interval, spilling to a
stack slot under pressure (so a deeply nested expression no longer fails — it
spills); a value live across a call is spilled, since no register survives an
internal runtime helper. The `bump` strategy (`BumpAndReset`) replays the legacy
per-statement bump numbering and is byte-identical to the pre-allocator backend;
it is retained as the differential reference oracle (`--regalloc bump`). Further
strategies (graph-coloring) slot in without touching the rewrite pass or the
register model.

### The CodegenPlatform Seam

The shared code generator is OS-independent.[[src/target/shared/code/mod.rs]]
Everything that differs between macOS and Linux is funnelled through the
`CodegenPlatform` trait, implemented once per OS.[[src/target/macos_aarch64/code.rs]][[src/target/linux_aarch64/code.rs]][[src/target/shared/code/types.rs:CodegenPlatform]]

The seam carries two kinds of platform knowledge: ABI struct layouts queried as
scalar accessors, and `emit_*` methods that splice platform-specific
instructions into the helper bodies.

**`termios` layout.** Raw-input mode (used by `term`/raw console input) toggles
`ECHO`/`ICANON` and sets the `VMIN`/`VTIME` control characters directly in a
stack `termios` struct, so the generator must know that struct's per-OS shape:

| accessor | macOS | Linux |
| --- | --- | --- |
| `termios_size` | 72 | 60 |
| `termios_lflag_offset` | 24 | 12 |
| `termios_lflag_width` | 8 | 4 |
| `termios_cc_offset` | 32 | 17 |
| `termios_echo_flag` (ECHO) | 8 | 8 |
| `termios_icanon_flag` (ICANON) | 256 | 2 |
| `termios_vmin_index` (VMIN) | 16 | 6 |
| `termios_vtime_index` (VTIME) | 17 | 5 |

**`stat` mode offset.** `stat_mode_offset` gives the byte offset of `st_mode`
within the platform `stat` struct, used by file-/directory-existence checks: 4 on
macOS, 16 on Linux.

**libc call decoration.** `emit_libc_call` emits a `bl` to a libc function named
by its platform-independent base (e.g. `socket`, `getaddrinfo`): macOS prepends a
leading `_` and routes through libSystem (`emit_libsystem_call`), Linux uses the
name verbatim through libc (`emit_linux_c_call`). The `net` helpers marshal
socket calls onto this seam.[[src/target/macos_aarch64/code.rs:emit_libc_call]]

**`emit_*` strategies.** Beyond `emit_libc_call`, the trait exposes one method
per platform-divergent operation — program exit, write/poll/terminal IO, path
existence/stat, current/temp directory, fs path operations, errno, file
open/read/close/sync/seek, rename, `mkstemps`, directory open/read/close,
`realpath`, arena map/unmap, variadic calls, and the app-mode entry/IO/term
helpers. Each implementation supplies the OS-correct syscall or libc sequence
for that operation.

**`random_bytes`.** `emit_random_bytes` fills a buffer with OS entropy; both
backends call `getentropy` (decorated `_getentropy` on macOS via libSystem,
verbatim on Linux via libc).[[src/target/macos_aarch64/code.rs:emit_random_bytes]]

## AArch64 Encoding

Architecture-specific instruction encoding is handled by the AArch64 encoder.[[src/arch/aarch64]]

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

The final OS-specific executable writers are per OS.[[src/os/macos/link/]][[src/os/linux/link/]]
Both patch relocations in the encoded text, resolve the
entry symbol to a text offset, encode the OS executable container, and write the
output. macOS emits a single Mach-O `build/<project>.out`; Linux emits one ELF
per flavor (`build/<project>-glibc.out`, `build/<project>-musl.out`) into
the one output directory, and chooses static vs. dynamic by whether external
imports are present.

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
* ./mfb spec memory runtime-helper-abi — per-helper register/clobber/fallibility ABI
* ./mfb spec linker object-plan — Mach-O/ELF object layout planning
* ./mfb spec linker macos-aarch64 — Mach-O executable encoding
* ./mfb spec linker linux-aarch64 — ELF flavors and dynamic linking
* ./mfb spec linker static-and-dynamic-output — static vs. dynamic output selection
* ./mfb spec language memory-semantics — the source-level ownership model
