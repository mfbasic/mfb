# Native Executable Generation

The native executable back end: lowering IR through NIR, MIR, instruction selection, encoding, and OS linking.

Native executable generation is implemented across the target layer, the shared
target support, the architecture backends, and the OS layer.[[src/target]][[src/target/shared]][[src/arch]][[src/os]]

The active native backend registry lists:[[src/target.rs:NATIVE_BACKENDS]]

- `macos-aarch64`
- `linux-aarch64`
- `linux-x86_64`
- `linux-riscv64`
- `windows-x86_64`

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
  -> shared code generation
  -> MIR optimization
  -> instruction selection
  -> register allocation and frame finalization
  -> native code plan
  -> architecture encoding
  -> OS linking
  -> platform-specific artifacts under build/
```

## Native IR

Native IR, or NIR, is defined by the native-IR layer.[[src/target/shared/nir/]]

NIR is close to the shared IR but adds native build concerns:

- Target name.
- Final link symbols for merged functions and native `LINK` routing imports.
- Runtime helper declarations.
- Native call forms for built-ins that require runtime support.

Installed packages are decoded and merged into IR before NIR lowering; their
functions become ordinary NIR functions. NIR imports route native `LINK` calls,
and supported builtins are rewritten to runtime-call forms where needed.
[[src/target/shared/nir/lower.rs:merge_packages]]
[[src/target/shared/nir/lower.rs:link_routing_imports]]

`mfb build --nir` writes `<project>.nir`.

## Runtime Helper Selection

Runtime-helper detection is handled by the runtime-helper layer.[[src/target/shared/runtime/]]

The compiler walks IR values and resource cleanup requirements to select
runtime-helper families. `RuntimeHelper` contains `Abi`, `App`, `Audio`,
`Canvas`, `Datetime`, `Fs`, `General`, `Io`, `Math`, `Net`, `Os`, `Process`,
`Perf`, `Tcp`, `Term`, `Thread`, `Tls`, and `Udp`. `Perf` is compiler-injected
instrumentation rather than a source package.
[[src/target/shared/runtime/mod.rs:RuntimeHelper]]

Native-direct calls stay inline and do not require a runtime-call capability.
Other calls are mapped through `helper_for_call` and checked against the selected
backend's `capabilities().runtime_calls`; these tables are target-specific,
not a promise that every backend supports the same calls. `strings` has no
runtime-helper family. Source-companion functions compile through ordinary
function lowering.
[[src/target/shared/runtime/usage.rs:is_native_direct_call]]
[[src/target/shared/runtime/mod.rs:helper_for_call]]
[[src/target/shared/validate/capabilities.rs:validate_capabilities]]
[[src/target/win_x86_64/mod.rs:RUNTIME_CALLS]]

### Helper Requirement Analysis

`required_helpers` computes the exact set of `RuntimeHelper` families an IR
project needs, walking every function body and value recursively. Cleanup requirements are
not visible from plain runtime-call dispatch and are handled specially:[[src/target/shared/runtime/usage.rs:required_helpers]]

- **Resource-union binds.** A `Bind` of a resource type pulls in the helper for
  that type's close op. A `Bind` of a *resource-union* type drops by dispatching
  to each variant's close op (codegen-emitted, not a runtime call), so it pulls
  in the close helper for *every* variant of the union. The variant-close map is
  built once over all `union` types whose variants all map to a
  `resource_close_function`.
Otherwise, helpers come from `Call`/`CallResult` targets via `helper_for_call`,
skipping native-direct calls (`is_native_direct_call`).

The declared==used invariant is enforced by `validate_nir`, which is the
authoritative gate on NIR shape. It first rejects any runtime helper declared
more than once, then accumulates the set of *used* helpers while validating each
function, and finally adds the variant-close helpers for any resource-union type
that is the subject of a `Bind` (mirroring `required_helpers`). It then enforces
both directions as hard errors:[[src/target/shared/validate/mod.rs:validate_nir]]

- a used helper that is not in `module.runtime_helpers` is an
  `"NIR runtime call requires undeclared helper"` error;
- a declared helper that is not used is an
  `"NIR declares unused runtime helper"` error.

Capability gating is a separate check. `validate_capabilities` collects every
runtime call reached from the function bodies, and for each non-native-direct
call rejects the build if the call is outside the backend's
`capabilities.runtime_calls` set (`"native backend does not support runtime
call"`). It additionally rejects any declared helper that is actually used by an
emitted call but has no `supported_helper_specs` entry in its family with a
non-empty return type, reporting `"native backend does not implement runtime
helper"`.[[src/target/shared/validate/capabilities.rs:validate_capabilities]]

The concrete ABI (registers, clobbers, fallibility) for each helper family is
owned by `./mfb spec memory runtime-helper-abi`.

## Native Validation

Native validation is handled by the native validator.[[src/target/shared/validate/mod.rs]]

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
that delegate to a shared planner.[[src/target/macos_aarch64/plan.rs]][[src/target/linux_aarch64/plan.rs]][[src/target/shared/plan/]]

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

OS object/container planning is implemented per OS.[[src/os/macos/object.rs]][[src/os/linux/object.rs]][[src/os/windows/]]

The object plan is still a JSON planning artifact, not the final executable
container. It describes how the already planned native code will be arranged in
Mach-O, ELF, or PE terms:

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
target, that delegate to a shared code generator.[[src/target/macos_aarch64/code.rs]][[src/target/linux_aarch64/code.rs]][[src/codegen/]]

The native code plan records:

- Target and architecture.
- Project.
- Entry symbol.
- Imports.
- Data objects.
- Functions.
- Stack frames.
- Parameters and locations.
- Selected instruction operations for the target ISA.
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
names).[[src/codegen/engine/regalloc/mod.rs:allocate]]

Every virtual-register id comes from one source per compilation: `allocate_register`,
the standalone helpers' `Vregs::next` and the optimizer's GVN all draw from
`mint_vreg`, which `lower_module_for_platform` resets at the start of each module, so no
two minters can produce the same name within a stream and a module always compiles to
the same ids.[[src/codegen/engine/regalloc/mod.rs:mint_vreg]] [[src/codegen/engine/regalloc/mod.rs:reset_vreg_ids]]

A register operand is a typed `abi::Reg`, never text. It is a minted virtual register,
an ABI role (`c_arg`, `mfb_return`, …), a role token the ABI module defines (the zero,
link and stack-pointer registers, the pinned arena and current-thread registers, the
`SCRATCH`/`LOCAL` hand-allocation banks), or a physical register the allocator or a
backend chose (`Reg::physical`). It has no conversion from a string, so emission code
cannot name a register by hand; `CodeInstruction::field` rejects text in a register
field, and `physical_field` is the backends' post-allocation
entry.[[src/target/shared/abi.rs:Reg]] [[src/codegen/engine/builder/code_impl.rs:field]]

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
register class differs. (`MOD`/`^` run
dedicated scalar kernels — their FP working set is allocator-placed virtual
registers plus the fixed `d0`–`d7` input/scratch bank — and stay GP-native.)

The nine `vector::` shapes (`vector.Float2` … `vector.Integer4`, matched by their
package-qualified identity — a bare `Float3` is a user type) are
**register-native** the same way. A constructor keeps one scalar value per lane
and no block, and a field read returns the lane. An inlined op (`scale`, `dot`,
`cross`, `length`, `distance`, `lerp`, `normalize`, over re-evaluation-safe
operands) builds its result from lanes. The N×8-byte block is built only at a
storage boundary: a binding the analysis could not keep in lanes, a
collection/record store, a call argument, or a return. A returned vector's block
is claimed as the function's result, so it is built once and not copied.
[[src/codegen/builtins/vector/builder_vector_inline.rs:vector_field_count]]
[[src/codegen/builtins/vector/builder_vector_inline.rs:vector_value_as_block]]
[[src/codegen/engine/control/builder_exits.rs:lower_returned_value]]

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

On AArch64, an FP value live across a call may use callee-saved `d8`–`d15`.
Call survival is determined by the selected target's register model; SysV x86-64
has no callee-saved XMM bank.
[[src/codegen/engine/regalloc/analysis.rs:caller_saved_mask]] A loop-carried float accumulator — a non-escaping `Float` local
assigned in a loop body — is **promoted** to a `d`-register held across the whole
loop: loaded from its slot once on entry, read and updated in the register each
iteration, and stored back once on exit, so the per-iteration slot round-trip
disappears. A local whose address is taken, or that is touched inside a nested
loop, is never promoted (its slot stays authoritative).

The allocator is split into two layers so every backend reuses the
core:

- **ISA-neutral core**: the virtual-register
  representation, the liveness analysis, and the linear-scan coloring.
  It names no physical registers.[[src/codegen/engine/regalloc]]
- **Per-ISA register model**: the `RegisterModel`
  trait answers every register question — the allocatable banks and their class
  (integer `x0`–`x30` vs FP/SIMD `d0`–`d31`, where `d_n` aliases the low 64 bits
  of the NEON `v_n`), the caller/callee-saved partition per class, ABI-pinned and
  scratch registers, and the per-class spill/reload emitters. FP spills are
  128-bit `str q`/`ldr q` into 16-byte slots — an FP virtual register can carry a
  SIMD vector (the `math::` kernels' working file), and a 64-bit `str d` would
  drop its high lane. All three ISAs — AArch64, x86-64, and RISC-V 64 —
  implement the trait.[[src/target/shared/regmodel.rs:RegisterModel]] [[src/arch/aarch64/regmodel.rs:Aarch64RegisterModel]]

Linear scan is the only allocation method: it computes liveness over the
lowered stream and colors the integer class by live interval, spilling to a
stack slot under pressure (so a deeply nested expression never fails — it
spills); an integer value live across an internal runtime-helper call is spilled
because that call clobbers the entire integer bank in the allocator model. User
functions and external C calls use the target's caller-saved mask instead.
[[src/codegen/engine/regalloc/analysis.rs:call_clobber_mask]] Which value spills is a choice at `-O1` and above: when
no register is free for the value being colored, it takes the register of an
active value whose spill priority is lower than its own, and that value spills
instead. Priority is a value's accesses, each weighted `8^loop depth` (depth =
the CFG back edges spanning the instruction, capped at 3), divided by its
interval length — so a long-lived value touched rarely (a promoted local) goes
to memory before a loop temporary. At the same levels a value occupies its
register only over its busy runs, not the span from first to last mention:
two values whose runs never meet may share a register, which matters because
the lowering reuses one virtual register across separate loops. Every access
of a spilled value is then scheduled as a one-instruction interval that must
also get a register; when none is free, the lowest-priority value busy at that
instruction and not named by it spills to make room. At `-O0` a value holds its
register over its whole span and the value being colored always spills.
[[src/codegen/engine/regalloc/linear_scan.rs:SpillCosts]] [[src/codegen/engine/regalloc/analysis.rs:Liveness]]
A spilled value is reloaded into a free scratch register
at each use; when every allocatable register is live there, the allocator
**evicts** one the instruction does not name — saving it to an evict slot before
the instruction and restoring it after. The allocator runs after instruction
selection, so on RISC-V 64 that instruction can be a compare-and-branch (`rv.br`,
a block terminator with two successors). There the restore after the branch
covers only the fall-through edge, so the taken edge is split: the branch is
retargeted to a fresh label that restores the victim and jumps to the original
target. No flag-reading branch has a register operand, so no other target ever
evicts around a terminator.[[src/codegen/engine/regalloc/linear_scan.rs:run]]
[[src/codegen/engine/regalloc/linear_scan.rs:fresh_label]]
### The CodegenPlatform Seam

The shared code generator is OS-independent.[[src/codegen/engine/builder/mod.rs]]
Platform-specific emission is exposed through the `CodegenPlatform` trait,
implemented by the target wrappers.[[src/target/macos_aarch64/code.rs]][[src/target/linux_aarch64/code.rs]][[src/codegen/engine/types/types.rs:CodegenPlatform]]

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
macOS, 16 on Linux AArch64/RISC-V, and 24 on Linux x86-64.
[[src/target/linux_x86_64/code.rs:stat_mode_offset]]
[[src/target/linux_common/code.rs:stat_mode_offset_stays_per_arch]]

**libc call decoration.** `emit_external_call` emits a `bl` to a libc function named
by its platform-independent base (e.g. `socket`, `getaddrinfo`): macOS prepends a
leading `_` and routes through libSystem (`emit_libsystem_call`), Linux uses the
name verbatim through libc (`emit_linux_c_call`). The `net` helpers marshal
socket calls onto this seam.[[src/target/macos_aarch64/code.rs:emit_external_call]]

**`emit_*` strategies.** Beyond `emit_external_call`, the trait exposes one method
per platform-divergent operation — program exit, write/poll/terminal IO, path
existence/stat, current/temp directory, fs path operations, errno, file
open/read/close/sync/seek, rename, `mkstemps`, directory open/read/close,
`realpath`, arena map/unmap, variadic calls, and the app-mode entry/IO/term
helpers. Each implementation supplies the OS-correct syscall or libc sequence
for that operation. On macOS `realpath` binds libSystem's POSIX entry point,
`_realpath$DARWIN_EXTSN` — the symbol `<stdlib.h>` binds — never the bare legacy
`_realpath`, whose `getcwd` scans every parent directory of a relative path and
costs a directory read per call.[[src/target/macos_aarch64/code.rs:DARWIN_REALPATH_SYMBOL]]

**Windows marshalling buffers.** Every Windows path and environment seam calls a
wide (`*W`) Win32 API, converting UTF-8 to UTF-16 with `MultiByteToWideChar` and
back with `WideCharToMultiByte` through arena buffers. Each such buffer carries
an 8-byte header holding its block size, so whoever reads it last frees it
without tracking the size.[[src/target/win_x86_64/code.rs:MARSHAL_HEADER]] A
seam frees its own scratch before it returns. A UTF-8 input's UTF-16 copy is
sized to the string (`n` bytes never need more than `n + 1` UTF-16 units); a
Win32 call that writes a path or an environment value gets a 64 KiB buffer (the
32 768-wchar maximum), and the host, user and executable-name queries a 4 KiB
one (2 048 wchars).[[src/target/win_x86_64/code.rs:emit_wide_alloc_for_utf8]][[src/target/win_x86_64/code.rs:emit_os_wide_string]] Three seams
return arena memory instead of a host pointer — `emit_env_get`,
`emit_os_wide_string` and `emit_environ_pointer` — each as one block sized to
its answer (the environment's pointer array and all its strings share one). The
shared consumer releases it with `emit_release_marshalled` after copying it, at
the helper's single exit with the four result registers carried across the
call.[[src/codegen/engine/types/types.rs:emit_release_marshalled]][[src/codegen/builtins/os/gen_shared.rs:emit_release_marshalled_result]]
`emit_opendir`'s `DIR` belongs to the caller until `emit_closedir` frees it, and
`emit_opendir` frees it itself when the search cannot start. The `os::args`
vector is built once at entry and never freed.
`scripts/test-winleak.sh` checks on a Windows host that no seam's live bytes grow
with its call count.

**`random_bytes`.** `emit_random_bytes` fills a buffer with OS entropy; the macOS
and Linux backends call `getentropy` (decorated `_getentropy` on macOS via libSystem,
verbatim on Linux via libc). Windows uses `BCryptGenRandom`.
[[src/target/macos_aarch64/code.rs:emit_random_bytes]]
[[src/target/win_x86_64/code.rs:emit_random_bytes]]

## Architecture Encoding

Architecture-specific instruction encoding is handled by the selected AArch64,
x86-64, or RISC-V encoder. [[src/arch/aarch64]] [[src/arch/x86_64]] [[src/arch/riscv64]]

The encoder consumes the native code plan and produces an `EncodedImage` with:

- text bytes
- data bytes
- symbols
- relocations
- imports
- entry symbol

The encoder handles the selected ISA's instruction forms and ABI details used by the
native code plan.

## Linking and Executable Writing

The final executable writers are per OS. [[src/os/macos/link/]]
[[src/os/linux/link/]] [[src/os/windows/]] They patch relocations in the encoded text, resolve the
entry symbol to a text offset, encode the OS executable container, and write the
output. macOS emits a single Mach-O `build/<project>.out`; Linux emits one ELF
per flavor (`build/<project>-glibc.out`, `build/<project>-musl.out`) into
the one output directory, and chooses static vs. dynamic by whether external
imports are present. Windows emits `build/<project>.exe` as PE32+.
App output shapes are specified in `./mfb spec architecture artifacts`.

The container byte details — Mach-O segments and the ad hoc code signature,
`libSystem` imports and import stubs; the ELF static/dynamic split, PLT/GOT,
per-flavor `DT_NEEDED`/interpreter selection, and `0755` permissions — are owned
by the linker spec: `./mfb spec linker macos-aarch64`,
`./mfb spec linker linux-aarch64`, `./mfb spec linker static-and-dynamic-output`.

## Runtime Value Memory Model

Native code generation realizes the language's value semantics over a per-arena
heap of flat, pointer-free blocks: copies are a single `arena_alloc` + byte copy,
ownership is established by copy-insertion at long-lived store sites, and owned
non-escaping locals are freed at scope exit. A value of a recursive type is the one
exception to "pointer-free": it is a graph of separate blocks, copied at every store
by a non-recursive graph copy (or moved at its source's last read) and freed by its
owner through a non-recursive graph drop, including the elements a collection discards
in place. The `memory` spec is the authority
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
