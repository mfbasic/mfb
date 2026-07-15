# plan-47: Windows x86-64 build target

Last updated: 2026-07-14
Overall Effort: huge (>3d)   <!-- whole feature: PE writer + Win64 ABI + Win32 runtime floor + fs/term/thread/net/tls surfaces -->

This plan adds a native **`windows-x86_64`** build target: `mfb build -target windows-x86_64`
emits a PE/COFF `.exe` that runs on 64-bit Windows and produces the same observable
behavior every other console target does. It reuses the existing x86-64 instruction
selector and encoder (`src/arch/x86_64/`) unchanged for instruction *bytes*, and adds
three genuinely new things: a **PE/COFF executable writer** (the Windows analog of the
ELF/Mach-O writers), a **Windows x64 calling-convention** realization (rcx/rdx/r8/r9 +
32-byte shadow space, a different callee-saved bank), and a **Win32 OS-interface**
(kernel32/ucrt/ws2_32/bcrypt via an Import Address Table — Windows has no stable
syscall ABI, so *every* OS call becomes an imported-DLL call).

The single behavioral outcome of the foundation (47-A..C): `mfb build -target
windows-x86_64 hello` produces `hello.exe` that, run on Windows x86-64, prints the same
bytes the Linux/macOS builds of the same program print and exits `0`. Later sub-plans
grow the OS surface (files, console, threads, sockets, TLS) until the target advertises
the full console runtime-call set the Linux x86-64 backend does today.

References (read first — these are the seams this plan parameterizes):

- `src/target.rs` — `BuildTarget`, the `NativeBackend` trait + `NATIVE_BACKENDS`
  registry, dispatch (`backend_for`).
- `src/target/linux_x86_64/{mod,plan,code}.rs` — the closest precedent: same ISA,
  different OS. The Windows backend mirrors this trio.
- `src/target/shared/code/types.rs:204` — the `CodegenPlatform` trait (the per-OS
  codegen seam); `src/target/shared/plan/mod.rs:152` — the `NativePlanPlatform` trait
  (the per-OS import-table seam).
- `src/target/shared/abi.rs` + `src/arch/x86_64/select.rs:107` (`remap_x86_abi`) +
  `src/arch/x86_64/regmodel.rs` — the ABI realization the Windows convention diverges
  from.
- `src/os/linux/link/{mod,elf}.rs` + `src/os/linux/object.rs` — the ELF writer the PE
  writer parallels; `src/os/macos/link/macho.rs` — the second precedent.
- `mfb spec linker …` (`src/docs/spec/linker/**`), `mfb spec memory 08_program-startup`
  and `06_native-calling-convention` — the startup/ABI contracts to extend.

## Feature map (the whole `47`)

Split by effort; each letter is an independently-landable small/medium/large plan.
A–C are the foundation (a runnable console `.exe`); D–H each add one OS surface and can
land in any order after C. The backend advertises each surface in its
`BackendCapabilities.runtime_calls` only once that surface's sub-plan lands, so partial
progress is always shippable and every accepted program provably works.

- **47-A — Target registration + Windows x64 ABI.** New `windows-x86_64` `BuildTarget`
  + `src/target/win_x86_64/` backend skeleton; a Windows-convention variant of the x86
  ABI remap (arg regs rcx/rdx/r8/r9, 32-byte shadow space, stack args past 4, Win64
  callee-saved bank incl. xmm6–15) and register model. Codegen-only; proven by
  selection/encoder unit tests. No executable yet. **Where the ABI risk concentrates.**
- **47-B — PE/COFF executable writer.** New `src/os/windows/{mod,object,link/}` writing
  a minimal PE32+ image (DOS stub, PE header, optional header, section table, `.text`,
  `.rdata`/`.data`, `.idata` import directory + IAT). Links a trivial program that only
  calls `ExitProcess` via the IAT into a `.exe` Windows runs with the expected exit
  code. **Where the format risk concentrates.**
- **47-C — Windows console runtime floor.** The `CodegenPlatform`/`NativePlanPlatform`
  Win32 impl for the machine floor: entry (`GetCommandLineW` + `CommandLineToArgvW`),
  arena (`VirtualAlloc`/`VirtualFree`), stdout/stderr (`GetStdHandle` + `WriteFile`),
  exit (`ExitProcess`), RNG seed (`BCryptGenRandom`). `hello.exe` and integer/string
  programs print correctly and exit. Depends on A, B.
- **47-D — Filesystem surface.** `fs::*` over Win32 (`CreateFileW`/`ReadFile`/
  `WriteFile`/`GetFileAttributesW`/`FindFirstFileW`/`FindNextFileW`/`MoveFileExW`/
  `GetTempPathW`/…) + UTF-8↔UTF-16 path marshaling. Depends on C.
- **47-E — Console/terminal surface.** `term::*` + `io::` terminal queries over the
  Console API (`GetConsoleMode`/`SetConsoleMode` for raw mode + VT processing,
  `GetConsoleScreenBufferInfo` for size). Depends on C.
- **47-F — Threads.** `thread::*` over `CreateThread`/`WaitForSingleObject` +
  `SRWLOCK`/`CONDITION_VARIABLE` (or `CRITICAL_SECTION`), replacing the pthread path in
  the shared trampoline/sync helpers behind a platform switch. Depends on C.
- **47-G — Networking.** `net::*` over Winsock2 (`WSAStartup`, `socket`/`connect`/
  `bind`/`listen`/`accept`/`recv`/`send`, `closesocket`, `ioctlsocket`,
  `getaddrinfo`), with the Winsock error/const divergences abstracted. Depends on C.
- **47-H — Crypto + TLS transport.** `crypto::*` over CNG/BCrypt and a `tls::*` Schannel
  backend (the third sibling to `code/tls/{openssl,macos}.rs`). Depends on C, G.

Bounded-surface evidence: the machine floor (47-C) needs **six** kernel32 imports
(`GetStdHandle`, `WriteFile`, `VirtualAlloc`, `VirtualFree`, `ExitProcess`,
`GetCommandLineW`) plus `CommandLineToArgvW` (shell32) and `BCryptGenRandom` (bcrypt) —
a tiny, fixed IAT. Each later surface adds one DLL's worth of imports on the same
mechanism.

## 1. Goal

- A `windows-x86_64` `BuildTarget` selectable via `mfb build -target windows-x86_64`
  that emits a PE32+ (`PE\0\0`, machine `0x8664`, subsystem `CONSOLE`) `.exe`.
- The `.exe` runs on 64-bit Windows and, for any program using only the surfaces whose
  sub-plans have landed, produces byte-identical stdout/stderr and the same exit code as
  the Linux x86-64 build of the same program.
- All OS access is via DLL imports through an Import Address Table — no `syscall`
  instruction is emitted for the Windows target.
- The x86-64 instruction *bytes* (`src/arch/x86_64/encode/`) are reused unchanged; only
  ABI realization (register roles / frame shape) and the container/OS layers are new.

### Non-goals (explicit constraints)

- **No change to any existing target's output bytes.** macOS-aarch64, linux-aarch64,
  linux-x86_64, linux-riscv64 must stay byte-identical. The Windows ABI variant is
  selected by the new backend only; the existing `remap_x86_abi` / register model paths
  keep producing today's bytes. Hard regression guard (golden diff).
- **No language-surface, value/copy/move/freeze, layout, or IR change.** This plan is
  entirely in the ABI-realization, container, and OS-interface layers. The NIR / native
  plan / MIR are ISA-and-OS-neutral and are consumed unchanged.
- **No external assembler, linker, or MSVC/CRT toolchain in the shipped `mfb`.** The PE
  writer is built-in, exactly like the ELF/Mach-O writers. `link.exe`/`clang`/`dumpbin`
  are development-time oracles only (for validating PE structure), never invoked by a
  build.
- **No GUI/app mode** (`mfb build -app`) for Windows — console subsystem only. The
  backend returns `false` from `supports_app_mode()`; the CLI already rejects `-app` for
  non-app targets (`src/cli/build.rs:239`).
- **No cross-run of the `.exe` from the build host.** A non-Windows host builds the
  artifact and reports its path (the existing cross-target path, `src/cli/build.rs:498`);
  running/validation happens on a Windows box or under Wine in CI.
- **No dual-flavor split.** Unlike Linux (glibc/musl), Windows emits a single `.exe`;
  the backend does not loop a `Flavor::ALL`.

## 2. Current State

- **Targets & dispatch.** `BuildTarget { os, arch }` (`src/target.rs:16`), open strings
  parsed `os-arch` (`:75`). Registry `NATIVE_BACKENDS` (`:161`) holds the four current
  backends; `backend_for` (`:168`) linear-searches by `target()` and errors "native
  output does not support {} yet" otherwise. `"windows"` is currently a *rejected*
  target string, asserted by a negative test (`src/os/linux/mod.rs:87`). CLI parses
  `-target` at `src/cli/build.rs:149`.
- **The x86-64 ISA layer already exists and is OS-neutral.** `src/arch/x86_64/`:
  `select.rs` (neutral MIR → x86 ops; `remap_x86_abi` at `:107`), `encode/` (bytes),
  `regmodel.rs`, `reloc.rs` (`RelocIntent` → `call_pc32`/`data_pc32`/`got_pc32`,
  single RIP-relative refs). linux-x86_64 reuses all of it; Windows will too.
- **The OS seam is two traits.** `CodegenPlatform` (`src/target/shared/code/types.rs:204`,
  70 methods) emits the machine-floor + runtime-helper OS calls; `NativePlanPlatform`
  (`src/target/shared/plan/mod.rs:152`) maps helper specs → `PlatformImport { library,
  symbol, required_by }`. linux-x86_64 implements both: raw syscalls for
  write/read/mmap/munmap/getrandom/exit (`src/target/linux_x86_64/code.rs:53`), libc
  PLT calls for the rest (`emit_libc_call`, `:509`); import tables in
  `src/target/linux_x86_64/plan.rs`.
- **The shared entry/arena/runtime helpers are OS-parameterized, not OS-specific.**
  `lower_program_entry` (`src/target/shared/code/entry_and_arena.rs:4`) branches on
  `platform.entry_args_in_registers()` (`:44`): macOS argc/argv in x0/x1, raw-ELF on the
  stack. Windows needs a third path (raw entry, args via `GetCommandLineW`). Threads use
  pthreads (`src/target/shared/code/runtime_helpers.rs:600`, `_pthread_create` vs
  `pthread_create`) and pthread mutex/cond sync (`runtime_helpers_thread.rs`) —
  hardcoded, so Windows needs a platform switch there (47-F). TLS transport is
  `code/tls/{openssl.rs,macos.rs}` — Windows adds `schannel.rs` (47-H).
- **The linkable-image type is shared.** `EncodedImage`/`EncodedSection`/
  `EncodedRelocation`/`EncodedImport`/`ImportKind` live in `src/arch/aarch64/encode/`
  and are reused verbatim by the x86 encoder — the PE writer consumes the *same*
  `EncodedImage` the ELF writer does (`src/os/linux/link/mod.rs:53` takes `&EncodedImage`).
- **Writers are parallel siblings, not a shared trait.** `src/os/{linux,macos}/` each
  have `object.rs` (JSON plan, `container:"elf"`/`"mach-o"`) + `link/` (byte emission).
  A PE writer is a third sibling `src/os/windows/` with `container:"pe"`.

## 3. Design Overview

Three independent new pieces, layered under the existing pipeline
(NIR → NativePlan → NativeCodePlan → EncodedImage → container):

1. **ABI realization (47-A)** — the only change *inside* the x86 arch layer. The neutral
   `%arg`/`%ret`/`%sysarg` tokens and `remap_x86_abi` (`select.rs:107`) currently hard-map
   to SysV homes. Introduce an `X86Abi` selector (SysV | Win64) threaded from the backend
   through `mir::Backend::select` so the Windows backend picks the Win64 realization:
   `CALL_ARGS = [rcx, rdx, r8, r9]` (4, then a stack tail above the 32-byte shadow
   space), return `rax`, callee-saved `{rbx, rbp, rdi, rsi, r12–r15, xmm6–xmm15}`. The
   scratch-pool remap (`map_scratch_register`, `:36`) and `regmodel.rs` allocatable/
   caller-saved/callee-saved sets get Win64 variants. **There is no syscall path on
   Windows** — the Win64 `AbiBoundary::Syscall` arm is unreachable for this target (the
   OS calls are all `AbiBoundary::Call`), which simplifies the remap. The mandatory
   32-byte shadow space and the 4-register cap make `REGISTER_ARGUMENT_COUNT` and the
   frame's outgoing-arg reservation (`INCOMING/OUTGOING_ARGS_BASE`, `abi.rs:45`)
   convention-dependent — this is the subtle part; stack args past the 4th are currently
   unimplemented (`abi.rs:24` errors), so 47-A must implement the outgoing stack tail for
   x86 (it already exists as sentinels resolved in `finalize_frame`).

2. **PE/COFF writer (47-B)** — a self-contained `src/os/windows/`. Consumes
   `EncodedImage`; emits DOS header+stub, `PE\0\0` signature, COFF file header
   (`machine 0x8664`), optional header PE32+ (`magic 0x20b`, `ImageBase`,
   `AddressOfEntryPoint`, `Subsystem 3 = CONSOLE`, section alignment 0x1000/file
   alignment 0x200, data directories), a section table (`.text` RX, `.rdata` R,
   `.data` RW, `.idata` R), and the import directory + IAT built from
   `EncodedImage.imports` grouped by DLL. Relocations: reuse the neutral `RelocIntent` →
   a PE patcher — internal `Call`/`DataAddr` are RIP-relative (patched at link time,
   no base reloc needed for a fixed `ImageBase` with `/FIXED`-style output, or add a
   `.reloc` base-relocation table if ASLR/`DYNAMICBASE` is wanted); external `Call`
   (`GotLoad`-style) becomes `call [rip+IAT_slot]`. Start with a fixed `ImageBase` and no
   `.reloc` (simplest correct image); optionally add `.reloc` + `DYNAMICBASE` later.

3. **Win32 OS-interface (47-C..H)** — the `CodegenPlatform`/`NativePlanPlatform` impls in
   `src/target/win_x86_64/{code,plan}.rs`. Every OS call is an IAT call (`emit_libc_call`
   is reused verbatim — it already emits `bl symbol` + an external `RelocIntent::Call`;
   only the import *library* differs, e.g. `kernel32.dll`). The machine-floor methods that
   are raw syscalls on Linux (`emit_arena_map`, `emit_write`, `emit_program_exit`,
   `emit_random_bytes`) become IAT calls to `VirtualAlloc`/`WriteFile`/`ExitProcess`/
   `BCryptGenRandom` with the correct Win64 argument staging. `entry_args_in_registers`
   returns a value that selects the new `GetCommandLineW` entry path. The per-surface
   sub-plans (D–H) fill the remaining trait methods; each is gated behind its
   `runtime_calls` advertisement so an unimplemented surface is a clean compile-time
   rejection, never a broken `.exe`.

**Correctness risk** concentrates in 47-A (ABI: shadow space + callee-saved xmm + stack
args — a mistake here corrupts every non-trivial call) and 47-B (PE layout: a wrong
field silently fails to load). Both are validated against toolchain oracles (`dumpbin`/
`link.exe`-produced references) by structural diff, mirroring how plan-30-A validates
Mach-O against `otool -l`.

**Rejected alternatives.**
- *Link the MSVC CRT / `mainCRTStartup`.* Rejected: pulls in an external toolchain and a
  CRT dependency; the raw-entry + kernel32-IAT approach mirrors the static-ELF path and
  keeps `mfb` self-contained (a hard non-goal).
- *Emit `syscall` to `ntdll` sysnums directly.* Rejected: Windows syscall numbers are
  unstable across builds and unsupported — kernel32/IAT is the only sanctioned ABI.
- *A shared container trait unifying ELF/Mach-O/PE.* Rejected for this plan: the two
  existing writers are deliberately parallel siblings; forcing a trait now is scope the
  target doesn't need. PE joins as a third sibling.

## 4. Windows x64 ABI (47-A detail)

- Argument registers: `rcx, rdx, r8, r9`; FP args `xmm0–xmm3` (positionally aliased with
  the int slot — arg *n* is either `rN` or `xmmN`, never both). Return `rax` / `xmm0`.
- 32-byte "shadow space" (home space) reserved by the *caller* below the return address
  for the callee to spill its 4 register args; `rsp` must be 16-byte aligned at the point
  of a `call` (so ≡ 8 mod 16 on entry, like SysV). Reuse the existing entry/trampoline
  stack-bias machinery (`code.rs:337`) with Win64 numbers.
- Stack arguments: args ≥ 5 go on the stack *above* the shadow space. This requires the
  x86 outgoing/incoming stack-arg tail (`abi.rs:45`, currently sentinel-only and
  AArch64-resolved) to be realized for x86 — implement in `finalize_frame`.
- Callee-saved: `rbx, rbp, rdi, rsi, r12, r13, r14, r15, xmm6–xmm15`. Note `rsi`/`rdi`
  are callee-saved on Win64 but caller-saved on SysV, and `xmm6–15` are callee-saved on
  Win64 (none are under SysV) — the register model's caller/callee split and the
  prologue save set both change.
- Pinned registers stay the same physical assignment where free: `r15` = `arena_base`,
  the zero register, `%thread`/closure homes — verify none collide with a Win64 arg or
  the shadow-space usage.

## 5. Import Address Table & relocations (47-B detail)

- Group `EncodedImage.imports` by DLL (`kernel32.dll`, `shell32.dll`, `bcrypt.dll`, later
  `ucrtbase.dll`/`ws2_32.dll`/`advapi32.dll`). Build `.idata`: an import-directory-table
  (one `IMAGE_IMPORT_DESCRIPTOR` per DLL), import-lookup tables + import-address tables
  (parallel arrays of hint/name RVAs, null-terminated), and the hint/name table.
- External calls: `emit_libc_call` already produces `bl symbol` + external
  `RelocIntent::Call`. The PE patcher resolves it to `call [rip+disp32]` targeting the
  symbol's IAT slot (the Windows analog of the ELF GOT/PLT — `got_pc32` semantics, an
  indirect call, not a direct `call rel32`). Confirm the x86 encoder can emit the
  indirect `call [rip+disp32]` form for an external `Call` (it already emits GOT loads
  for data); if the current external `Call` selects a direct `call rel32`, add an
  indirect form gated on the import being a function (PE has no PLT stub layer).
- Data imports (e.g. none needed for the floor; `environ` has no Windows analog — use
  `GetEnvironmentStringsW`) resolve via IAT the same way.
- `.reloc`: omit initially (fixed `ImageBase`, no `DYNAMICBASE`); the image loads at its
  preferred base. Optionally add a base-relocation table later for ASLR compliance.

## Compatibility / Format Impact

- **New:** a `windows-x86_64` target; a `container:"pe"` object plan; a `.exe` artifact
  naming (`<name>.exe`, single file, no flavor suffix). New import libraries in plan
  tables. A new `X86Abi` parameter on the x86 selection path (internal).
- **Unchanged:** every existing target's emitted bytes (golden guard); NIR / native plan
  / MIR JSON schemas (OS-neutral already); the `EncodedImage` type; the language,
  builtins, resolver, and IR. The `CodegenPlatform`/`NativePlanPlatform` traits gain no
  new required methods (Windows implements the existing surface); if a Win64-only hook is
  unavoidable it is added with a default so other backends are untouched.

## Phases

Phases map 1:1 to the feature-map letters; land in order for A–C, then D–H in any order.
Each ships its own `planning/plan-47-<letter>-*.md` sub-plan when broken out; this master
doc carries the map and the shared design.

### Phase A — Target registration + Windows x64 ABI

Codegen-only ABI realization; no `.exe` yet, so it is safe to land behind unit tests
before the writer exists.

- [ ] Add `windows-x86_64` handling: a `src/target/win_x86_64/mod.rs` `Backend`
      (`target()` = `{os:"windows", arch:"x86_64"}`), registered in `NATIVE_BACKENDS`
      (`src/target.rs:161`), initially advertising an empty `runtime_calls` and
      `executable:false` (so dispatch resolves but execution is gated).
- [ ] Introduce an `X86Abi { SysV, Win64 }` selector threaded from the backend into
      `arch::x86_64::select` (via `mir::Backend`); `SysV` keeps byte-identical output.
- [ ] Add Win64 realizations: `CALL_ARGS`/return/`map_scratch_register` variants
      (`select.rs:36,67`), caller/callee-saved + xmm6–15 sets in `regmodel.rs`, and the
      x86 outgoing/incoming **stack-arg tail** in `finalize_frame` (past the 4th arg,
      above the 32-byte shadow space).
- [ ] Tests: `src/arch/x86_64/encode/tests.rs` + a new selection test module — a call
      with 6 int args realizes rcx/rdx/r8/r9 + two stack slots with a 32-byte shadow
      space and correct alignment; callee-saved prologue saves the Win64 bank; the SysV
      path is asserted byte-unchanged.

Acceptance: Win64 selection unit tests pass; `scripts/artifact-gate.sh` shows every
existing target byte-identical. Commit: —

### Phase B — PE/COFF executable writer

- [ ] New `src/os/windows/{mod,object}.rs` + `src/os/windows/link/{mod,pe}.rs`:
      `container:"pe"` object plan (mirroring `src/os/linux/object.rs`) and a PE32+ byte
      writer consuming `EncodedImage` (DOS stub, `PE\0\0`, COFF header machine `0x8664`,
      optional header, section table, `.text`/`.rdata`/`.data`/`.idata`).
- [ ] Build the import directory + IAT from `EncodedImage.imports` grouped by DLL; a PE
      relocation patcher realizing `RelocIntent` (internal RIP-relative; external `Call`
      → `call [rip+IAT]`).
- [ ] Wire `src/os/mod.rs` + `src/target/win_x86_64/mod.rs::write_executable` to call the
      new writer; drop/replace the `"windows"`-rejection negative test
      (`src/os/linux/mod.rs:87`) with the real path.
- [ ] Tests: `src/os/windows/link/tests.rs` — a trivial image (entry that calls
      `ExitProcess(42)` via a one-entry kernel32 IAT) produces bytes beginning `MZ`…`PE\0\0`,
      machine `0x8664`, subsystem CONSOLE; structural fields diff-match a `dumpbin`
      oracle checked into the test as expected constants.

Acceptance: the trivial `.exe` runs on Windows x86-64 (or Wine in CI) and exits `42`;
PE structure matches the oracle. Commit: —

### Phase C — Windows console runtime floor

- [ ] `src/target/win_x86_64/{code,plan}.rs`: `CodegenPlatform`/`NativePlanPlatform`
      Win32 impls — `emit_arena_map`→`VirtualAlloc`, `emit_arena_unmap`→`VirtualFree`,
      `emit_write`→`GetStdHandle`+`WriteFile`, `emit_program_exit`→`ExitProcess`,
      `emit_random_bytes`→`BCryptGenRandom`; import tables for kernel32/shell32/bcrypt.
- [ ] New Windows entry path: extend `entry_and_arena.rs` so `entry_args_in_registers`'s
      Windows case captures args via `GetCommandLineW` + `CommandLineToArgvW` (UTF-16→
      UTF-8 for `os::args`), instead of stack/reg argc-argv. Seed the RNG via
      `BCryptGenRandom` + a wall-clock mix (`GetSystemTimePreciseAsFileTime`).
- [ ] Flip the backend to `executable:true` and advertise the console `io.*`/`os.*`
      subset in `runtime_calls`.
- [ ] Tests: rt-behavior fixtures for `hello`, integer arithmetic, string print, and
      `os::args` run on Windows/Wine with stdout byte-identical to the Linux build.

Acceptance: `mfb build -target windows-x86_64 hello` → `hello.exe` prints the expected
bytes and exits `0` on Windows/Wine; output matches the linux-x86_64 build byte-for-byte.
Commit: —

### Phase D — Filesystem surface

- [ ] Implement the `fs`/path `CodegenPlatform` methods over Win32 (`CreateFileW`,
      `ReadFile`, `WriteFile`, `SetFilePointerEx`, `GetFileAttributesW`,
      `CreateDirectoryW`, `RemoveDirectoryW`, `DeleteFileW`, `FindFirstFileW`/
      `FindNextFileW`/`FindClose`, `MoveFileExW`, `GetTempPathW`, `GetFullPathNameW`) with
      UTF-8↔UTF-16 marshaling; map the `stat`/`dirent` layout accessors to Win32
      equivalents (`WIN32_FIND_DATAW`).
- [ ] Advertise the `fs.*` runtime calls; tests: the fs rt-behavior suite on Windows/Wine.

Acceptance: the `fs::` acceptance fixtures pass on Windows/Wine with matching output.
Commit: —

### Phase E — Console / terminal surface

- [ ] `term.*` + `io` terminal queries over the Console API (`GetConsoleMode`/
      `SetConsoleMode` incl. `ENABLE_VIRTUAL_TERMINAL_PROCESSING` for ANSI, raw-mode
      toggles; `GetConsoleScreenBufferInfo` for size; `GetStdHandle`/`GetFileType` for
      isatty). Advertise `term.*` runtime calls.
- [ ] Tests: the `term::` shadow-grid fixtures render correctly on a Windows console.

Acceptance: the `term::` acceptance fixtures pass on Windows/Wine. Commit: —

### Phase F — Threads

- [ ] Add a platform switch in the shared thread trampoline + sync helpers
      (`runtime_helpers.rs:600`, `runtime_helpers_thread.rs`) selecting Windows
      primitives: `CreateThread`/`WaitForSingleObject`/`TerminateThread` and
      `SRWLOCK`/`CONDITION_VARIABLE` (or `CRITICAL_SECTION`) instead of pthread; resize
      the mutex/cond storage (`os.rs:32,61`) for the Windows objects.
- [ ] Advertise `thread.*`; tests: the thread + stdin-broadcast rt-behavior suites on
      Windows/Wine.

Acceptance: the `thread::` acceptance fixtures pass on Windows/Wine. Commit: —

### Phase G — Networking

- [ ] `net.*` over Winsock2 (`WSAStartup` once at entry, `socket`/`connect`/`bind`/
      `listen`/`accept`/`recv`/`send`/`recvfrom`/`sendto`, `closesocket`,
      `ioctlsocket` for non-blocking, `getaddrinfo`/`freeaddrinfo`, `WSAPoll`); abstract
      the Winsock error codes / socket constants behind the existing `CodegenPlatform`
      accessors (`eagain`/`o_nonblock`/`so_*`, adding a Winsock error hook where the
      POSIX shape doesn't fit). Advertise `net.*`.
- [ ] Tests: the `net::` rt-behavior suite (loopback client/server) on Windows/Wine.

Acceptance: the `net::` acceptance fixtures pass on Windows/Wine. Commit: —

### Phase H — Crypto + TLS transport (highest-risk external surface last)

- [ ] `crypto::*` over CNG/BCrypt (`BCryptGenRandom` already in C; add the P-256/384/521
      key/sign/verify via BCrypt or reuse the existing library-backed shape); a `tls::*`
      Schannel backend as a third `code/tls/schannel.rs` sibling.
- [ ] Advertise `crypto.*`/`tls.*`; tests: the crypto + tls rt-behavior suites on
      Windows/Wine (with a network peer for tls).

Acceptance: the `crypto::`/`tls::` acceptance fixtures pass on Windows/Wine. Commit: —

## Validation Plan

- Tests: per-phase unit tests in `src/arch/x86_64/**` (ABI) and `src/os/windows/**` (PE),
  plus the existing rt-behavior/acceptance fixtures re-run for the Windows target on a
  Windows box or Wine. Negative cases: a program using an un-landed surface must fail the
  capability gate at build time with a clear "not yet supported on windows-x86_64" error,
  never emit a broken `.exe`.
- Runtime proof: `mfb build -target windows-x86_64 <prog>` on the build host produces
  `<prog>.exe`; the artifact is copied to a Windows runner (or Wine) and executed, with
  stdout/stderr/exit-code compared byte-for-byte against the linux-x86_64 build of the
  same program. The foundation proof is `hello.exe`.
- Oracle diffs: PE structure validated against `dumpbin`/`link.exe` reference constants
  (dev-time only), mirroring plan-30-A's `otool -l` Mach-O oracle.
- Regression guard: `scripts/artifact-gate.sh` (execution-free codegen gate) must show
  every existing target byte-identical after each phase, especially 47-A.
- Doc sync: add `src/docs/spec/linker/NN_windows-x86_64.md` (the PE emission contract);
  extend `mfb spec memory 06_native-calling-convention` (Win64 ABI) and
  `08_program-startup` (GetCommandLineW entry path); update any target list in
  `src/docs/**` and `.ai/compiler.md` if it enumerates targets.
- Acceptance: the project's full test suite (`scripts/test-accept.sh` + `cargo test`)
  green, with the Windows fixtures included in CI once a Windows/Wine runner is wired.

## Open Decisions

- **Runner in CI: Wine vs a native Windows runner.** Recommend Wine for the console
  floor/fs/term (fast, no new infra) and gate net/tls/thread on a real Windows runner if
  Wine's coverage proves insufficient. (§Validation)
- **ASLR: fixed `ImageBase` (no `.reloc`) vs `DYNAMICBASE` + base-reloc table.** Recommend
  fixed base first (simplest correct image), add `.reloc` + `DYNAMICBASE` as a later
  hardening step. (§5)
- **CRT dependency for math/formatting: none (in-tree) vs `ucrtbase.dll`.** Recommend
  none — the float formatter and libm kernels are already in-tree
  (`float_format.rs`, plan-01-libm-kernels), so the floor needs only kernel32/bcrypt.
  Revisit only if a helper has no in-tree form. (§3)
- **ABI plumbing: an `X86Abi` param on `select` vs a separate Win64 select module.**
  Recommend the parameter (keeps the encoder and 90% of selection shared; the SysV path
  stays byte-identical by construction). (§4)

## Summary

The real engineering risk is two-fold and front-loaded: the **Win64 ABI realization**
(shadow space, the 4-register arg cap with a real x86 stack-arg tail, and the
callee-saved bank including xmm6–15) in 47-A, and the **PE/COFF image layout + IAT** in
47-B — both silent-failure-prone and both validated against toolchain oracles. Everything
after is mechanical surface-by-surface work on the established `CodegenPlatform` seam,
where each OS call is just an IAT import and each surface lands independently behind its
capability gate. The x86-64 instruction encoder, the NIR/plan/MIR pipeline, every
existing target's bytes, and the entire language layer are untouched.
