# plan-47: Windows x86-64 build target

Last updated: 2026-07-20
Overall Effort: huge (>3d) — PE writer + Win64 ABI + Win32 runtime floor + fs/term/thread/net/tls surfaces

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
  registry (`:197`), dispatch (`backend_for`).
- `src/target/linux_common/{mod,code}.rs` — **the real precedent.** One
  `CodegenPlatform` impl (`code.rs:302`) serves all three Linux arches, parameterized by
  a `LinuxArch` ISA delta. `src/target/linux_x86_64/` is a thin arch binding, not a
  standalone platform.
- `src/target/macos_aarch64/code.rs:38` — the second `CodegenPlatform` impl, and the
  model Windows follows: a whole new OS, not an arch variant of an existing one.
- `src/target/shared/code/types.rs:212` — the `CodegenPlatform` trait;
  `src/target/shared/plan/mod.rs:154` — the `NativePlanPlatform` trait.
- `src/target/shared/abi.rs` + `src/arch/x86_64/select.rs:107` (`remap_x86_abi`) +
  `src/arch/x86_64/regmodel.rs` — the ABI realization the Windows convention diverges
  from.
- `src/os/linux/link/{mod,elf}.rs` + `src/os/linux/object.rs` — the ELF writer the PE
  writer parallels; `src/os/macos/link/macho.rs` — the second precedent.
- `src/target/shared/code/io_helpers.rs:866` and `src/target/shared/code/net/` — the
  **shared, POSIX-shaped** lowering that §3.1 is about. Read before scoping E/F/G.
- `mfb spec linker …` (`src/docs/spec/linker/**`), `mfb spec memory 08_program-startup`
  and `06_native-calling-convention` — the startup/ABI contracts to extend.

## Prerequisites

These are a precondition on the whole feature, not a dependency to negotiate. plan-47
does not finish, port, or work around any of them.

| Must be true | Command | Status 2026-07-20 |
|---|---|---|
| An x86-64 backend + a shipping x86-64 OS target exist to mirror | `ls src/arch/x86_64/ src/target/linux_x86_64/` | **MET** |
| A Windows x86-64 machine is reachable for runtime proof | `grep -n 'Win11' .ai/remote_systems.md` → `:11`, ssh port 2230 | **MET** |
| Byte-identity goldens exist for **every** target whose bytes must not change (§1 non-goal 1) | see the §2.1 census row | **PARTIAL — `linux-riscv64` has 0** |
| plan-57's `kind = 2` flip has either not happened or has already rebaselined — never straddle it | `rg -n 'MFB_KIND2' src/` → gate still live at `builder_collection_layout.rs:2196` | **MET (not yet flipped)** — but see the note below |

> **NOTE — the Status column is a 2026-07-20 snapshot; the Command column is the truth.**
> Re-run all four and update the statuses before you continue, and again before you
> decide to stop. Never act on a status you did not just verify.
>
> **If you stop, report the status of all four rows**, not just the one that blocked you.

**On row 3.** §1's first non-goal is "no change to any existing target's output bytes",
guarded by golden diff. That guard covers `macos-aarch64` (66 goldens), `linux-x86_64`
(6) and `linux-aarch64` (6) — and **`linux-riscv64` not at all** (§2.1). 47-A only
touches the x86 path, so its guard exists. But **E, F and G edit shared lowering that
riscv64 compiles through** (§3.1), and there the guard is absent. Seed riscv64
byte-identity goldens before starting E, F or G — not before A or B.

**On row 4 — a baseline conflict, not a file conflict.** plan-57 edits
`builder_collection_{layout,mutate,queries,query}.rs` and `link_thunk.rs`. **No plan-47
phase touches any of them**, so the two plans can proceed in parallel at source level.

The collision is in the *acceptance criterion*. Several plan-47 phases are inert
refactors whose entire proof is "`scripts/artifact-gate.sh` reports 0 diffs". When
plan-57-D flips `MFB_KIND2` to the default and removes the gate, `kind = 2` changes the
emitted bytes of **every list-bearing program on every target** — mass golden churn. A
zero-diff phase that straddles that flip cannot prove anything.

So: land every zero-diff phase **either before plan-57-D's flip or after its rebaseline
commit, never across it.** Check whether plan-57's own byte-identity anchor
(`d8893f0ef`, "add a `.ncode` byte-identity anchor for list codegen") can serve both
plans before seeding a second one.

## 2. Current State

### 2.1 Measured populations

Every number that sizes this plan, with the command that produced it.

| What | Count | Command |
|---|---|---|
| `CodegenPlatform` trait methods (span `types.rs:212-630`) | **65** — **54 required, 11 defaulted** | `awk '/pub\(crate\) trait CodegenPlatform/,0' src/target/shared/code/types.rs \| awk '/^}/{exit} /^    fn /{c++} END{print c}'` |
| — implemented by the Linux platform | **64** | same over `impl<A: LinuxArch> code::CodegenPlatform` at `linux_common/code.rs:302` |
| — implemented by the macOS platform | **63** | same over `macos_aarch64/code.rs:38` |
| — that are **app-mode only** (a stated non-goal here) | **8** | `… \| grep -c '^app_\|^emit_app_'` |
| — that are **POSIX struct-offset constants** (termios/dirent/stat) | **11** | `… \| grep -c '^termios_\|^dirent_\|^stat_'` |
| — that are **POSIX socket constants** | **10** | `… \| grep -cE '^(eagain\|einprogress\|emsgsize\|o_nonblock\|so_\|sol_\|addrinfo_)'` |
| `NativePlanPlatform` trait methods | **8** | `awk '/trait NativePlanPlatform/,/^}/' src/target/shared/plan/mod.rs \| grep -cE '^\s+fn '` |
| Backends in `NATIVE_BACKENDS` | 4 | `src/target.rs:197` |
| Golden dirs in `tests/` | 1021 | `find tests -type d -name golden \| wc -l` |
| — carrying **any** native artifact | **28** (2.7%) | `find tests -path '*/golden/*' \( -name '*.ncode' -o -name '*.ncodesum' -o -name '*.nobj' -o -name '*.nir' -o -name '*.nplan' -o -name '*.mir' \) \| xargs -n1 dirname \| sort -u \| wc -l` |
| Native goldens by target | macos-aarch64 **66**, app 9, linux-x86_64 **6**, linux-aarch64 **6**, **linux-riscv64 0** | same find, bucketed on the `<target>` filename infix |

**A new OS platform must author 54 methods.** Only 11 have defaults, and they are
exactly the ones a console-only OS wants free: the **8 app-mode** methods, plus
`emit_tls_block_trampolines`, `entry_args_in_registers`, and `libc`. So the app-mode
non-goal costs Windows nothing — it inherits all 8.

Of the 54 required, **21 are POSIX ABI constants** for structures Windows does not have
(§3.1). That is the part of this plan nobody scoped.

This also refutes 47-A's own Phase 4, which says `win_x86_64/code.rs` is "a minimal
`CodegenPlatform` … leave the trait's own defaults." With 54 required methods that does
not compile, and the `termios_*`/offset accessors return plain `usize` — they cannot
even carry an "unimplemented" error, so they must return fabricated values. The stub
wall is real work currently hidden inside A.

### 2.2 The seams

- **Targets & dispatch.** `BuildTarget { os, arch }` (`src/target.rs:16`), parsed
  `os-arch` (`:79`). `NATIVE_BACKENDS` (`:197`) holds four backends; `backend_for`
  (`:247`) linear-searches and errors otherwise. `"windows"` is currently a *rejected*
  target string, asserted by a negative test at **`src/os/linux/mod.rs:141`**
  (`write_native_object_plan_propagates_lowering_error`, setting `plan.target =
  "windows"` at `:143`). That test asserts an ELF container rejects a non-Linux target —
  **it must stay.** The draft's Phase B said to drop it.
- **The x86-64 ISA layer is OS-neutral.** `src/arch/x86_64/`: `select.rs`
  (`remap_x86_abi` at `:107`), `encode/`, `regmodel.rs`, `reloc.rs`. linux-x86_64 reuses
  all of it; Windows will too.
- **The OS seam is two traits, and Linux is not the shape to copy.** `CodegenPlatform`
  is implemented **twice** in the tree: once for all of Linux
  (`linux_common/code.rs:302`, generic over `LinuxArch`) and once for macOS
  (`macos_aarch64/code.rs:38`). Windows is a new OS, so it mirrors **macOS's** shape — a
  standalone impl — not `linux_common`'s arch-parameterized one.
- **The shared entry/arena/runtime helpers are OS-parameterized, not OS-specific.**
  `lower_program_entry` (`shared/code/entry_and_arena.rs:4`) branches on
  `platform.entry_args_in_registers()`; Windows needs a third path (raw entry, args via
  `GetCommandLineW`). Threads are hardcoded pthreads
  (`shared/code/runtime_helpers.rs:600`, `runtime_helpers_thread.rs`) — 47-F. TLS is
  `code/tls/{openssl,macos}.rs` — Windows adds a third — 47-H.
- **The linkable-image type is shared.** `EncodedImage` and friends live in
  `src/arch/aarch64/encode/` and are reused verbatim by the x86 encoder, so the PE
  writer consumes the *same* `EncodedImage` the ELF writer does.
- **Writers are parallel siblings, not a shared trait.** `src/os/{linux,macos}/` each
  have `object.rs` + `link/`. PE is a third sibling `src/os/windows/`.

### 2.3 Verified properties

Claims a `file:line` cannot settle.

| Claim | Verdict | How checked |
|---|---|---|
| The x86 ISA layer is OS-neutral and reusable unchanged | **CONFIRMED** | `linux_x86_64` consumes `arch/x86_64/` with no OS conditionals in the encoder |
| "linux-x86_64 implements both traits" | **MISLEADING** | `CodegenPlatform` is implemented in `linux_common/code.rs:302` for all three Linux arches; `linux_x86_64` supplies only the `LinuxArch` delta and `NativePlanPlatform` (`plan.rs:53`) |
| "The traits gain no new required methods (Windows implements the existing surface)" | **FALSE** | §3.1 — 21 of the required methods describe POSIX structs Windows lacks, and their *consumers* in shared lowering build those structs inline |
| 47-F is the only sub-plan that edits shared lowering | **FALSE** | §3.1 — E (`io_helpers.rs:866`) and G (`shared/code/net/`, `tls/`) do too |
| Existing targets are guarded byte-identical | **PARTIAL** | 28/1021 fixtures carry native goldens; `linux-riscv64` has **zero** (§2.1) |
| A Windows box is available for runtime proof | **CONFIRMED** | `.ai/remote_systems.md:11`, ssh port 2230 |
| Stack args past the register cap are unimplemented on x86 | **CONFIRMED** | `shared/abi.rs:16-24` errors past the cap; sentinels exist but are AArch64-resolved |

## 3. Design Overview

Three independent new pieces, layered under the existing pipeline
(NIR → NativePlan → NativeCodePlan → EncodedImage → container):

1. **ABI realization (47-A)** — an `X86Abi` selector (SysV | Win64) threaded from the
   backend through `mir::Backend::select`, so the Windows backend picks Win64:
   `CALL_ARGS = [rcx, rdx, r8, r9]` then a stack tail above the 32-byte shadow space,
   return `rax`, callee-saved `{rbx, rbp, rdi, rsi, r12–r15, xmm6–xmm15}`. **There is no
   syscall path on Windows** — the `AbiBoundary::Syscall` arm is unreachable for this
   target. Requires implementing the x86 outgoing stack-arg tail (`shared/abi.rs:16-24`
   currently errors past the cap).
2. **PE/COFF writer (47-B)** — a self-contained `src/os/windows/` consuming
   `EncodedImage`; DOS stub, `PE\0\0`, COFF header (`machine 0x8664`), PE32+ optional
   header (`magic 0x20b`, `Subsystem 3 = CONSOLE`), section table, and `.idata`
   import directory + IAT built from `EncodedImage.imports` grouped by DLL. External
   calls become `call [rip+IAT_slot]`. Fixed `ImageBase`, no `.reloc` initially.
3. **Win32 OS-interface (47-C..H)** — the platform impls in
   `src/target/win_x86_64/{code,plan}.rs`. `emit_libc_call` is reused verbatim; only the
   import *library* differs. Each surface is gated behind its `runtime_calls`
   advertisement, so an unimplemented surface is a clean compile-time rejection, never a
   broken `.exe`.

### 3.1 The unscoped problem: the shared layer is POSIX-shaped in four ways

**This is the plan's largest design uncertainty and the 2026-07-14 master did not
mention any of it.** That draft claimed the traits "gain no new required methods
(Windows implements the existing surface)", which framed C–H as *adding methods to a
Windows platform object*. Windows-specific behavior actually has to live in **four**
places, and only the first is reachable through the trait:

| # | Where POSIX is baked in | Size | Reachable via `CodegenPlatform`? |
|---|---|---|---|
| 1 | POSIX ABI **constants** on the trait (`termios_*`, `dirent_*`, `stat_mode_offset`, socket constants) | 21 methods | Yes — but see below, the *consumers* still assume the struct |
| 2 | Hardcoded POSIX **symbol-name literals** in shared lowering, passed to `emit_libc_call` | ~125 sites | **No.** The trait changes *how* a symbol is called, never *which* symbol |
| 3 | Binary `platform.target()` **branches** in shared lowering | **20 sites** | **No** — and they fail silently (§3.2) |
| 4 | Backend **dispatch** functions (`crypto_ec.rs:113`, the TLS arms, `mod.rs:680`/`:703`) | ~12 sites | No — Windows falls into the OpenSSL/else arm |

Category 2, by surface: sockets **32** sites (`net/mod.rs` 14, `net/io.rs` 16,
`net/poll.rs` 2); pthreads **~85** (`runtime_helpers_thread.rs`, `stdin_broadcast.rs`,
`runtime_helpers.rs`, `os.rs`); termios **6** (`io_helpers.rs:825,838,911,952,1034`,
`term.rs:470`); clock 2+ (`datetime.rs:72`). Winsock needs `closesocket` not `close`,
`ioctlsocket`/`WSAPoll` not `fcntl`/`poll`, and `WSAStartup` with no POSIX analog at
all — so **G is the same kind of work as F, at 38% the scale**, not the
"just add methods" phase the map implies.

On category 1, the constants are worse than they look because their consumers build the
POSIX struct inline:

| Group | Count | Example consumer (shared — every backend compiles it) |
|---|---|---|
| `termios_*` (size, lflag offset/width, cc offset, vmin/vtime index, echo/icanon flags) | 8 | `shared/code/io_helpers.rs:866` — builds a `termios` struct inline at `slots.modified + platform.termios_lflag_offset()` |
| `dirent_*`, `stat_mode_offset` | 3 | `fs` lowering reads `struct dirent` / `struct stat` at these offsets |
| socket constants (`sol_socket`, `so_*`, `o_nonblock`, `eagain`, `einprogress`, `emsgsize`, `addrinfo_addr_offset`) | 10 | `shared/code/net/mod.rs`, `net/poll.rs`, `tls/mod.rs`, `tls/openssl.rs` |

Windows has no `termios` (it has `GetConsoleMode`/`SetConsoleMode` over a DWORD
bitmask), no `dirent` (`WIN32_FIND_DATAW`), and no `struct stat` in that shape. Winsock
does define `SOL_SOCKET`/`SO_*`, but `O_NONBLOCK` is `ioctlsocket(FIONBIO)` and the
error codes are `WSAE*`, not `EAGAIN`.

So there is **no set of integers a Windows platform can return that makes
`io_helpers.rs:866` correct**. The consumer must branch, which means editing shared
lowering — and that is why E, F and G are all in the same risk class, not just F.

### 3.2 The dominant failure mode: 20 silent wrong arms

Every `platform.target()` branch in shared lowering is **binary** — `if macos { … } else
{ …POSIX… }` or `if linux { … } else { …macOS… }`. Not one has an `else if windows`.

**The moment `windows-x86_64` is registered in `NATIVE_BACKENDS` (47-A), all 20 become
reachable and every one silently resolves to a POSIX arm — with no compile error, no
diagnostic, and no failing test** (the Windows fixtures don't exist yet). A sample of
what Windows would inherit:

| Site | Windows silently gets |
|---|---|
| `fs_helpers_io.rs:2744` (`open_flag_set`) | **Darwin `O_*` bit values** (`write: 1537`, `append: 521`) |
| `fs_helpers_paths.rs:922`, `:1039` | the macOS `d_namlen` dirent shape |
| `term.rs:233`, `:316`, `:800` | `LINUX_TIOCGWINSZ` and an `ioctl` that does not exist |
| `runtime_helpers.rs:63`, `:612`, `:617` | bare `pthread_*` symbols |
| `os.rs:1116`, `:1334` | `_SC_NPROCESSORS_ONLN = 84`; the Linux `/proc` exe path |
| `datetime.rs:59` | `CLOCK_MONOTONIC_LINUX` for a `clock_gettime` Windows lacks |
| `mod.rs:688` | **ALSA sonames baked into `.rdata`** |
| `crypto_ec.rs:113`, `tls/openssl.rs` ×7 | the **OpenSSL** backend |
| `mod.rs:712` (`skip_entry_arena_destroy`) | `false` — an undeliberated answer about destroying the arena while workers live |

This is the same silent-wrong-value class 47-F correctly identifies for its timed-wait
polarity, replicated across C, D, E, G and H. It is the single largest risk in the
feature and the original plan does not name it.

**The fix is mechanical and belongs before registration:** convert those 20 binary
branches to an exhaustive `match` on a platform-family enum, so that adding Windows
produces **20 compile errors** instead of 20 silent wrong arms. That turns the dominant
silent-failure class into a build failure. It is inert (no behavior change, provable by
byte-identical goldens) and it blocks on nothing — see sub-plan **47-P**.

### 3.3 Ways out of the constant seam

Three ways out, to be decided before E/F/G start (§Open Decisions 1):

- **(a) Raise the seam.** Replace the constant accessors with intent-level methods
  (`emit_set_raw_mode`, `emit_read_dir_entry`) that each OS implements however it likes.
  Cleanest; largest diff; touches every existing backend.
- **(b) Branch in the consumer.** Add `if platform.has_termios()` forks in shared
  lowering. Smallest diff; leaves POSIX assumptions in shared code and grows a new fork
  every surface.
- **(c) Emulate.** Have Windows return synthetic offsets into a fake struct it then
  interprets. Rejected on sight — it encodes a lie in the platform seam.

Recommended **(a)**, scoped as its own sub-plan (**47-S**) landing *before* E/F/G,
because it edits shared code that every backend compiles and must stay byte-identical
for the four existing targets. That is new work the original letter map has no room for.

**Where correctness risk concentrates:** 47-A (ABI — shadow space, callee-saved xmm,
stack args; a mistake corrupts every non-trivial call), 47-B (PE layout — a wrong field
silently fails to load), and **47-S plus E/F/G (shared lowering — a mistake here is not
a Windows bug, it is a linux-aarch64 or riscv64 bug)**. The 2026-07-14 master named only
the first two.

**Where design uncertainty concentrates:** §3.1, and nowhere else. A and B are
mechanical against well-documented formats with toolchain oracles. §3.1 is a design
question with no precedent in this tree, and it should be settled by a cheap spike
(convert *one* surface — `term::` raw mode — end to end) before D–H are scheduled.

**Rejected alternatives.**
- *Link the MSVC CRT / `mainCRTStartup`.* Pulls in an external toolchain and a CRT
  dependency; raw-entry + kernel32-IAT mirrors the static-ELF path and keeps `mfb`
  self-contained (a hard non-goal).
- *Emit `syscall` to `ntdll` sysnums directly.* Windows syscall numbers are unstable
  across builds and unsupported — kernel32/IAT is the only sanctioned ABI.
- *A shared container trait unifying ELF/Mach-O/PE.* The two existing writers are
  deliberately parallel siblings; forcing a trait now is scope the target doesn't need.

## Feature map (the whole `47`)

**Letters are identifiers, not an order.** Execution is topological over the graph
below. Every letter is additionally gated behind §Prerequisites.

```
  BLOCKS ON NOTHING — land these first, all inert, all provable by 0-diff goldens:
    P  (exhaustive platform-family match — turns 20 silent arms into 20 compile errors)
    F1 (collapse pthread emission onto one sync_symbol chokepoint)
    G1 (collapse 32 net/ call sites onto one net_symbol chokepoint)
    E1 (collapse 6 termios call sites onto one term_symbol chokepoint)
    B1 (src/os/windows/ PE writer, standalone — touches zero shared code)

  A (Win64 ABI + register model + the 54-method stub wall)
        │
        ├──► B2 (wire the writer to the backend)
        ├──► F3 (spawn / release / timed wait — needs the shadow space + stack tail)
        │
        ▼
  C (Win32 runtime floor)  ── also edits entry_and_arena.rs + mod.rs
        │
        ├──► D (fs)          ├──► F4 (advertise thread.*)
        ├──► S (raise the constant seam)
        │         │
        │         ├──► E2 (term over Console API)
        │         └──► G2 (net over Winsock) ──► H (crypto + TLS)
```

Dependency list, in the form the executor checks:
`P, F1, G1, E1, B1 ← nothing`; `A ← P`; `B2 ← A + B1`; `F3 ← A + F1`; `C ← A + B2`;
`D ← C`; `S ← C`; `F4 ← C + F3`; `E2 ← S + E1`; `G2 ← S + G1`; `H ← G2`.

**The re-cut's central move:** five units block on nothing and are provable by
byte-identity alone. The original map had *everything* behind A, which put the whole
feature behind its riskiest phase. Landing P first is what makes the rest safe, because
after it the 20 silent arms are compile errors.

- **47-P — Exhaustive platform-family match.** §3.2. Convert the 20 binary
  `platform.target()` branches in shared lowering to an exhaustive `match` on a
  platform-family enum, so adding a new OS is a compile error at every site instead of a
  silent POSIX arm. Inert; zero behavior change; proven by 0-diff goldens on all four
  targets. **New; not in the 2026-07-14 map.** Depends on: nothing. **Land first.**
- **47-A — Win64 ABI + register model.** The `X86Abi` selector, the Win64 register
  model, the shadow space and outgoing stack-arg tail, target registration, and the
  `CodegenPlatform` stub wall (see below). Codegen-only, proven by selection/encoder
  unit tests. *Declared `large` — must be split (§Corrections).* Depends on: P.
  - **Undeclared cost the draft hid:** 47-A Phase 4 says `win_x86_64/code.rs` is "a
    minimal `CodegenPlatform` … leave the trait's own defaults." **That does not
    compile.** Only 11 of 65 methods have defaults; **54 are required.** A must author
    ~51 stubs, and the 8 `termios_*` plus the offset/constant accessors return plain
    `usize`/`u64` — they cannot even carry an "unimplemented" error, so they must return
    fabricated values. Give the stub wall its own phase; it is real work currently
    hidden inside A and silently moved out of D/E/G.
  - Note `external_int_argument_registers` (added by bug-296 on 2026-07-18, overridden
    to 6 at `x86_64/regmodel.rs:159`, refusal at `link_thunk.rs:661-672`) is the
    existing mechanism for expressing Win64's 4-register external cap. The master's §4
    predates it and discusses `REGISTER_ARGUMENT_COUNT` instead.
- **47-B — PE/COFF executable writer.** `src/os/windows/{mod,object,link/}`; a minimal
  PE32+ image Windows runs. **Splits in two:** *B1* (the writer itself — Phases 1–3,
  touching only the new `src/os/windows/` leaf and one line of `src/os/mod.rs`) depends
  on **nothing** and is the only unit in the feature that touches zero shared code;
  *B2* (wire `write_executable` to the backend) depends on A. The draft declared the
  whole of B behind A.
  - **A↔B conflict to settle once:** B Phase 3 adds a test pinning the external-call
    bytes `[0xB8,8,0,0,0,0xE8,…]` (`emitter.rs:710`). That `B8 08` is `mov eax,8`, the
    **SysV variadic vector-count marker** — meaningless on Win64, and A's proposed
    `CALL_ARGS_WIN64` makes `rax` an argument slot. The `internal` guard at `:706` saves
    it today, but A introducing `X86Abi` is the natural moment to drop the marker for
    Win64 — which B's new test would forbid. Decide in whichever lands first.
- **47-C — Windows console runtime floor.** The Win32 machine floor: entry
  (`GetCommandLineW` + `CommandLineToArgvW`), arena (`VirtualAlloc`/`VirtualFree`),
  stdout/stderr (`GetStdHandle` + `WriteFile`), exit (`ExitProcess`), RNG seed
  (`BCryptGenRandom`). `hello.exe` prints and exits. Depends on: A, B.
- **47-S — Raise the platform seam off POSIX.** §3.1. Replaces the 21 POSIX constant
  accessors with intent-level methods across all existing backends, byte-identically.
  **New; not in the 2026-07-14 map.** Depends on: C (needs one real Windows surface to
  validate the new seam against). **Blocks E, F, G.**
- **47-D — Filesystem surface.** `fs::*` over Win32 + UTF-8↔UTF-16 path marshaling.
  Depends on: C. *(Consumes `dirent_*`/`stat_mode_offset`, so it is affected by S — see
  §Open Decisions 2.)*
- **47-E — Console/terminal surface.** `term::*` + `io::` terminal queries over the
  Console API. **Splits:** *E1* (collapse the 6 termios call sites onto one
  `term_symbol` chokepoint — inert) depends on **nothing**; *E2* (Console API arms,
  `GetConsoleMode`/`SetConsoleMode`, and the three `TIOCGWINSZ` branches at
  `term.rs:233/316/800`) depends on S + E1.
- **47-F — Threads.** `thread::*` over `CreateThread`/`WaitForSingleObject` +
  `SRWLOCK`/`CONDITION_VARIABLE`. **Its four phases have four different dependencies**,
  which is why the single header `Depends on: plan-47-C` was wrong:
  | Phase | Real dependency |
  |---|---|
  | F1 — collapse 3 emission routes onto one `sync_symbol` | **nothing** — inert refactor, lands today |
  | F2 — rename-compatible Win32 arms + init-check gating | **nothing** — needs only a target string |
  | F3 — spawn / release / timed wait | **47-A** (shadow space + outgoing tail; `CreateThread` takes six args, two on the stack) |
  | F4 — advertise `thread.*`, kernel32 imports, fixtures | **47-C** |
  *Declared `large` — this split is the required one (§Corrections).*
- **47-G — Networking.** `net::*` over Winsock2. **Same shape as F, at 38% the scale:**
  32 hardcoded POSIX symbol literals across `net/mod.rs` (14), `net/io.rs` (16),
  `net/poll.rs` (2), plus `WSAGetLastError` diverging from `errno`. **Splits:** *G1*
  (collapse onto one `net_symbol` chokepoint — inert) depends on **nothing**; *G2*
  (Winsock arms: `closesocket`, `ioctlsocket`, `WSAPoll`, `WSAStartup`) depends on
  S + G1. The draft declared G as "just add methods" — there are **no** `emit_socket` /
  `emit_connect` methods on the trait at all; the net surface is constants only.
- **47-H — Crypto + TLS transport.** `crypto::*` over CNG/BCrypt and a `tls::*` Schannel
  backend. Depends on: G2. **Not a "third sibling" as the draft says:**
  `crypto_ec.rs:113` is `if target.contains("macos") { macos } else { openssl }`, so
  Windows falls into the **OpenSSL** arm; `tls/openssl.rs` is the *default* backend with
  seven internal macOS branches (`:15,924,1453,1814,2069,2215,2380`), not the Linux one.
  Adding Schannel means editing that dispatch and the data-object emission at
  `mod.rs:680`/`:703` — otherwise Windows bakes OpenSSL sonames into its `.rdata`.

Bounded-surface evidence: the machine floor (47-C) needs **seven** kernel32 imports —
`GetStdHandle`, `WriteFile`, `VirtualAlloc`, `VirtualFree`, `ExitProcess`,
`GetCommandLineW`, `GetSystemTimePreciseAsFileTime` — plus `CommandLineToArgvW`
(shell32) and `BCryptGenRandom` (bcrypt): **9 total**, a tiny fixed IAT. (The
2026-07-14 draft said "six" in its overview while its own Phase C required the
time import — an internal contradiction.) Each later surface adds one DLL's worth of
imports on the same mechanism.

## 1. Goal

- A `windows-x86_64` `BuildTarget` selectable via `mfb build -target windows-x86_64`
  that emits a PE32+ (`PE\0\0`, machine `0x8664`, subsystem `CONSOLE`) `.exe`.
- The `.exe` runs on 64-bit Windows and, for any program using only the surfaces whose
  sub-plans have landed, produces byte-identical stdout/stderr and the same exit code as
  the Linux x86-64 build of the same program.
- All OS access is via DLL imports through an Import Address Table — no `syscall`
  instruction is emitted for the Windows target.
- The x86-64 instruction *bytes* (`src/arch/x86_64/encode/`) are reused unchanged; only
  ABI realization and the container/OS layers are new.

### Non-goals (explicit constraints)

- **No change to any existing target's output bytes.** macOS-aarch64, linux-aarch64,
  linux-x86_64, linux-riscv64 must stay byte-identical. Guarded by golden diff — **but
  see §Prerequisites row 3: that guard does not currently exist for riscv64.**
- **No language-surface, value/copy/move/freeze, layout, or IR change.**
- **No external assembler, linker, or MSVC/CRT toolchain in the shipped `mfb`.**
  `link.exe`/`clang`/`dumpbin` are development-time oracles only, never invoked by a
  build.
- **No GUI/app mode** for Windows — console subsystem only; `supports_app_mode()`
  returns `false`. This is why the 8 app-mode trait methods (§2.1) need a uniform
  unreachable answer rather than an implementation.
- **No cross-run of the `.exe` from the build host.** Validation happens on the Win11
  box (ssh port 2230) or under Wine in CI.
- **No dual-flavor split.** Windows emits a single `.exe`; no `Flavor::ALL` loop.

## 4. Windows x64 ABI (47-A detail)

- Argument registers: `rcx, rdx, r8, r9`; FP args `xmm0–xmm3` (positionally aliased with
  the int slot — arg *n* is either `rN` or `xmmN`, never both). Return `rax` / `xmm0`.
- 32-byte shadow space reserved by the *caller* for the callee to spill its 4 register
  args; `rsp` 16-byte aligned at the `call`.
- Stack arguments: args ≥ 5 go *above* the shadow space. Requires realizing the x86
  outgoing/incoming stack-arg tail (`shared/abi.rs:16-24` currently errors past the cap).
- Callee-saved: `rbx, rbp, rdi, rsi, r12–r15, xmm6–xmm15`. Note `rsi`/`rdi` are
  callee-saved on Win64 but caller-saved on SysV, and `xmm6–15` are callee-saved on
  Win64 (none are under SysV) — both the register model's split and the prologue save
  set change.
- Pinned registers keep their physical assignment where free (`r15` = arena base, the
  zero register, `%thread`/closure homes) — verify none collide with a Win64 arg or the
  shadow space.

## 5. Import Address Table & relocations (47-B detail)

- Group `EncodedImage.imports` by DLL. Build `.idata`: an import-directory table, import
  lookup + address tables, and the hint/name table.
- External calls: `emit_libc_call` produces `bl symbol` + an external `RelocIntent::Call`.
  The PE patcher resolves it to `call [rip+disp32]` targeting the IAT slot — an
  *indirect* call, not a direct `call rel32`. **Confirm the x86 encoder can emit the
  indirect form for an external `Call`**; if it currently selects `call rel32`, add an
  indirect form gated on the import being a function (PE has no PLT stub layer).
- `environ` has no Windows analog — use `GetEnvironmentStringsW`.
- `.reloc`: omit initially (fixed `ImageBase`, no `DYNAMICBASE`); optionally add later.

## Compatibility / Format Impact

- **New:** a `windows-x86_64` target; a `container:"pe"` object plan; `<name>.exe`
  artifact naming; new import libraries in plan tables; an internal `X86Abi` parameter on
  the x86 selection path.
- **Changed (47-S):** the `CodegenPlatform` trait loses 21 POSIX constant accessors and
  gains intent-level methods; every existing backend is updated. Emitted bytes for all
  four existing targets must be **identical** — the byte-identity guard's hardest test in
  this plan, and the reason §Prerequisites row 3 exists.
- **Unchanged:** NIR / native plan / MIR JSON schemas; the `EncodedImage` type; the
  language, builtins, resolver, and IR.

## Validation Plan

- Tests: per sub-plan. A and B are unit-testable on any host; C–H need the Win11 box.
- Coverage check: **before E/F/G, confirm the byte-identity guard actually covers the
  backends they can break.** Today `linux-riscv64` has 0 native goldens (§2.1). A green
  `scripts/artifact-gate.sh` means "nothing covered changed", not "nothing changed".
- Runtime proof: `hello.exe` on the Win11 box (ssh port 2230) printing byte-identical
  stdout and exiting 0, then one program per landed surface.
- Doc sync: `src/docs/spec/linker/**`, `mfb spec memory 08_program-startup` and
  `06_native-calling-convention`.
- Acceptance: the project's full suite, plus `scripts/artifact-gate.sh` showing every
  existing target byte-identical.

## Open Decisions

1. **How to de-POSIX the platform seam** (§3.1) — **must be settled before E/F/G, and it
   creates the new sub-plan 47-S.** Recommended **(a) raise the seam** to intent-level
   methods. Alternatives: (b) branch in the shared consumer; (c) emulate POSIX offsets
   (rejected on sight). Settle it with a one-surface spike (`term::` raw mode) rather
   than by argument.
2. **Whether 47-D lands before or after 47-S.** D consumes `dirent_*`/`stat_mode_offset`,
   so the seam change affects it. Recommended: land D *after* S, so it is written once
   against the raised seam. If D lands first, budget a rewrite.
3. **Whether the 8 app-mode methods get a shared no-app default** on the trait rather
   than 8 stubs in the Windows impl. Recommended: add defaults returning "unsupported",
   so every future console-only OS gets them free. Touches shared code, so it belongs
   with 47-S.

## Corrections

<!-- Filled in during execution. -->

- 2026-07-20 — **"70 methods" was wrong: the trait has 65**, the Linux impl 64, the
  macOS impl 63 (§2.1). More usefully, the draft never split them by kind — 8 are
  app-mode (an explicit non-goal) and 21 are POSIX constants (§3.1).
- 2026-07-20 — **"The traits gain no new required methods" is false**, and it was the
  claim hiding this plan's biggest design problem. §3.1 and sub-plan 47-S are new.
- 2026-07-20 — **"47-F is the only sub-plan that edits shared lowering" is false.**
  E consumes `termios_*` at `shared/code/io_helpers.rs:866`; G consumes socket constants
  in `shared/code/net/` and `tls/`. The risk model named only A and B; it now names the
  shared-lowering group as a third, equal risk.
- 2026-07-20 — **47-F's declared dependency contradicted its own body.** Header said
  `Depends on: plan-47-C`; `plan-47-F:429` says "If 47-A has not landed that, 47-F is
  blocked — this is the concrete dependency." The graph now shows F depending on A
  **and** C.
- 2026-07-20 — **47-A and 47-F are declared `large`**, above the sub-plan band; the
  split rule says large plans are split into small/medium sub-plans before starting.
  Both need splitting. F's own header admits it ("the four phases below are individually
  medium and land separately").
- 2026-07-20 — **C, D, E, G, H were never written.** The 2026-07-14 master said each
  ships "when broken out"; five of eight never were. Every sub-plan is now required up
  front.
- 2026-07-20 — **The byte-identity guard is thinner than the non-goal assumes**: 28 of
  1021 golden dirs carry native artifacts and `linux-riscv64` has zero. Recorded as
  §Prerequisites row 3. *(An earlier pass of this review counted only `.ncode` and
  concluded Linux had no coverage at all; that was wrong — `.ncodesum` goldens for
  linux-x86_64 and linux-aarch64 landed 2026-07-20 in `ff163ddeb`. Checked before acting
  on it.)*
- 2026-07-20 — **The dominant failure mode was unnamed: 20 silent wrong arms** (§3.2).
  Every `platform.target()` branch in shared lowering is binary; registering
  `windows-x86_64` makes all 20 resolve to a POSIX arm with no compile error and no
  failing test. Windows would inherit Darwin `O_*` bits, `LINUX_TIOCGWINSZ`, bare
  `pthread_*`, `clock_gettime`, ALSA sonames in `.rdata`, and the OpenSSL crypto
  backend. New sub-plan **47-P** converts these to an exhaustive match so they become
  compile errors. This is a larger finding than the constant seam and subsumes it.
- 2026-07-20 — **The POSIX coupling is four categories, not one** (§3.1). The trait
  constants (21) are the only one the trait can reach. The bigger one is ~125 hardcoded
  POSIX **symbol literals** passed to `emit_libc_call` — the trait changes how a symbol
  is called, never which symbol. Sockets alone are 32 sites.
- 2026-07-20 — **"47-G just adds methods" is false.** There is no `emit_socket` /
  `emit_connect` on the trait; the net surface is *constants only*, and all 32 socket
  calls are string literals in shared code. G is the same kind of work as F at 38% the
  scale — the draft classed it with D.
- 2026-07-20 — **B does not depend on A** (mostly). B Phases 1–3 create
  `src/os/windows/` and touch one line of `src/os/mod.rs`; `object.rs::validate`
  compares the target as a *string*, not against the registry. Only B Phase 4 needs A.
  B1 is the one unit in the feature touching zero shared code.
- 2026-07-20 — **F's four phases have four different dependencies** (nothing / nothing /
  A / C), not the single `Depends on: plan-47-C` in its header. F1+F2 block on nothing
  and are the biggest available de-risking move.
- 2026-07-20 — **47-A's "minimal `CodegenPlatform`" does not compile.** Only 11 of 65
  methods have defaults; 54 are required. A must author ~51 stubs, and the `termios_*` /
  offset accessors return plain `usize` so they cannot even carry an error — they must
  return fabricated values. Real work hidden inside A.
- 2026-07-20 — **`external_int_argument_registers` exists** (bug-296, 2026-07-18;
  `x86_64/regmodel.rs:159`, refusal at `link_thunk.rs:661-672`) and is the actual
  mechanism for Win64's 4-register external cap. The master predates it and reasons
  about `REGISTER_ARGUMENT_COUNT` instead.
- 2026-07-20 — **The plan-57 conflict is a baseline conflict, not a file conflict.**
  An earlier pass of this rewrite recorded plan-57 as holding the same files —
  **that was wrong**; plan-57 edits `builder_collection_*.rs` and `link_thunk.rs`, which
  no plan-47 phase touches. The real constraint is not straddling the `MFB_KIND2` flip
  with a zero-diff phase. Prerequisite row 4 corrected.
- 2026-07-20 — **47-A's Validation Plan is stale in the good direction.** It prescribes
  a manual cross-target `-ncode -nobj` `cmp` workaround because "the gate diffs the host
  target only". `ff163ddeb` (2026-07-20) made the gate multi-target; delete the
  workaround and rely on the gate.
- 2026-07-20 — Line citations corrected: `CodegenPlatform` is `types.rs:212` (draft said
  `:204`); `NativePlanPlatform` is `plan/mod.rs:154` (`:152`); `NATIVE_BACKENDS` is
  `target.rs:197` (`:161`); the `"windows"`-rejection negative test is
  `src/os/linux/mod.rs:143` (`:87`) **and must stay** — the draft's Phase B told you to
  drop it; `cli/build.rs` app-mode rejection `:270` (`:239`) and cross-target path
  `:493` (`:498`); `runtime_helpers.rs` thread trampoline is `:62`/`:70`/`:383`/`:723`
  (`:600`). Also: `CodegenPlatform` is implemented at `linux_common/code.rs:302` for all
  three Linux arches, not by `linux_x86_64` — Windows mirrors **macOS's** standalone
  shape.
- 2026-07-20 — **The master never mentions LINK / `link_thunk.rs` / CSTRUCT** (zero
  grep hits). That file grew **+94%** since the master was written (plan-50 landed
  struct-by-pointer marshaling), and a native `LINK` call is now a first-class Windows
  ABI surface. It needs a place in the phase map.
- 2026-07-20 — **The required-method count was settled by direct measurement after two
  review passes disagreed** (one said 25 required, one said 54). Definitive:
  `types.rs:212-630`, **65 total, 54 required, 11 defaulted**. The 11 defaults are the 8
  app-mode methods plus `emit_tls_block_trampolines`, `entry_args_in_registers` and
  `libc` — so the console-only non-goal costs Windows nothing; it inherits all 8 free.
  Recorded here because the disagreement is the reason to trust the command in §2.1 over
  any prose number, including this document's.
- 2026-07-20 — **`mir::Backend` has 4 methods, not "exactly three interesting" ones**
  (47-A:171). `is_aarch64()` (`mir.rs:541`) is an existing ISA-dispatch hook the plan
  never mentions, and it is exactly the kind of seam a new backend must answer.
- 2026-07-20 — **`REGISTER_ARGUMENT_COUNT` is read at 7 shared-lowering sites, not 3**
  (`builder_emit_helpers.rs:81,87,91`; `function_lowering.rs:577,580,661,674`). 47-A's
  argument for *not* making it backend-dependent understates the blast radius it is
  arguing about.
- 2026-07-20 — **47-A's "~90% of `select_x86` is ISA, not ABI" is likely inverted.**
  Measured: `select.rs` non-test is 780 lines, of which `remap_x86_abi` alone
  (`:107-621`) is 515 — roughly **71% ABI-specific**. Re-derive before using it to argue
  the Win64 delta is small.
- 2026-07-20 — **`src/os/{linux,macos}/mod.rs` expose 6 and 4 public fns, not "the same
  three wrappers"** (47-B:112). The three named do exist in both, but a PE sibling
  should be scoped against the real surface.

## Summary

The engineering risk is in three places, not the two the original named: the Win64 ABI
(47-A), the PE image layout (47-B), and **the POSIX-shaped shared layer (§3.1–3.2)**.
The third is the one with no precedent, no plan, and the nastiest failure mode — 20
binary branches that hand Windows a POSIX arm with no compile error the instant the
target is registered. It gets two new sub-plans: **47-P** (make them exhaustive, so they
become compile errors) and **47-S** (raise the constant seam).

The re-cut's payoff: **five units block on nothing** — P, F1, G1, E1, B1 — all inert,
all provable by byte-identical goldens alone. The original map put every one of them
behind A, the riskiest phase in the feature. Landing P first is what makes the rest
safe.

One prerequisite is unmet: `linux-riscv64` has no byte-identity goldens, and P, E, F and
G can all break it. That does not block A or B, but it blocks P — which is the thing
that should land first, so seed those goldens now.

What is left untouched: the language, IR, NIR/plan/MIR schemas, `EncodedImage`, the
x86-64 instruction encoder, and every existing target's emitted bytes.
