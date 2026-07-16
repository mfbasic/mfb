# Linux riscv64

The `linux-riscv64` backend writes ELF64 RISC-V executables directly, with no host
linker (`ld`, `gold`, `lld`, `gcc`, `clang`). It is cross-compiled — a build on any
host produces the RV64 binary. Like the x86-64 backend, it **shares one ELF linker**
with the aarch64 Linux backend, parameterized by an `arch` string (`"riscv64"`
versus `"aarch64"`/`"x86_64"`). Only the target front-end and the per-ISA (RV64GC)
encoder are separate. [[src/os/linux/link/]] [[src/target/linux_riscv64/]] [[src/arch/riscv64/]] [[src/os/linux/link/elf.rs:encode_dynamic_elf]] [[src/target/linux_riscv64/plan.rs:target]]

A console build emits two flavors, one per dynamic loader / library naming, both
inside the project's `build/` directory:

```text
build/<project>-glibc.out
build/<project>-musl.out
```

Both worlds share every kernel struct layout the codegen bakes in (stat/dirent/
termios, pthread object sizes), so only the import library names and the
interpreter differ. The two-flavor emission loop lives in the target front-end, not
the linker. [[src/target/linux_riscv64/mod.rs:write_executable]]

App mode (`mfb build --app`) is **not supported** on this target — see below — so
every riscv64 build is a console build.

## Container layout

Constants are shared with the other Linux backends: image base `0x400000`, text
file offset `0x1000`, page size `0x1000` (4 KiB). The ELF header is `ET_EXEC`,
**`EM_RISCV` (machine 243)** (aarch64 is 183, x86-64 is 62), entry =
`text_vmaddr + entry_offset`. There is no `ET_DYN`/PIE path.
[[src/os/linux/link/mod.rs:IMAGE_BASE]] [[src/os/linux/link/elf.rs:e_machine]]

RISC-V additionally uses the ELF header's `e_flags` to declare its float ABI: a
dynamic riscv64 image sets **`EF_RISCV_FLOAT_ABI_DOUBLE` (`0x4`)** for the `lp64d`
ABI. The rv64 glibc/musl loader refuses a soft-float (`0x0`) executable, so this
must be set; x86-64 and aarch64 leave `e_flags` zero.
[[src/os/linux/link/elf.rs:e_flags]]

A dynamic image (imports present, `encode_dynamic_elf`) has the **same program
headers** as the other Linux targets — seven, or eight with a read-only constant
partition (see `linux-aarch64` for the list). Only the machine field, `e_flags`,
and the interpreter string branch on arch.

The **static** image carries **two** `PT_LOAD`s — a text segment (R+X) and a
separate page-aligned **writable** data segment (R+W) — because the program entry
writes `_mfb_rt_main_arena` and the data must be writable. It also carries
`PT_GNU_STACK` and the `PT_NOTE` provenance marker (see `provenance-marker`), for
four headers total. The aarch64/RISC-V static encoder is shared. Every console
build imports libc, so no console build produces a static image. [[src/os/linux/link/elf.rs:encode_static_elf]]

## Calling convention and ABI

The register-role model (`lp64d`: integer args in `a0`–`a7`, float args in
`fa0`–`fa7`, callee-save `s0`–`s11`, arena base in `s11`) is owned by
`./mfb spec memory native-calling-convention`. Only the link-relevant specifics
appear here:

- The return address lives in the **`ra` (`x1`) register**, not on the stack, so a
  16-aligned frame keeps `sp` 16-aligned at call sites with no per-call padding —
  like aarch64, unlike x86-64. [[src/arch/riscv64/backend.rs:frame_call_padding]]
- The image declares the double-float (`lp64d`) ABI in `e_flags`, as above.
- RV64GC has **no native 128-bit SIMD**, so `v128` vocabulary scalarizes to `2× f64`;
  this does not surface in the linked output beyond ordinary text. Hardware FMA is in
  the base `D` extension, so the transcendental kernels stay in-tree (no `libm`
  import). [[src/arch/riscv64/mod.rs]]

## Program entry

The program entry is a **raw Linux ELF entry**: the kernel jumps directly to the
`e_entry` address (the `entry` function, renamed `_main` at link time) with
`argc`/`argv`/`envp` on the initial stack, not in registers. There is no
`__libc_start_main` C-runtime bootstrap on the console path (that trampoline belongs
to the GTK app path, which is not ported to this target). The general entry sequence
— arena carve, hook install, language-entry dispatch, teardown — is owned by
`./mfb spec memory program-startup`. [[src/target/linux_riscv64/code.rs:entry_args_in_registers]] [[src/target/shared/code/entry_and_arena.rs:lower_program_entry]]

The entry imports `_exit` (teardown), `write` (the error-report path), `getentropy`
and `clock_gettime` (to seed the per-arena memory-fill RNG), and — in console mode —
`signal` (to install the `SIGINT`/`SIGTERM` handlers that run `_mfb_shutdown`). The
RNG seed draws from the `getentropy` libc import on riscv64 (x86-64 uses the
`getrandom` syscall instead). [[src/target/linux_riscv64/plan.rs:entry_imports]]

## Static vs dynamic output

The static-vs-dynamic choice is the shared Linux model owned by
`./mfb spec linker static-and-dynamic-output`: a build that imports nothing links to
a static, interpreter-less ELF; a build with imports links libc dynamically
(interpreter + `PT_DYNAMIC` + PLT/GOT). Every riscv64 console build links libc for
its POSIX surface, so in practice every shipped riscv64 binary is dynamic.

## Dynamic metadata

For a dynamic image the linker builds `.dynstr`, `.dynsym` (entry 0 is the null
symbol), a SysV `.hash` table, `.rela`, and `.got`, then a `.dynamic` section. The
dynamic tags are **identical to aarch64** (`DT_NEEDED` per distinct library,
`DT_HASH`/`DT_STRTAB`/`DT_SYMTAB`, the `DT_RELA`/`DT_JMPREL` set, `DT_PLTGOT`,
`DT_FLAGS_1 = DF_1_NODELETE`, `DT_NULL`). The general model is owned by
`./mfb spec linker symbols-and-relocations`; the only arch-specific values are the
relocation-type constants. [[src/os/linux/link/elf.rs:encode_dynamic_elf]]

Each imported function gets a **12-byte** import stub and an 8-byte GOT slot, with a
dynamic relocation in `.rela` binding that slot. RISC-V has **no dedicated
`GLOB_DAT`**: an imported data global's GOT slot is bound with an absolute
**`R_RISCV_64` (2)**, and an imported function's slot with **`R_RISCV_JUMP_SLOT`
(5)** (aarch64 uses 1025/1026, x86-64 uses 6/7). [[src/os/linux/link/mod.rs:R_RISCV_JUMP_SLOT]] [[src/os/linux/link/elf.rs:encode_dynamic_elf]]

The import stub loads the resolved address from the symbol's GOT slot and jumps to
it, using `t3` (`x28`) as the scratch register:

```text
auipc t3, %pcrel_hi(got_slot)   ; materialize the slot page
ld    t3, %pcrel_lo(got_slot)(t3) ; load the bound address
jalr  x0, 0(t3)                 ; tail-jump, no link
```

It is padded to the fixed 12-byte per-stub slot so the surrounding layout math is
arch-independent. [[src/os/linux/link/mod.rs:emit_import_stub]]

Symbol versioning (`.gnu.version`/`.gnu.version_r`) is emitted the same way as the
other Linux targets when an import carries a `version`; the encode path emits all
imports unversioned. [[src/os/linux/link/elf.rs:encode_dynamic_elf]]

## Relocations

Every RISC-V PC-relative reference is a **pair** of instructions — a hi20 that
materializes the upper 20 bits with `auipc`, and a lo12 that adds or loads the low
12 bits — so a data address or GOT load splits into `*Hi`/`*Lo` kinds exactly like
aarch64's `page21`/`pageoff12`. A call is the single `auipc; jalr` pair the linker
patches as one unit from the `auipc` site. The neutral `RelocIntent` → rv64 kind
mapping is: [[src/arch/riscv64/reloc.rs:reloc_kind]]

```text
Call        -> riscv_call        (auipc ra,%hi ; jalr ra,%lo(ra))
DataAddrHi  -> riscv_pcrel_hi20  (auipc rd,%pcrel_hi(sym))
DataAddrLo  -> riscv_pcrel_lo12  (addi  rd,rd,%pcrel_lo(sym))
GotLoadHi   -> riscv_got_hi20    (auipc rd,%got_pcrel_hi(sym))
GotLoadLo   -> riscv_got_lo12    (ld    rd,%pcrel_lo(sym)(rd))
```

The encoder tags each relocation with a `binding`: `internal`/`data` for a
reference to a defined symbol, `external` for an imported one. The linker's patch
pass resolves an external `riscv_call` to the import's stub, and an external
`riscv_got_hi20`/`riscv_got_lo12` pair to the symbol's GOT slot; internal `riscv_*`
kinds resolve to the final text/data address. Each displacement is reach-checked
against the `auipc` pair's ±2 GiB range and errors rather than silently truncating.
[[src/os/linux/link/mod.rs:patch_relocations]] [[src/os/linux/link/mod.rs:riscv_hi_lo]]

Because a lo12 completes the low 12 bits of the **paired `auipc`'s** displacement,
the linker computes it from that `auipc`'s address, not from `offset - 4`: the two
halves need not be adjacent, since the register allocator may spill `rd` between
them under pressure. The lo12's base is the nearest *preceding* hi relocation to the
same target. [[src/os/linux/link/mod.rs:paired_auipc_offset]]

## glibc flavor

```text
interpreter  /lib/ld-linux-riscv64-lp64d.so.1
libc.so.6        C/POSIX runtime functions
libpthread.so.0  pthread_create for thread::start
```

Each soname an import names becomes a `DT_NEEDED` entry; the per-call
`(library, symbol)` mapping is owned by `./mfb spec linker import-selection`.
Imported symbols use plain ELF names with no leading underscore. `libm.so` is **not**
needed — every `math::` transcendental, `pow`, `atan2`, `tan`, and `Float MOD`
(`fmod`) lowers to an in-tree kernel. [[src/target/linux_riscv64/plan.rs:runtime_imports]] [[src/target/linux_riscv64/plan.rs:native_call_imports]]

## musl flavor

```text
interpreter  /lib/ld-musl-riscv64.so.1
libc.musl-riscv64.so.1   C/POSIX runtime functions and pthread_create
```

musl exposes the pthread entry points from libc, so `pthread_create` (for
`thread::start`) is imported from `libc.musl-riscv64.so.1` rather than a separate
pthread library. As on glibc, `libm.so` is not needed. [[src/target/linux_riscv64/plan.rs:libpthread]]

## App mode

App mode (`mfb build --app`) is **rejected** for this target: the GTK4 toolkit
bootstrap has not been ported to rv64, so `supports_app_mode()` is false and the CLI
refuses `--app`. The app-only code paths (the GTK entry, the `__libc_start_main`
trampoline, the toolkit rodata) hard-stop rather than emit aarch64-convention code.
A build mode other than console — or the internal Linux-app mode — is rejected with
`Linux riscv64 native targets do not support the … build mode`.
[[src/target/linux_riscv64/mod.rs:supports_app_mode]] [[src/target/linux_riscv64/mod.rs:lower_validated_module]]

## Failure rules

- An unsupported `(binding, kind)` relocation pair is
  `linux linker does not support relocation … …`.
- An external symbol with no stub is
  `linux-riscv64 linker cannot bind external symbol '…' from …`; the data variant is
  `linux-riscv64 linker cannot bind external data symbol '…' from …`.
- A displacement past the `auipc` pair's range is
  `linux-riscv64 linker: displacement … exceeds the ±2 GiB reach of auipc`.

[[src/os/linux/link/mod.rs:patch_relocations]] [[src/os/linux/link/mod.rs:riscv_hi_lo]]

## Executable signing metadata

When the build supplies executable signing metadata, the linker emits it as a
`.mfb_sign` ELF section (shared with the other Linux targets). Unlike macOS, Linux
executables are not otherwise signed by the linker.
[[src/os/linux/link/elf.rs:append_elf_signing_section]]

## See Also

* ./mfb spec linker linux-aarch64 — the sibling aarch64 ELF backend this one shares
  its linker with
* ./mfb spec linker linux-x86_64 — the other sibling ELF backend
* ./mfb spec architecture riscv64-instruction-set — the RISC-V instruction repertoire and encoding this backend emits
* ./mfb spec linker import-selection — the per-call `(library, symbol)` mapping and
  flavor soname selection
* ./mfb spec linker symbols-and-relocations — relocation kinds, import stubs, and the
  GOT
* ./mfb spec linker static-and-dynamic-output — the static-vs-dynamic image choice
* ./mfb spec memory native-calling-convention — the RV64 `lp64d` register roles
* ./mfb spec memory program-startup — the program-entry sequence
