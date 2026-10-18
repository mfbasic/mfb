# Design Work-Pad — Bare-metal targets and the `MEMORY` block

> **Status:** scratch. This is a design work-pad, *not* a plan. It captures the
> current shape of the idea, the decisions we've settled, and the open questions.
> Nothing here is committed to implement yet.

## Contents

1. Goal and scope
2. Who owns what
3. Targets and build output
4. `project.json`: the `baremetal` section
5. Startup and shutdown
6. Functions the program must supply
7. Standard library on bare metal
8. Other features not available on bare metal
9. The `MEMORY` block
10. Compiler touch points
11. Related bugs
12. Open questions
13. Reference points

---

## 1. Goal and scope

Add three targets that run with no operating system:

- `--target baremetal-x86_64`
- `--target baremetal-aarch64`
- `--target baremetal-riscv64`

A bare-metal program is the only software on the chip. The compiler emits the startup
code, the runtime and the program in one image. The program reaches its hardware
through `MEMORY` blocks (§9), and supplies a small set of functions the runtime calls
where it would otherwise call the OS (§6).

There is no board concept. The compiler keeps no database of boards, chips or devices.
Everything chip-specific comes from the project (§4) or from the program's own code (§6,
§9).

`MEMORY` exists **only** on bare-metal targets. There is no `--mem` switch and no
user-space `/dev/mem` path on Linux.

## 2. Who owns what

The line is set by what MFBASIC code can reach. A `MEMORY` block reaches memory and
memory-mapped device registers (`OVERLAY`, `VIEW`, `PEEK`, `POKE`), but has no way to read or
write **system registers** (FPU enable, page tables, vector base, privilege state).

| Owner | Responsibility |
|---|---|
| Compiler, from `project.json` | Image format and load address; FP/SIMD (and RISC-V vector) enable; page tables; the vector table and its entry wrappers; parking every core except the boot core; stack placement and the stack-overflow limit |
| Program, through §6 functions | Console output, exit, memory for the runtime's heap, the random seed, vector handlers |
| Program, through `MEMORY` blocks | Every device: UART, timers, framebuffers, GPIO, interrupt controllers, … |
| Compiler built-ins inside `MEMORY` | The few system-register operations a program needs: interrupts on/off, GICv3 claim/complete (§9 #36) |

## 3. Targets and build output

`BuildTarget::parse` splits on the first `-` (`src/target.rs`), so `baremetal-<arch>` parses
as os `baremetal` and an existing arch with no change.

Measured on today's Linux output (`objdump -p` on a `FUNC main AS Integer` program built
with `--target linux-<arch>`): every executable has `PT_INTERP` and `DT_NEEDED libc.so.6`,
and each build is produced twice (glibc and musl, `LinuxFlavor` in
`src/os/linux/link/flavor.rs`). None of that applies on bare metal.

Bare-metal output:

- One static image, no interpreter, no dynamic section, no PLT/GOT/RELRO, no symbol
  versioning, no libc flavor axis.
- No AppImage/AppDir, icons, code signing, vendor-library copies or RPATH.
- Every relocation the code generator emits is PC-relative and `EncodedImage.initializers`
  is always empty, so the image runs at whatever address it is loaded. Only the image header
  and `loadAddress` (§4) depend on placement. `adrp` needs 4 KiB alignment of the image.
- The existing static ELF writer (`encode_static_elf` / `encode_static_elf_x86` in
  `src/os/linux/link/elf.rs`) is the starting point. Today it is reached only when
  `image.imports.is_empty()` (`encode_executable_bytes`, `src/os/linux/link/mod.rs`), which no
  program with an entry point satisfies.

Formats (`format`, §4): `elf`, `bin` (flat raw image), and `multiboot2` (x86-64 only).

## 4. `project.json`: the `baremetal` section

The section holds chip-level facts the startup code needs before any MFBASIC code runs.

```json
"baremetal": {
  "format": "elf",
  "loadAddress": "0x40080000",
  "stackSize": "64K",
  "bootCore": 0,
  "mmap": {
    "lowram":  { "kind": "ram",    "base": "0x40000000", "size": "0x4000000" },
    "highram": { "kind": "ram",    "base": "0x48000000", "size": "0x4000000" },
    "uart":    { "kind": "device", "base": "0x09000000", "size": "0x1000" },
    "disp":    { "kind": "device", "base": "0x09010000", "size": "0x1000" },
    "vram":    { "kind": "device", "base": "0x3C000000", "size": "0x1000000" }
  },
  "vectors": {
    "irq":  "onIrq",
    "sync": "onFault"
  },
  "rv64": { "privilege": "s" }
}
```

(The example mixes targets: `vectors` names are aarch64's, `rv64` is riscv64-only.)

| Property | Targets | Why |
|---|---|---|
| `format` | all | `elf`, `bin`, `multiboot2` (x86-64). The boot ROM or loader decides what it accepts. |
| `loadAddress` | all | Where the image is loaded and entered. Sets only the image header and file start (code is PC-relative, §3). |
| `stackSize` | all | Replaces the `prlimit64(RLIMIT_STACK)` read at startup. The software stack-overflow check (`src/codegen/runtime/stack_limit.rs`) uses it. |
| `bootCore` | aarch64; riscv64 when `privilege` is `m` | The one core that runs `main`. The startup code parks every other core. |
| `mmap` | all | Named memory regions. Drives the page tables and names every `OVERLAY`/`VIEW` target (§9). |
| `vectors` | all | Optional. Maps an architecture vector name to the MFBASIC function that handles it (§6 "Vector handlers"). |
| `rv64.privilege` | riscv64 only | `m` or `s`. Required on riscv64, an error on other targets. |

### Value rules

- Addresses and sizes are **strings**: hex (`"0x09000000"`) or a size with a suffix
  (`"64K"`, `"16M"`). JSON numbers are usually parsed as doubles and lose precision above 2^53.
- **Unknown keys are errors.** (`options` ignores unknown keys; that is wrong here, because a
  misspelled key would build and then hang with no output.)
- The section is required on a `baremetal-*` target and an error on any other target.

### `mmap`

Each entry is `name: { kind, base, size }`. `kind` is `ram` or `device`.

- `ram` is mapped as normal, cacheable memory; `device` as device (uncached, ordered) memory.
- RAM may be split into any number of regions.
- The `ram` region that contains `loadAddress` is **compiler-owned**: it holds the image and,
  at its top, the stack. Every other `ram` region is **program-owned**; the program hands it
  to the runtime through `bmGetMem` (§6) or uses it through `OVERLAY`/`VIEW`.
- Names are identifiers. They are the only way to name an address in source (§9).

Checks:

- No two regions overlap.
- At least one `ram` region exists.
- `loadAddress` lies inside a `ram` region, and the image plus `stackSize` fits in it.
- `base` and `size` are 4 KiB multiples (the page-table granule).

### Why there is no entry-mode setting

The startup code works out the CPU state itself where that is reliable:

- **aarch64:** `CurrentEL` is readable at EL1, EL2 and EL3. The startup code reads it and
  writes the FPU-enable, MMU and vector-base registers for that level (`CPACR_EL1.FPEN`,
  `CPTR_EL2`, `CPTR_EL3`, `SCTLR_ELx`, `TTBR0_ELx`, `VBAR_ELx`).
- **x86-64:** the boot protocol fixes the entry state, and the protocol follows from
  `format`. `multiboot2` always enters in 32-bit protected mode, so the startup code builds
  page tables, enters long mode and enables SSE (`CR0`/`CR4.OSFXSR`).
- **riscv64:** FP and vector enable work in both modes through `sstatus.FS`/`sstatus.VS`.
  But core parking and trap vectors differ and the mode can't be detected reliably:
  - M-mode: every hart starts; only M-mode can read `mhartid`; traps go to `mtvec`.
  - S-mode under SBI firmware: only the boot hart starts and its id arrives in `a0`; traps
    go to `stvec`.
  - Reading an M-mode register from S-mode traps into the firmware, and whether the firmware
    hands the trap back depends on the firmware.

  So riscv64 alone needs `rv64.privilege`. With `"s"`, `bootCore` does not apply.

### RISC-V vector: always on, no setting

`baremetal-riscv64` requires RV64GC **plus V** (RVV 1.0). There is no feature setting: the
code generator decides which instructions it emits, and with no `ASM` a program can't add
others, so the only optional extension in play is V.

Today riscv64 is the only target that detects a CPU feature at run time: the startup code
walks `envp` to the auxiliary vector and reads `AT_HWCAP` to set `_mfb_rt_has_rvv` (seen in
`program.entry` of a `--ncode --target linux-riscv64` build), and every vector sequence is
compiled twice, an RVV arm and a scalar arm behind that flag (`lower_v128_run`,
`src/arch/riscv64/v128.rs`). On bare metal there is no auxiliary vector, and the answer is
fixed: the flag is 1, and the scalar arms need not be emitted at all.

- RVV 1.0 implies `VLEN ≥ 128` (`Zvl128b`), which the 128-bit `vsetivli` sequences need.
- A chip without RVV 1.0 is not supported: the first vector instruction raises an
  illegal-instruction exception, reported through `bmStderr` (§5 step 5). From memory, not
  checked: the SiFive U74 (VisionFive 2) has no V, and the T-Head C906/C910 have only the
  incompatible draft 0.7.1.
- In M-mode the startup code can check `misa.V` and fail with a clear report before `main`.
  S-mode has no reliable check, so it trusts the chip.

Unverified: whether x86-64 or aarch64 code generation assumes any optional ISA extension
(guess: no; x86-64 uses SSE2, aarch64 uses NEON, both baseline).

## 5. Startup and shutdown

### Today's Linux startup (measured)

From `program.entry` in a `--ncode --target linux-riscv64` build of a `FUNC main AS Integer`
program:

1. Walk `envp` to the auxiliary vector, read `AT_HWCAP`, set `_mfb_rt_has_rvv` (riscv64 only).
2. Zero the arena state (`_mfb_rt_main_arena`, in `.data`).
3. `prlimit64(RLIMIT_STACK)` → the arena's stack-limit words.
4. `fcntl(F_GETFD)` on fds 0, 1, 2; reopen a closed one on `/dev/null`; `abort` on failure.
5. `signal(SIGINT)`, `signal(SIGTERM)` → `_mfb_rt_signal_handler`; `signal(SIGPIPE, SIG_IGN)`.
6. `clock_gettime` → `ARENA_START_TIME_OFFSET`.
7. Seed the RNG if the program uses `math::rand`/`math::seed` (§6, bug-1096).
8. Call `main`; report an error to fd 2 or range-check the exit code.
9. `_mfb_shutdown` → `_mfb_arena_destroy` (`munmap` each block) → `_exit`.

The minimal libc import set of that program (`.nobj` import list): x86-64 `abort`,
`clock_gettime`, `fcntl`, `open`, `signal`; aarch64 and riscv64 add `_exit` and `write`.

### Bare-metal startup

1. Stop every core except the boot core (aarch64 `MPIDR_EL1`; riscv64 M-mode `mhartid`; x86
   application processors are not started by the firmware).
2. x86-64 `multiboot2`: build page tables, enter long mode.
3. Enable FP/SIMD, and on riscv64 the vector unit (`sstatus.VS`; M-mode also checks
   `misa.V`, §4). Code
   generation uses FP/SIMD unconditionally (`Float` lives in `d`/`xmm` registers; SIMD list
   helpers use NEON/SSE2/RVV), so this must come first.
4. Build identity page tables from `mmap` and turn the MMU and caches on (aarch64, x86-64).
   riscv64 runs with translation off (`satp` = 0); cacheability is fixed by the chip.
5. Install the vector table (`VBAR_ELx`; `mtvec`/`stvec`; IDT, plus on x86-64 a GDT and a TSS
   whose IST stack serves `doubleFault`). Every slot gets a compiler-generated wrapper; slots
   named in `vectors` call their handler (§6), the rest are fatal: reported through
   `bmStderr`, ending in `bmExit`. Enable the per-source bits the hardware keeps in system
   registers for every interrupt slot named in `vectors` (riscv64 `mie`/`sie` for `software`,
   `timer`, `external`). On aarch64 with `irq` or `fiq` named, if `ID_AA64PFR0_EL1.GIC` reports
   the GICv3 system-register interface, initialize it (`ICC_SRE_ELx`, `ICC_PMR_EL1`,
   `ICC_IGRPEN1_EL1`). Interrupts stay masked until the program runs `INTERRUPTS ON` (#36).
6. Set the stack pointer to the top of the compiler-owned `ram` region; set the stack limit
   from `stackSize`. Zero `.bss`.
7. riscv64: `_mfb_rt_has_rvv` is fixed at 1 (or the guard and scalar arms are not emitted).
8. Zero the arena state. The first arena block comes from `bmGetMem` on first allocation.
9. Seed the RNG through `bmSeed` if the program calls `math::rand`.
10. Call `main` (the no-argument form only; see §8).
11. Shutdown: `_mfb_arena_destroy` returns each block through `bmRetMem`, then `bmExit(code)`.
    If `bmExit` returns, the core parks (`wfi`/`hlt` loop).

Dropped from today's sequence:

- Step 3 (`prlimit64`): replaced by `stackSize`.
- Steps 4–5 (fd checks, signals): no files, no signals. `_mfb_rt_signal_handler` is not
  emitted. The `SIGPIPE`/`EPIPE` path in `syscall_io.rs` (`emit_sigpipe_restore_and_raise`) goes
  with it.
- Step 6 (`clock_gettime`): the only reader of `ARENA_START_TIME_OFFSET` is the debugger
  (`grep -rn ARENA_START_TIME_OFFSET src` → `src/debugger/runtime.rs`), which does not exist on
  bare metal.

### Stack and interrupts

Handlers run on the interrupted code's stack (x86-64 `doubleFault` excepted, on its IST
stack), so `stackSize` must cover the deepest handler on top of the deepest program path.

x86-64 red zone: an interrupt pushes onto the stack at `rsp`, overwriting the 128 bytes below
it, so bare-metal x86-64 code must not use the red zone. One sample (`linux-x86_64` hello
`.ncode`) showed 0 negative `rsp` offsets among 549 `rsp`-based accesses; a sample is not
proof, and spill code needs an audit before interrupts are enabled on x86-64.

## 6. Functions the program must supply

The runtime calls these by fixed name and signature. On a `baremetal-*` target a missing
function, or one with the wrong signature, is a compile error naming it. On other targets the
names have no meaning.

| Function | Replaces | Required |
|---|---|---|
| `bmStdout(text AS String) AS Nothing` | `write` to fd 1 | always |
| `bmStderr(text AS String) AS Nothing` | `write` to fd 2: the runtime error report, out of memory, stack overflow, CPU exceptions | always |
| `bmExit(code AS Integer) AS Nothing` | `_exit`, `abort` | always |
| `bmGetMem(size AS Integer) AS Integer` | `mmap` in `_mfb_arena_alloc` (`emit_arena_map`) | always |
| `bmRetMem(startAddr AS Integer, size AS Integer) AS Nothing` | `munmap` in `_mfb_arena_destroy` (`emit_arena_unmap`) | always |
| `bmSeed() AS Integer` | `getentropy` (aarch64, riscv64) / raw `getrandom` (x86-64) for the RNG seed | only when the program calls `math::rand` |

### `bmStdout`, `bmStderr`

- `bmStderr` runs while the runtime is failing: the heap may be full and the stack close to
  its limit. It must not allocate and must not fail. If it fails, the runtime goes straight
  to `bmExit`.
- Problem: any loop over a String's bytes today allocates (`strings::toBytes` returns a
  `List OF Byte`). See §12.

### `bmExit`

Called after `main` returns (with its result) and after a fatal error (with 255). If it
returns, the boot core parks.

### `bmGetMem`, `bmRetMem`

The program owns memory management. The runtime only asks for blocks and gives them back.

Contract, from `src/codegen/memory/arena/arena.rs`:

- **Request size.** `_mfb_arena_alloc` asks for the larger of the request plus its 32-byte
  header (`ARENA_BLOCK_HEADER_SIZE`) plus growth slack, and `ARENA_DEFAULT_BLOCK_SIZE` (4096),
  rounded up to a multiple of 4096. `size` is always a multiple of 4096 and at least 4096.
- **Result.** The address of at least `size` bytes of `ram`-kind memory, or `0` for out of
  memory, which the runtime raises as `ErrOutOfMemory`. Today the arena treats only a
  negative result as failure (`branch_ge` after `emit_arena_map`), so the bare-metal path
  must test for `0` instead.
- **Alignment.** At least 16; 4096 matches what `mmap` gives today. Which one the arena
  actually needs is unverified (§12).
- **Return.** `bmRetMem` is called only by `_mfb_arena_destroy`, once per block, with the
  block's own address and size read from its header: always exactly a block `bmGetMem`
  returned, never part of one. Its callers are shutdown and thread teardown
  (`grep -rn ARENA_DESTROY_SYMBOL src`); with no threads on bare metal it runs only at
  shutdown, before `bmExit`.
- The memory must be `ram` kind. Arena blocks on device memory would break unaligned and
  exclusive accesses.

Constraints:

- Both run **inside** the allocator, so nothing they call may allocate (it would re-enter
  `_mfb_arena_alloc`). This needs a compile-time check of their call graphs (§12).
- The first `bmGetMem` call happens before the arena exists, so before any global: globals
  live in arena slots (`AbiCtx::arena_global_slots`). The allocator's own bookkeeping (bump
  pointer, free list) must live in program-owned `ram` reached through `OVERLAY`. But an
  overlay's descriptor is allocated normally (§9 decision 1), which would also re-enter the
  allocator (§12).

### `bmSeed`

- Runs before global initializers (an unseeded `math::rand` must be seeded before any user
  code), so it must not read globals.
- Typical sources: x86 `RDRAND`/`RDSEED`, aarch64 `RNDR` (optional `FEAT_RNG`), RISC-V Zkr
  `seed` CSR, or a device. Those are instructions, not MMIO, so a `MEMORY` block cannot reach
  them today (§12).
- Only `math::rand` needs it. Today `math::seed` alone also triggers the startup seed
  (bug-1096); once that is fixed, a program that calls only `math::seed` needs no `bmSeed`.

### Vector handlers (`vectors`)

`vectors` maps a vector name to a handler function: `{ "vecName": "funcName" }`. Vector names
come from the **architecture**, never from the chip: each is a slot of that CPU's own vector
table, so the compiler knows all of them without a board database. Which device sits behind a
slot is the handler's business.

| Arch | Names |
|---|---|
| aarch64 | `sync`, `irq`, `fiq`, `serror` (the four current-exception-level slots; the 12 lower-level slots are unused) |
| riscv64 | Interrupts: `software`, `timer`, `external`. Exceptions: `illegalInstruction`, `breakpoint`, `misalignedLoad`, `misalignedStore`, `loadAccessFault`, `storeAccessFault`, `instructionAccessFault`, `ecall`, `loadPageFault`, `storePageFault`, `instructionPageFault`. One compiler-generated trap entry dispatches on `mcause`/`scause`, so the optional hardware vectored mode is not needed. |
| x86-64 | Exceptions 0–31 by name: `divide`, `debug`, `nmi`, `breakpoint`, `overflow`, `bound`, `invalidOpcode`, `deviceNotAvailable`, `doubleFault`, `invalidTss`, `segmentNotPresent`, `stackFault`, `generalProtection`, `pageFault`, `x87`, `alignmentCheck`, `machineCheck`, `simd`, … Interrupts: `vector32` … `vector255`. One IDT entry each. |

Rules:

- An unknown name, a name from another architecture, or a function that doesn't exist or has
  the wrong signature is a compile error.
- A slot not listed is fatal: reported through `bmStderr`, ending in `bmExit`.
- **Interrupt handlers** (`irq`, `fiq`, riscv64 `software`/`timer`/`external`, x86-64
  `nmi` and `vector32`–`vector255`) have the form `FUNC onIrq AS Nothing`. When the handler
  returns, the interrupted code resumes.
- **Exception handlers** (every other name) have the form
  `FUNC onFault(cause AS Integer, at AS Integer, addr AS Integer) AS Nothing`: riscv64
  `mcause`/`mepc`/`mtval` (or the `s` versions), aarch64 `ESR`/`ELR`/`FAR`, x86-64 the vector
  number, the faulting `rip`, and `CR2`. In v1 an exception is still fatal: when the handler
  returns, the runtime reports through `bmStderr` and ends in `bmExit`. (Resuming would loop
  forever on a bad load.)
- **Wrapper.** The compiler generates the entry for each listed slot: save every register,
  FP and (riscv64) vector state included; call the handler; restore; return (`eret`, `mret`,
  `sret`, `iretq`).
- **No nesting.** All three architectures mask further interrupts on entry (x86-64 through
  interrupt gates), and the wrapper does not unmask them.
- **Handlers must not allocate.** A handler can start in the middle of an allocation, and the
  arena state is one pinned register (`x19`/`s11`/`r15`) shared with the interrupted code. The
  same compile-time call-graph check as `bmGetMem` applies (§12).
- **Dispatch to a device is the program's job.** `onIrq` asks the interrupt controller which
  source fired, runs the right code and signals completion: PLIC/APLIC claim/complete,
  x86-64 LAPIC EOI and aarch64 GICv2 `GICC_IAR`/`GICC_EOIR` are memory-mapped and go through
  `MEMORY` blocks. aarch64 GICv3 uses system registers, so it goes through the built-ins in
  §9 #36.
- **Turning interrupts on and off** (`DAIF`, `mstatus.MIE`/`sstatus.SIE`, `sti`/`cli`) is
  `INTERRUPTS ON` / `INTERRUPTS OFF` (#36). Startup leaves them off.

Example (aarch64, GICv3, a PL011 UART on INTID 33):

```basic
MEMORY
    FUNC onIrq AS Nothing
        LET id = INTERRUPT CLAIM            ' GICv3 ICC_IAR1_EL1
        IF id = 33 THEN
            uartReceive()                   ' MEMORY code in the UART's block
        END IF
        INTERRUPT COMPLETE id               ' GICv3 ICC_EOIR1_EL1
    END FUNC
END MEMORY
```

### Example

```basic
MEMORY
    LAYOUT Pl011 CLOSE BY Nothing
        data AS CUInt32             ' @ 0x00 DR
        RESERVED 20
        flags AS CUInt32            ' @ 0x18 FR
    END LAYOUT

    FUNC bmStdout(text AS String) AS Nothing
        RES u AS Pl011 = OVERLAY Pl011 AT uart
        FOR EACH b IN strings::toBytes(text)       ' allocates; see §12
            WHILE bits::band(u.flags, 32) <> 0     ' TX FIFO full
            END WHILE
            u.data = toInt(b)
        NEXT
    END FUNC
END MEMORY
```

## 7. Standard library on bare metal

Packages not on the keep list are rejected at `IMPORT` with a diagnostic naming the target.

### Method

Every member's `const EX` example in `src/codegen/builtins/<pkg>/func_*.rs` was built with
`--ncode --target linux-aarch64` (301 members, 16 packages; probe `/tmp/bmprobe/exprobe.py`, a
one-off). A member is **clean** when its example imports nothing beyond the hello-world set
(`_exit abort clock_gettime fcntl open signal write raise __errno_location`) and links no
`runtime.(fs|os|term|datetime|thread|crypto|net|tcp|tls|process|canvas|app|audio|stdin)`
function.

### Keep (14 packages)

| Package | Clean | Unchecked (no example) |
|---|---|---|
| `big` | 29/29 | |
| `bits` | 17/17 | |
| `color` | 28/28 | |
| `compress` | 7/7 | |
| `encoding` | 30/30 | |
| `general` | 18/18 | |
| `money` | 3/3 | |
| `vector` | 19/19 | |
| `strings` | 41/43 | `gen_cat`, `line_break_class` |
| `collections` | 47/50 | `find`, `mid`, `replace` |
| `astrings` | 15/18 | `read_spans`, `scalar_len`, `write_spans` |
| `regex` | 8/10 | `gen_cat`, `script_of` |
| `math` | 19/21 | `rand`/`seed` need only the seed source (§6 `bmSeed`) |
| `errorcode` | — | constants only; codes of removed packages are unused, which is harmless |

`math::sin` + `math::pow` link no `libm` (measured: a `--nobj --target linux-aarch64` build
imports exactly the hello-world set).

Fix-up before `term` can be removed: `strings::padLeftToWidth`/`padRightToWidth`
(`strings/helper_pad_to_width.rs`) call `term::measureTextRaw`/`scalarWidthRaw`. They measured
clean (pure inline code) but live in `term`; move them to `strings` or a shared helper.

### Remove (20 packages)

Wholly OS:

| Package | Members | Why |
|---|---|---|
| `tcp` | 11 | sockets, `getaddrinfo`, `poll`, `accept4` |
| `udp` | 8 | sockets, `sendto`/`recvfrom` |
| `tls` | 11 | `dlopen` OpenSSL, plus the `tcp` calls |
| `http` | 19 | written in MFBASIC on top of `tcp`, `tls`, `fs` |
| `process` | 14 | `fork`/`execvp`/`waitpid`/`pipe2`/`kill` (`posix_spawn` on macOS) |
| `thread` | 12 | pthreads, a separately allocated heap per thread |
| `audio` | 11 | `dlopen` libasound (CoreAudio on macOS) |
| `app` | 6 | GTK4/AppKit window, title lock |
| `perf` | — | injected into `--debug` builds only; clock, `write`, `mmap` |

Partly usable, removed whole:

| Package | Members | Why |
|---|---|---|
| `io` | 16 | `print`/`write` go to fd 1/2; readers use `read`, `poll`, terminal modes, the shared-stdin lock (a pthread mutex even single-threaded, `io/stdin/stdin_broadcast.rs`) |
| `term` | 27 | `tcgetattr`/`tcsetattr`, `ioctl` window size, `isatty`, `getenv`, `poll` |
| `datetime` | 44 | `nowNanos`/`monotonicNanos` need `clock_gettime`; local zone needs `localtime_r`. UTC and fixed-offset maths would work. |
| `crypto` | 21 | `randomBytes`, `uuid4`/`uuid7`, `ulid`, `randomInt` need OS entropy; `generate`/`sign`/`verify` `dlopen` libcrypto. Hashes, HMAC and AEAD would work. |
| `os` | 25 | environment, pid, `uname`, `sysinfo`, user/home, `/proc/self/exe`, `nanosleep`, argv |
| `fs` | 46 | files, directories, `stat`, temp files, `memfd_create` |
| `net` | 5 | `lookup` → `getaddrinfo`; `ping` → ICMP sockets |
| `canvas` | 42 | Metal/Vulkan via `dlopen`, a graphics thread, system fonts, clock, mouse |

Removed because they depend on `fs`:

| Package | Members | Why |
|---|---|---|
| `csv` | 3 | All 3 examples link the `fs` runtime. `csv::parse(String)` opens `fs::openData("", "r")` and feeds the shared scanner (`__csv_parse`, `csv/func_parse.rs`): `memfd_create`, `read`, `lseek`, `close`. Keepable if the scanner stops taking an `fs::File`. |
| `json` | 5 | Every member links `runtime.fs.readAll`, even a `stringify`-only program: the `parse(fs::File)` overload body is linked whenever `json` is used. Keepable if that overload is split out (possible dead-code bug, §11). |

Test harness:

| Package | Members | Why |
|---|---|---|
| `testing` | 12 | `expect*`. `mfb test` runs the binary only on the host, reports through `io`/`fs`, writes coverage files. |

## 8. Other features not available on bare metal

| Feature | Why | Handling |
|---|---|---|
| `LINK` blocks, vendor libraries | `_mfb_linker_init` calls `dlopen`/`dlsym` before `main` (`src/codegen/link/thunk/link_thunk.rs`) | compile error |
| `resources` | copied beside the executable, found through `os::appResourcePath`; nothing is embedded | error (embedding is a possible later addition) |
| `mode: "app"`, `--app`, `icon` | OS windowing | error |
| `main(args AS List OF String)` | argv comes from the OS | error; only `FUNC main` with no arguments |
| `--debug` `perf` injection | needs clock, `write`, `mmap` | error (or rewire to §6 later) |
| `mfb test` | runs only when `target.is_host()` | build only; a QEMU runner is a later addition |
| debugger | attaches to a host PID | error (a gdbstub is a later addition) |
| threads, `ISOLATED FUNC` | pthreads | error |

`LINK` needs one more step: a new OS token joins the `LINK` locator vocabulary automatically
(`registered_target_oses`). Without excluding `baremetal`, every binding package gets
`NATIVE_LIBRARY_TARGET_UNCOVERED` warnings for bare-metal slots.

## 9. The `MEMORY` block

### The core analogy (load-bearing)

**`MEMORY` block : field-access / raw memory  ::  `LINK` block : `CPtr`.**

The raw capability (dereferencing an overlay's fields, indexing a window) is legal **only
inside the declaring block**. Outside, the overlay type is an **opaque resource**: you can only
pass it through the block's own `FUNC`s. This reuses the §15 resource model's opacity,
non-owning, no-copy and no-store-in-collection rules for free, and makes the block the single
audited trust boundary, exactly like `LINK`.

As with every existing resource, there is **one resource identity** and every `RES` binding or
function parameter is an alias to it. A `LAYOUT` resource and the memory it describes are never
copied or moved as `LET`/`MUT` values. Returning a resource can change its cleanup scope or
which aliases remain valid, but it never creates another resource or relocates the memory.

```basic
' Compile error: UartRegs is opaque outside its MEMORY block (NATIVE_CPTR_ESCAPE-style)
FUNC getUartStatusTest(RES r AS UartRegs) AS Boolean
    RETURN bits::band(r.status, 1) = 1
END FUNC
```

### Settled decisions

Numbering is stable; replaced items keep their number.

1. **Producer FUNCs return a `RES`** (like `LINK` wrappers). Resources come from a call, are
   scope-bound, and drop through a real close. The overlay *descriptor* (the small struct of
   pointers/offsets) is allocated normally; only the *target memory* is raw. A producer's
   signature names only the type (`FUNC openUart AS UartRegs`); the close comes from the
   type (#2). (Conflict with `bmGetMem`, which runs before the arena: §6, §12.)

2. **`CLOSE BY` belongs to the `LAYOUT`, not the producer.** Written on the header:
   `LAYOUT UartRegs CLOSE BY closeUart`, the same form as `RESOURCE Name CLOSE BY fn` (§17).
   The clause is **required**; `CLOSE BY Nothing` states explicitly that the type needs no
   teardown. The close can do real work on drop, e.g. quiesce the device.
   - **The close function lives in the same `MEMORY` block** as its `LAYOUT` (it needs field
     access, #27) and may be declared later in the block.
   - **One teardown per type.** Two producers that need different teardown need two `LAYOUT`s.
   - **Partly built resources close normally.** `OVERLAY` creates the resource identity with
     its close already attached, so a failure after `OVERLAY` and before the producer returns
     drops the local `RES` and runs the close under §15. In `attachDisplay`, a `VIEW` that
     fails (`ErrInvalidArgument`, #30) runs `blankDisplay` on a resource whose `pixels` is
     still the null view. A close must not assume its `MEM` fields are wired; touching an
     unwired one fails with `ErrIndexOutOfRange` (#21).
   - **Drop runs only the user's close.** There is no mapping to undo (#34).

3. **Two distinct tools inside the same block**; do NOT unify them through `CPtr`:
   - **(a) Register overlays:** fixed register maps (a UART). Width-typed fields, and **field
     access *is* the volatile register I/O**: `r.status` is one volatile read of the field's
     width; `r.control = 0` is one volatile write. The field offsets *are* the register map.
   - **(b) Dynamic windows:** byte regions (a framebuffer). A fat-pointer **view** (`MEM` =
     pointer **and** length) with bounds-checked `PEEK r.mem AT off FOR n` /
     `POKE r.mem AT off FOR n = val` (`FOR n` mandatory on both, #15).

   Forcing both through `CPtr` fields makes `r.control = 0` ambiguous: null a pointer, or
   write the register. See "Why not unify" below.

4. **Views carry their length** (`MEM` fat pointer), **not bare `CPtr`.** A bare address has
   no length, so `AT`/`FOR` would have nothing to bounds-check. `CPtr` never appears in a
   `MEMORY` block (#35).

5. **Field assignment is a `MEMORY`-block-local exception.** The language has no field
   assignment generally; inside the block, `r.control = 0` (register write) and overlay
   construction are permitted and lower to volatile stores.

6. **C-width types in source positions are OK *because* they're quarantined.** `CUInt16`/
   `CUInt32` as overlay field types never escape (the type is opaque outside), and a field
   read marshals `CUInt32 → Integer` (zero-extended; a signed `CInt*` field sign-extends,
   #28), exactly like a `LINK` return.

7. **Block keyword is `MEMORY … END MEMORY`.** Chosen over reusing `RESOURCE`, which would
   collide with `RESOURCE Name CLOSE BY fn` and the inner `RES … CLOSE BY`.

8. **`LAYOUT` overlay type: transparent inside, opaque outside.** Fields are accessible inside
   the declaring block and opaque everywhere else, including every other `MEMORY` block (#27).

9. **Low-n-bytes placement (reads) / extraction (writes); n = 8 is the degenerate case.**
   - A field/`PEEK` of `n` bytes places those bytes into the low `n` bytes of the destination
     `Integer`; the high `8 − n` bytes are zero, so for `n < 8` the result is non-negative
     (signed fields are the exception, #28). For **`n = 8`** the read is an **exact 64-bit
     bit-copy**, so a value with bit 63 set reads back negative. Code needing unsigned
     semantics uses `bits::`. `FOR 8` is allowed.
   - A field write or `POKE … FOR n = value` stores the **low `n` bytes** of the value's bit
     pattern; the high `8 − n` bytes are discarded (no range check). `r.control = v` on a
     `CUInt16` field with runtime `v = 0x1_0005` writes `0x0005`; `POKE … FOR 2 = -1` writes
     `0xFFFF`. `POKE … FOR n = v` then `PEEK … FOR n` yields `bits::band(v, 2^(8n) − 1)` for
     `n < 8`, and `v` for `n = 8`. Byte order is little-endian (#13).
   - **A constant that doesn't fit is a compile error.** A constant written to `n` bytes must
     lie in −2^(8n−1) … 2^(8n) − 1. `POKE … FOR 1 = 0x1FF` is rejected; runtime values keep
     the silent truncation.

9b. **Access width equals field width; no splitting.** A `CUInt64` field is one 64-bit bus
   access. A device that permits only 32-bit access is modeled with two `CUInt32` fields.

10. **`MEM` is a simple byte-window type, `PEEK`/`POKE` only.** Not `MemView OF T`, not a List:
    a bounded byte window (pointer + length) with no `length`/index/iterate/slice surface. To
    build a List you `PEEK` each element and `append()`. `MEM` is valid **only inside a
    `MEMORY` block**. Bounded subviews are deferred.

11. **Overlay layout is packed: what you write is what you get.** Fields are placed in
    declaration order with no automatic padding and no reordering.
    - **Gaps are written `RESERVED n`**, an unnamed, inaccessible `n`-byte skip.
    - **Each register field's offset must be a multiple of its width**, or it is a compile
      error. A `CUInt32` at `0x01` is rejected; a 64-bit register at a 4-aligned offset is two
      `CUInt32` halves.
    - **Fields never overlap.** A shared address (the 16550's RBR/THR/DLL) is one field.
    - **No trailing padding.** A layout's size is the sum of its register fields and
      `RESERVED` bytes. A `MEM` field takes no bytes in the register map (#35).
    - **The base must be aligned to the widest register field.** The base is a region base
      plus an offset (#34): a constant offset is checked at compile time; a runtime offset is
      checked when `OVERLAY` runs and fails with `ErrInvalidArgument`.

12. **Scope is memory-mapped I/O and program-owned RAM.** There is no OS, so there are no
    device nodes or libraries to bind instead. Port I/O (x86 `in`/`out`) and system
    registers are out of scope (§12).

13. **Multi-byte access is defined little-endian.** All three targets are little-endian, so
    this compiles to plain loads and stores. The same source behaves identically on every
    target.

14. **Grammar: `PEEK` is an expression, `POKE` is a statement.** `PEEK … AT off FOR n` yields
    a primitive usable inline; `POKE … AT off FOR n = value` never yields a value. This is what
    `PEEK()`/`POKE` have meant since classic BASIC (#26).

15. **`FOR n` is required on both `PEEK` and `POKE`, and `n` is the literal 1, 2, 4 or 8.** No
    width inference: MFB literals are untyped, so `POKE r.mem AT off = 0` would be ambiguous.
    - **Integer literal only**, not a named constant or expression, so every width check is
      static.
    - This removes a parse hazard: with an expression after `FOR`, `POKE … FOR W = val` would
      read `W = val` as an equality test, and so would `IF PEEK r.mem AT 0 FOR 4 = 1 THEN`.

16. **Keep the `MEMORY` domain small.** A block contains only `LAYOUT` declarations (any
    number, #27) and `FUNC` wrappers; no constants, type aliases or other top-level forms. It
    *may reference* symbols defined outside (e.g. an outside `CONST`).

17. **Overlay declaration keyword is `LAYOUT … END LAYOUT`** (not `TYPE`), so the overloaded
    meaning of field access is explicit at the declaration.

18. **`MEMORY`-only keywords, and this is load-bearing.** `LAYOUT`, `MEM`, `PEEK`, `POKE`,
    `OVERLAY`, `VIEW`, `FENCE`, `RESERVED`, `AT`, `INTERRUPTS` and `INTERRUPT` are valid **only
    inside a `MEMORY` block**. `COMPLETE` is a keyword only directly after `FENCE` or
    `INTERRUPT` (#24, #36); `ON`/`OFF` only after `INTERRUPTS`; `CLAIM` only after `INTERRUPT`.
    - **`AT` must be contextual.** `find . -name '*.mfb' -not -path './target/*' -print0 |
      xargs -0 grep -o -i -w at | wc -l` reports 23,099 matches across 20,564 files.
    - **Existing keywords with new roles.** `FOR` and `WITH` are already global keywords
      (`src/lexer.rs:keyword`). `PEEK`/`POKE`/`VIEW`/`OVERLAY` consume their own `AT`/`FOR`
      clauses, and because `n` is a bare literal (#15), `FOR i = 0 TO PEEK r.mem AT 0 FOR 4`
      is unambiguous. Each shape gets a parser test.
    - **The opener `MEMORY` gets its own rule.** It opens a block only at the start of a
      top-level statement followed by end of line; elsewhere `memory` is an ordinary
      identifier.
    - **Why it's load-bearing.** `keyword()` matches case-insensitively, and the identifier
      after `::` goes through the same lookup, so global keywords would capture
      `collections::mem`, `foo::layout` and friends. Across the tree's 27,369 `.mfb` files:
      `mem` 75 word-matches, `layout` 52, `memory` 233 (`view`, `fence`, `overlay` 0).
    - **Implementation:** the lexer recognizes this vocabulary contextually, tracking
      `MEMORY … END MEMORY`, not through the global keyword table (precedent: `DOC` raw
      capture, `REM` at statement start, `is_statement_start`).
    - **Residual cost, accepted:** inside a block these spellings can't be identifiers.

19. **Volatile by definition; no annotation.** Every `LAYOUT` field, `PEEK` and `POKE` access
    is one in-order bus access, exempt from fold/CSE/reorder/caching.

20. **Device memory is uncached by construction.** Volatile (one in-order instruction per
    access) is the compiler's contract. Non-cacheable is a mapping property; on bare metal the
    compiler builds the page tables, so every `device` region (§4) is mapped as device memory
    (aarch64 Device-nGnRE, x86-64 UC). On riscv64 translation is off and cacheability of an
    address is fixed by the chip. A write-combining kind for framebuffers is a possible later
    addition.

21. **An unwired `MEM` field is the null view `{ptr: 0, len: 0}`.** Every access is
    bounds-checked against `len`, so any `PEEK`/`POKE` on an unwired field fails with
    `ErrIndexOutOfRange` and never dereferences null.

22. **`VIEW` bounds are overflow-free.** `VIEW AT region [+ off] [FOR size]` (#34). `size`
    defaults to the region size minus `off`. Checks: `off ≥ 0`, `size ≥ 0`, `off + size ≤`
    region size, computed without overflow; constant values are checked at compile time,
    runtime values fail with `ErrInvalidArgument`. A region based at address 0 is allowed.
    A `PEEK`/`POKE` is in bounds iff `0 ≤ off` **and** `1 ≤ n` **and** `n ≤ len` **and**
    `off ≤ len − n` (the subtraction is guarded by `n ≤ len`; not `off + n > len`, which can
    wrap).

23. **Alignment: register fields aligned by construction; `MEM` windows are the programmer's
    responsibility.** Register-field accesses are always naturally aligned (#11). A `PEEK`/
    `POKE` offset need not be aligned, but the single-access guarantee applies only when
    `off % n = 0`; an unaligned access on device memory may fault.

24. **`FENCE` and `FENCE COMPLETE`, `MEMORY`-block-only barriers.** Both order all prior
    `MEMORY`-block accesses before any subsequent one.
    - **`FENCE` orders.** Use it for the fill-buffer-then-write-"go" DMA pattern.
    - **`FENCE COMPLETE` waits** for prior accesses to complete.

    | Target  | `FENCE`           | `FENCE COMPLETE`  |
    |---------|-------------------|-------------------|
    | aarch64 | `DMB SY`          | `DSB SY`          |
    | x86-64  | `mfence`          | `mfence`          |
    | rv64    | `fence iorw,iorw` | `fence iorw,iorw` |

    On RISC-V device accesses count as I/O, so `fence rw,rw` would not order them. On an
    aarch64 Device-nGnRE mapping a write may be acknowledged early, so a driver that must know
    the device saw it reads a register back. MFB never inserts barriers itself.

25. **`OVERLAY … [WITH …]` is an expression.** It appears as a bare trailing expression
    (implicit return) or as the right side of `RES r AS T = …`.

26. **The window accessors are `PEEK`/`POKE`, not `GET`/`SET`.**
    - `GET`/`SET` collide with `collections::`: across 27,369 `.mfb` files, `get` has 14,238
      word-matches and `set` 3,135. `PEEK`/`POKE` have 0/0.
    - `PEEK`/`POKE` are BASIC's own words for raw, width-explicit memory access, and they do
      not understate the danger the way `collections::get` would.

27. **Block structure: many blocks, many `LAYOUT`s, the block is the boundary.**
    - A file may contain any number of `MEMORY` blocks; a block any number of `LAYOUT`s.
    - **The block, not the `LAYOUT`, is the opacity boundary.** Every `FUNC` in a block may
      access the fields of every `LAYOUT` in that block:

      ```basic
      FUNC kick(RES dma AS DmaRegs, RES d AS Display) AS Nothing
          dma.srcLo = d.bufOffset
          FENCE
          dma.control = 1
      END FUNC
      ```

    - **Blocks are isolated from each other.** Block B may name A's `LAYOUT`, bind it with
      `RES` and pass it to A's functions, but not touch its fields.
    - **`LAYOUT` names live in the package namespace,** so two blocks in one package can't
      both declare `UartRegs`.
    - **A type's `CLOSE BY` function is in the same block as its `LAYOUT`** (#2).
    - **Blocks do not nest** (#18).
    - Wrapper visibility follows normal package rules. A `LAYOUT` named by an exported
      signature is nameable as an opaque type by callers.

28. **Signed fields are allowed.** `CInt8/16/32/64` may be field types. A read below 8 bytes
    sign-extends; an 8-byte read is the same bit-copy as `CUInt64` (#9). `PEEK` has no signed
    form.

29. **Mutating, returning and re-wiring a `RES` use existing resource semantics.**
    - **Aliases, not copies.** Only code inside the declaring block may access or assign a
      field; an assignment mutates the one resource in place, visible through every alias.
    - **`RETURN r` moves nothing and does not run `CLOSE BY`.** Cleanup responsibility for a
      freshly produced resource passes to the caller under §15.
    - **A `MEM` field may be reassigned** any number of times inside the declaring block.
      Reassignment replaces the view on the one shared resource.

30. **Failures are ordinary MFB failures, catchable with `TRAP`.** Existing error codes
    (`src/docs/spec/diagnostics/02_error-codes.md`):

    | Failure | Code |
    |---------|------|
    | `PEEK`/`POKE` out of bounds, including an unwired `MEM` | `ErrIndexOutOfRange` (`77050001`) |
    | Runtime offset negative or outside the region, misaligned runtime `OVERLAY` base, negative `VIEW` size | `ErrInvalidArgument` (`77050002`) |

    The descriptor allocation can fail with `ErrOutOfMemory` like any allocation. `CLOSE BY`
    follows the existing rule: an explicit close propagates its failure; a failure during
    implicit drop is recorded as cleanup metadata and cannot replace an error in flight.

31. **Read-modify-write is never atomic.** `r.control = bits::bor(r.control, 1)` is two
    accesses. There are no generic MMIO atomics; use device set/clear registers.

32. **No threads on bare metal (v1).** `THREAD_SENDABLE` is not accepted on a `LAYOUT`. If
    threads come later, the opt-in uses the `RESOURCE` spelling:
    `LAYOUT UartRegs CLOSE BY closeUart THREAD_SENDABLE`.

33. **`MEMORY` is bare-metal-only, and that propagates through the import graph.** A `MEMORY`
    block on any non-`baremetal-*` target is a compile error at the block, naming the target.
    A `.mfp` carries a transitive "bare-metal only" bit, set when the package or any dependency
    uses `MEMORY`, so a binary-only package can't hide it. A non-bare-metal build that imports
    such a package fails and reports the shortest deterministic import chain to it. There is
    no `--mem` switch.

34. **Addresses are names from `mmap`; `AT` takes a region name, optionally plus an offset.**
    - Forms: `OVERLAY T AT <name>`, `OVERLAY T AT <name> + <offset>`,
      `VIEW AT <name> [+ <offset>] [FOR <size>]`.
    - `<name>` is an `mmap` region (§4). There is no address-expression form, so in the `AT`
      slot a name is always a region, never a variable.
    - `<offset>` is an `Integer` expression. Constant: in range, aligned (#11) and size-fit
      are checked at compile time. Runtime: checked when `OVERLAY`/`VIEW` runs, failing with
      `ErrInvalidArgument` (#30).
    - `OVERLAY` checks that the `LAYOUT`'s packed size (#11) fits between the offset and the
      end of the region.
    - The region may be `device` or program-owned `ram`. The compiler-owned `ram` region (the
      one holding `loadAddress`) is rejected.
    - There is no mapping step: the page tables identity-map every region, so the address is
      used directly. Drop does not unmap anything.

35. **`LAYOUT` field types are C integers and `MEM` only.** Allowed: `CUInt8/16/32/64`,
    `CInt8/16/32/64` (#28), `MEM`, plus `RESERVED n` gaps (#11). Rejected: `CFloat`/`CDouble`
    (decode floats with `bits::`), `CPtr`, `CString`, `CBool`, `CVoid`. A `MEM` field is
    runtime-kept pointer and length, takes no space in the register map, and does not move the
    offset cursor.

36. **System built-ins: the only system-register access.** A `MEMORY` block reaches memory,
    not system registers. These few compiler-provided forms cover what interrupt handling needs
    and nothing more; there is no general system-register access and no `ASM`.

    | Form | Kind | aarch64 | riscv64 | x86-64 |
    |---|---|---|---|---|
    | `INTERRUPTS ON` | statement | clear `DAIF.I`/`DAIF.F` | set `mstatus.MIE` / `sstatus.SIE` | `sti` |
    | `INTERRUPTS OFF` | statement | set `DAIF.I`/`DAIF.F` | clear `mstatus.MIE` / `sstatus.SIE` | `cli` |
    | `INTERRUPT CLAIM` | expression → `Integer` | GICv3 `ICC_IAR1_EL1` | compile error (use the PLIC's memory-mapped claim register) | compile error (the LAPIC is memory-mapped) |
    | `INTERRUPT COMPLETE <id>` | statement | GICv3 `ICC_EOIR1_EL1` | compile error | compile error |

    - Each form is one instruction (plus an `isb` where the architecture requires one), and
      counts as a volatile access for ordering (#19).
    - `INTERRUPT CLAIM` / `INTERRUPT COMPLETE` on an aarch64 chip without the GICv3 system-register
      interface is an undefined-instruction exception (fatal, or the `sync` handler). GICv2 chips
      use the memory-mapped `GICC_*` registers instead.
    - `INTERRUPTS ON` with no interrupt slot named in `vectors` is a compile error: an interrupt
      would reach a fatal slot.

### Deferred

- **Register arrays** (N identical channel blocks in one `LAYOUT`). MFB has no bracket
  indexing. Until then: one field per channel, or one `LAYOUT` overlaid at
  `AT <name> + i * stride` (#34).
- **`READ`/`WRITE` access modes on fields.** Write-only registers are a noted gap.
- **Bounded subviews** of a `MEM`. Until then a helper takes the whole `MEM` plus an offset.
- **Running tests without hardware.** A QEMU runner for `mfb test` (§8).

Dropped: inline `pixels AS MEM AT 0x1000 FOR <const>` sugar; `WITH pixels = VIEW AT …`
already expresses it.

### Why not unify register maps and windows through `CPtr`

`=` cannot mean both "set the pointer" and "write through the pointer":

```basic
uart.data = dataView    ' set-the-pointer  (RHS is a view)
r.control = 0           ' write-through-the-pointer (RHS is a scalar) — intended
```

Splitting into (a) field-access overlays and (b) `MEM` views removes the ambiguity: in (a) the
field *is* the register, so `=` is always a register write; in (b) all access is explicit
`PEEK`/`POKE`.

### Sketches

The sketches assume the `mmap` from §4 (`uart`, `disp`, `vram`).

#### (a) Register overlay

```basic
MEMORY
    LAYOUT UartRegs CLOSE BY closeUart  ' teardown belongs to the type (#2)
        data    AS CUInt32
        status  AS CUInt32
        control AS CUInt16          ' @ 0x08; layout size is 10 bytes, no padding (#11)
    END LAYOUT

    FUNC openUart AS UartRegs
        OVERLAY UartRegs AT uart    ' region from mmap (#34)
    END FUNC

    FUNC getUartStatus(RES r AS UartRegs) AS Boolean
        RETURN bits::band(r.status, 1) = 1   ' one volatile 32-bit read
    END FUNC

    FUNC closeUart(RES r AS UartRegs) AS Nothing
        r.control = 0                        ' one volatile 16-bit write
    END FUNC
END MEMORY
```

Two identical UARTs 0x100 apart in one region: `OVERLAY UartRegs AT uart + 0x100`.

#### (b) Dynamic window

```basic
MEMORY
    LAYOUT Framebuffer CLOSE BY Nothing  ' no teardown, stated explicitly (#2)
        mem AS MEM                  ' a view (pointer + length), NOT a bare CPtr
    END LAYOUT

    FUNC openFb AS Framebuffer
        OVERLAY Framebuffer WITH mem = VIEW AT vram   ' whole region; length from mmap
    END FUNC

    ' rowBytes: bytes per row, supplied by the caller
    FUNC setPixel(RES r AS Framebuffer, x AS Integer, y AS Integer, rowBytes AS Integer, val AS Integer) AS Nothing
        POKE r.mem AT y * rowBytes + x FOR 1 = val   ' 1-byte write, bounds-checked
    END FUNC
END MEMORY
```

#### (c) Mixed register + buffer in one overlay

The split in #3 is about *access modes*, not *resources*. One resource can hold register
fields and `MEM` buffer fields; the field type picks the mode.

```basic
MEMORY
    LAYOUT Display CLOSE BY blankDisplay
        ' --- registers: field access = volatile I/O ---
        width     AS CUInt32        ' @ 0x00
        height    AS CUInt32        ' @ 0x04
        control   AS CUInt32        ' @ 0x08
        bufOffset AS CUInt32        ' @ 0x0C  pixel buffer's offset inside vram
        ' --- buffer: indexed bounded PEEK/POKE ---
        pixels    AS MEM            ' takes no bytes in the register map (#35)
    END LAYOUT
```

**Static:** the buffer sits at a fixed place.

```basic
    FUNC openDisplay AS Display
        OVERLAY Display AT disp
            WITH pixels = VIEW AT vram FOR 1920 * 1080 * 4
    END FUNC
```

**Register-described:** the device reports where the buffer is inside `vram`. Bind the overlay,
read the registers, then form the view with a runtime offset.

```basic
    FUNC attachDisplay AS Display
        RES r AS Display = OVERLAY Display AT disp   ' registers usable now; pixels unset
        ' Geometry is snapshotted once: the view length is fixed here.
        LET w = r.width
        LET h = r.height
        ' If VIEW fails (#30), r drops here and blankDisplay runs (#2).
        r.pixels = VIEW AT vram + r.bufOffset FOR w * h * 4
        RETURN r
    END FUNC
```

(A device that reports an absolute address rather than an offset needs the region's base as a
value; §12.)

Both access modes in the wrappers:

```basic
    FUNC setPixel(RES r AS Display, x AS Integer, y AS Integer, argb AS Integer) AS Nothing
        POKE r.pixels AT (y * r.width + x) * 4 FOR 4 = argb   ' bounds-checked buffer write
    END FUNC

    FUNC getPixel(RES r AS Display, x AS Integer, y AS Integer) AS Integer
        RETURN PEEK r.pixels AT (y * r.width + x) * 4 FOR 4   ' bounds-checked read
    END FUNC

    FUNC present(RES r AS Display) AS Nothing
        FENCE                                                  ' pixel writes first (#24)
        r.control = bits::bor(r.control, 1)
    END FUNC

    FUNC blankDisplay(RES r AS Display) AS Nothing
        r.control = 0                                          ' register write on drop
    END FUNC
END MEMORY
```

Notes on the mix:

- Register fields sit at fixed offsets from the overlay base (packed, little-endian, #11,
  #13). A `MEM` field is wired by the producer (`WITH …` or field assignment).
- **The view is a snapshot.** If the device later changes geometry, re-attach.
- **Live read vs snapshot.** Each direct field use (`r.status`) is a fresh volatile read.
  `LET current = r.control` is one read whose value then never changes. Read directly for
  status and ready bits and polling; snapshot for geometry, read-modify-write and
  read-to-clear registers.

### `PEEK` / `POKE` semantics

`PEEK <memField> AT <off> FOR <n>` reads `n` bytes from the view at `off` and yields an
`Integer`.

- **Low-`n`-bytes placement** (#9); `n` is the literal 1, 2, 4 or 8 (#15).
- **Primitives only.** No List/Map/record/bulk form; build a List by `PEEK`-ing each element.
- **Bounds-checked** against the view length, overflow-free (#22).

`POKE <memField> AT <off> FOR <n> = <value>` writes the low `n` bytes of `value` at `off`, with
the same rules. `FOR n` is required.

```basic
MEMORY
    ' ... LAYOUT Display as in sketch (c) ...

    FUNC readRow(RES r AS Display, y AS Integer) AS List OF Integer
        LET w = r.width                       ' snapshot geometry once
        MUT row AS List OF Integer = []
        MUT x AS Integer = 0
        WHILE x < w
            row = collections::append(row, PEEK r.pixels AT (y * w + x) * 4 FOR 4)
            x = x + 1
        END WHILE
        RETURN row
    END FUNC
END MEMORY
```

### Volatile semantics: the implementation crux

Every `LAYOUT` field, `PEEK` and `POKE` access is **exactly one bus access of the declared
width, in program order**. The compiler exempts these access nodes from:

- store-to-load forwarding (plan-01 float),
- CSE / common-subexpression folding (two `r.status` reads must not collapse),
- dead-store elimination,
- reordering / coalescing,
- register-allocator value caching.

Read-only does **not** make a value constant: a status register changes between reads. The
exemption applies only to `MEMORY`-block access nodes, so the rest of the program keeps full
optimization.

```basic
MEMORY
    ' ... LAYOUT UartRegs as in sketch (a) ...

    FUNC resetDevice(RES r AS UartRegs) AS Nothing
        LET current = r.control               ' one volatile read
        LET next    = bits::bor(current, 128) ' set the reset bit (bit 7)
        r.control = next                      ' one volatile write
        r.control = current                   ' one volatile write (restore), NOT dead-store-eliminated
        RETURN NOTHING
    END FUNC
END MEMORY
```

To an ordinary optimizer the first write looks dead; the exemption keeps both writes on the
bus, in order.

### How these differ from §15 resources

1. **Field access permitted inside the declaring block** (lifts the §15 "never
   field-accessed" rule, only inside, like `CPtr` inside `LINK`).
2. **Backed by raw memory, not an arena allocation** (the descriptor is allocated; the target
   memory is not).
3. Otherwise unchanged: bound with `RES`, passed by pointer, auto-closed by lexical drop,
   never copied or stored in a collection.

### Read-to-clear registers: a limit of the model

Some registers change device state when read. Example: the 16550 Line Status Register clears
its error bits (overrun, parity, framing, break) on read.

```text
LSR bit 0  Data Ready
    bit 1  Overrun Error    \
    bit 2  Parity Error      |  these clear on read
    bit 3  Framing Error     |
    bit 4  Break Interrupt  /
    bit 5  THR Empty
    bit 6  Transmitter Empty
```

Volatile guarantees one bus read per source read, not read-once:

```basic
IF bits::band(r.lsr, 2) <> 0 THEN ...   ' read #1 clears the error bits
IF bits::band(r.lsr, 4) <> 0 THEN ...   ' read #2: the parity bit is already gone
```

The mitigation is a wrapper that reads once and decodes everything:

```basic
' Ordinary record, declared OUTSIDE the MEMORY block (#16)
TYPE LineStatus
    dataReady AS Boolean
    overrun   AS Boolean
    parity    AS Boolean
    framing   AS Boolean
    txEmpty   AS Boolean
END TYPE

MEMORY
    ' ... LAYOUT UartRegs with `lsr AS CUInt8` ...

    FUNC readLineStatus(RES r AS UartRegs) AS LineStatus
        LET s = r.lsr                       ' the single read-to-clear access
        RETURN LineStatus[
            bits::band(s, 1)  <> 0,
            bits::band(s, 2)  <> 0,
            bits::band(s, 4)  <> 0,
            bits::band(s, 8)  <> 0,
            bits::band(s, 32) <> 0
        ]
    END FUNC
END MEMORY
```

Read-to-clear and write-1-to-clear registers are an **accepted limitation**, handled by
convention. A `READCLEAR` field annotation that rejects more than one direct read on a path is
a possible future idea.

## 10. Compiler touch points

Counts from `grep -rn 'PlatformFamily::' src | wc -l` (270 references in 88 files) and the
`CodegenPlatform` trait in `src/codegen/engine/types/types.rs` (95 methods, 76 without a
default).

- **Target registry.** A `baremetal` entry in `NATIVE_BACKENDS` (`src/target.rs`), a
  `PlatformFamily::BareMetal` variant (`platform_family()` hits `unreachable!` on an unknown
  OS), and a decision at each of the 270 `PlatformFamily::` sites.
- **`CodegenPlatform` for bare metal.** `emit_write` → `bmStdout`/`bmStderr`;
  `emit_program_exit` → `bmExit`; `emit_arena_map`/`emit_arena_unmap` → `bmGetMem`/
  `bmRetMem`; `emit_random_bytes` → `bmSeed`; `emit_program_entry` and `emit_stack_limit`
  → §5. The libc defaults (`emit_heap_alloc` = `malloc`, `emit_lib_open` = `dlopen`,
  `emit_arena_start_time` = `clock_gettime`, terminal control) must not be reachable.
- **Allowed runtime calls.** `BackendCapabilities.runtime_calls`, checked late on NIR by
  `validate_capabilities` (`src/target/shared/validate/capabilities.rs`) with no source span.
  The Windows `RUNTIME_CALLS` list is the minimal precedent.
- **Early package rejection with spans.** `src/cli/build/mod.rs` already rejects
  `IMPORT app`/`canvas` without app mode right after parse; the bare-metal package list (§7)
  follows that pattern. `validate_project` (`src/target/shared/validate/mod.rs`) is an empty
  stub and the natural place for §4/§8 checks.
- **Manifest.** Parse and validate the `baremetal` section (§4); exclude `baremetal` from
  `registered_target_oses` (§8).
- **Hidden couplings:** `write_uses_raw_syscall` compares against `"linux-x86_64"`;
  `linux_common/code.rs` return-register staging compares `"linux-x86_64"`/`"windows-x86_64"`;
  the RVV probe is keyed on `"linux-riscv64"` (`builder/mod.rs`, `function/entry.rs`);
  `debug/arena.rs` and `debug/perf.rs` use `starts_with`; the manifest libc axis tests
  `os == "linux"`.
- **Vectors.** Per-arch vector tables and entry wrappers (full register save including FP
  and RVV state), x86-64 GDT/TSS/IST, `vectors` validation against the per-arch name list,
  handler signature checks (§6).
- **Language.** Contextual lexing (#18), `MEMORY`/`LAYOUT` parsing and resolution, volatile
  exemptions (store-to-load forwarding, `src/optimizer/opt2/peephole.rs`, the register
  allocator), the §6 hook checks, the #36 built-ins.
- **Spec.** A bare-metal linker topic and a `MEMORY` language topic (Hard Completion Gate,
  `.ai/specifications.md`).

## 11. Related bugs

- **bug-1096:** `math::seed` without `math::rand` still draws OS entropy at startup and links
  the unused `_mfb_rng_next`. Once fixed, `bmSeed` is needed only for `math::rand`.
- **bug-1097:** linux-x86_64 declares an unused libc `getentropy` import for
  `math::rand`/`math::seed`.
- Not filed: every `json` member links `runtime.fs.readAll`, even a `stringify`-only program
  (§7). Possible dead-code bug.

## 12. Open questions

1. **Allocation-free hooks.** `bmGetMem`, `bmRetMem` and `bmStderr` must not allocate. This
   needs a compile-time check of their call graphs, and two gaps closed:
   - an `OVERLAY` descriptor is arena-allocated (#1), but `bmGetMem` runs before the arena
     exists; the descriptor needs a home outside the arena (static, or a compiler-provided
     overlay for hooks);
   - no way to walk a String's bytes without allocating (`strings::toBytes` builds a list).
     One option is a byte-at-a-time hook, `bmStderrByte(b AS Integer)`, that the runtime
     loops over.
2. **`bmSeed` sources are instructions, not MMIO** (`RDRAND`, `RNDR`, the Zkr `seed` CSR). A
   `MEMORY` block can't execute them. Options: a compiler-provided seed source selected in
   `project.json`, or a device RNG only.
3. **Region base as a value.** A device that reports an absolute buffer address needs
   `phys − base(region)` as an offset. Expose the base (e.g. a `BASE OF vram` expression inside
   `MEMORY`), or require devices to be described by offsets.
4. **Should hooks bring packages back?** `io::print`/`io::write` could route to `bmStdout`;
   `datetime` now to a `bmNow`; `crypto::randomBytes` to a `bmEntropy`; `io` input to a
   `bmRead`. That would shrink the remove list (§7).
5. **Sharing data between a handler and the main code.** Handlers can't allocate, and the
   main code's values live in the arena. A mailbox in program-owned `ram` reached through
   `OVERLAY` (with `FENCE`) works but is clumsy. Decide whether something better is needed.
   Also: the x86-64 red-zone audit (§5) blocks interrupts on that target.
6. **`bmGetMem` alignment:** 16 or 4096. Check what the arena block header and the SIMD list
   helpers assume.
7. **Timers and port I/O are still outside `MEMORY`.** The aarch64 generic timer (`CNTVCT_EL0`,
   `CNTP_CTL_EL0`, `CNTP_TVAL_EL0`) and the riscv64 S-mode timer (`rdtime`, plus an SBI call or
   the Sstc `stimecmp` register) are system registers; x86 `in`/`out` are instructions. The
   riscv64 M-mode timer (CLINT `mtime`/`mtimecmp`) and the x86-64 LAPIC timer are memory-mapped
   and fine. Without a built-in, the `timer` vector is unusable on aarch64 and riscv64 S-mode.
   Candidate: extend #36 with `TIMER NOW` (expression) and `TIMER AT <ticks>` (statement).
8. **Unaligned access on device memory:** whether code generation emits unaligned loads or
   stores anywhere (matters for `MEM` windows on device regions).

## 13. Reference points

- `LINK` design and the resource model: `src/docs/spec/language/17_native-libraries.md`,
  `src/docs/spec/language/15_resource-management.md`.
- C ABI type names and marshaling: `src/codegen/link/thunk/link_thunk.rs`; the `is_c_abi_type`
  allow-list in `src/resolver/mod.rs`.
- Resource resolution (`CLOSE BY`, opaque types, `NATIVE_CPTR_ESCAPE`):
  `src/resolver/resolution.rs`.
- Startup: `src/codegen/engine/function/entry.rs` (`lower_program_entry`),
  `src/docs/spec/memory/08_program-startup.md`.
- Arena: `src/codegen/memory/arena/arena.rs` (`lower_arena_alloc`, `lower_arena_destroy`).
- Linux backends: `src/target/linux_aarch64`, `src/target/linux_x86_64`,
  `src/target/linux_riscv64`, sharing `src/target/linux_common`; import planning in
  `src/target/linux_common/plan.rs` (`LinuxPlan::entry_imports`).
- ELF writer: `src/os/linux/link/mod.rs`, `src/os/linux/link/elf.rs`.
- Error codes: `src/docs/spec/diagnostics/02_error-codes.md`.
- Prior art: `planning/completed/mussings-asm.md` issue 9 (`-unsafe` and `-baremetal` were
  considered orthogonal).
