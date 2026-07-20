# plan-47-C: PE/COFF executable writer

Last updated: 2026-07-19
Effort: medium (1h–2h)
Depends on: **per phase.** *C1* = Phases 1–3 (`src/os/windows/{mod,object,link/}` plus
one line of `src/os/mod.rs`) depends on **nothing** — `object.rs::validate` compares the
target as a *string*, not against the registry, and the `ExitProcess(42)` test image is
hand-built in `link/tests.rs`. *C2* = Phase 4 (wire `write_executable` to the backend)
depends on plan-47-B.

**C1 is the only unit in the whole feature that touches zero shared code** — it is a new
leaf sibling of `src/os/{linux,macos}/`. Land it early, in parallel with 47-A/E1/F1/G1.

Add a third container writer beside the ELF and Mach-O ones: `src/os/windows/`,
emitting a minimal **PE32+** console executable from the same `EncodedImage` the
other two consume. Scope is the *format*, not the OS surface — headers, section
table, the `.idata` import directory + Import Address Table, and the relocation
patcher that binds an external call to an IAT slot. No `CodegenPlatform` work
(that is 47-D); this sub-plan proves the container by hand-building one image.

The single behavioral outcome: an `EncodedImage` whose entry function calls
`ExitProcess(42)` through a one-entry `kernel32.dll` IAT is written by
`os::windows::write_linked_executable` to a `.exe` that Windows (or Wine) loads
and exits `42` from — and whose header fields match a `dumpbin`-derived oracle
field-for-field.

References:

- `planning/plan-47-windows-x86_64.md` — the master design; §3 item 2 and §5 are
  this sub-plan's charter. Do not contradict it.
- `src/os/linux/link/mod.rs` — the writer this one parallels: the
  `write_executable` wrapper (`:56`), `encode_executable_bytes` (`:113`),
  `patch_relocations` (`:172`), `append_import_stubs` (`:397`), and the x86 PLT
  stub emitter (`:504`–`:522`).
- `src/os/linux/link/elf.rs`, `src/os/linux/link/tests.rs` (41 tests) — the
  header-encoder and byte-level test shapes to mirror.
- `src/os/linux/object.rs` — the `container:"elf"` object plan and its
  `validate` (`:183`); `src/os/macos/link/macho.rs` — the second precedent.
- `src/arch/aarch64/encode/mod.rs:17`–`:91` — the real `EncodedImage`,
  `EncodedSymbol`, `EncodedSection`, `EncodedRelocation`, `EncodedImport`,
  `ImportKind` definitions.
- `src/arch/x86_64/reloc.rs:24` (`reloc_kind`) and
  `src/arch/x86_64/encode/emitter.rs:694` (`"bl"`) — what the x86 encoder
  actually emits for a call. Read `:694`–`:720` before designing the IAT path.
- Microsoft **PE Format** specification (`IMAGE_NT_HEADERS64`,
  `IMAGE_OPTIONAL_HEADER64`, `IMAGE_SECTION_HEADER`, `IMAGE_IMPORT_DESCRIPTOR`),
  and `dumpbin /headers /imports` on a `link.exe`-produced console `.exe` as the
  development-time oracle.
- `AGENTS.md` (the STOP rule on tests/goldens) and `.ai/compiler.md` (§spec sync:
  the embedded spec must be updated in the same change).

## Prerequisites

**C1 (Phases 1–3) has no prerequisites** — it is a new leaf under `src/os/windows/` and
touches no shared code. That is why it can land in parallel with 47-A.

**C2 (Phase 4)** additionally requires:

| Must be true | Command | Status 2026-07-20 |
|---|---|---|
| plan-47-B has landed (a backend to wire the writer to) | `ls src/target/win_x86_64/` | **NOT MET** |
| The Win11 box answers (to run the produced `.exe`) | `ssh -p 2230 test@127.0.0.1 true` | **UNVERIFIED — run it** |

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run
> every row before you continue and again before you decide to stop. Never act on a
> status you did not just verify. **If you stop, report the status of every row**, not
> only the one that blocked you.


## 1. Goal

- A `src/os/windows/` sibling of `src/os/{linux,macos}/`: `mod.rs` (the public
  wrappers), `object.rs` (a `container:"pe"` `.nobj` plan), and
  `link/{mod.rs,pe.rs,tests.rs}` (the byte writer).
- `os::windows::write_linked_executable(project_dir, name, image)` writes
  `build/<name>.exe` — one file, no flavor suffix — from an `&EncodedImage`.
- The emitted image is a valid PE32+ console executable: `MZ` at 0, `PE\0\0` at
  `e_lfanew`, COFF `Machine = 0x8664`, optional-header `Magic = 0x20B`,
  `Subsystem = 3` (`WINDOWS_CUI`), `SectionAlignment = 0x1000`,
  `FileAlignment = 0x200`, 16 data directories, sections `.text` / `.rdata` /
  `.data` / `.idata` with correct characteristics words.
- Imports from `EncodedImage.imports`, grouped by DLL, materialized as a
  standard `.idata`: import directory table (one `IMAGE_IMPORT_DESCRIPTOR` per
  DLL + a zero terminator), per-DLL Import Lookup Table and Import Address Table
  (parallel null-terminated `u64` arrays), and a hint/name table. Data
  directories `[1]` (Import) and `[12]` (IAT) point at them.
- Every `EncodedRelocation` the x86 encoder can produce is bound: `internal`
  `call_pc32`, `data` `data_pc32`, and `external` `call_pc32` (→ an IAT slot).
  Anything else is a hard error naming the binding and kind, never a silent
  skip — the discipline `src/os/linux/link/mod.rs:378` already uses.
- **Runtime proof:** a hand-built `ExitProcess(42)` image runs on Windows
  x86-64 / Wine and exits `42`.

### Non-goals (explicit constraints)

- **No change to any existing target's output bytes.** ELF and Mach-O writers,
  `src/arch/x86_64/**`, and every golden are untouched. `scripts/artifact-gate.sh`
  must show all four existing targets byte-identical.
- **No `CodegenPlatform` / `NativePlanPlatform` work.** No `emit_write`,
  no `GetCommandLineW` entry path, no arena over `VirtualAlloc` — those are 47-D.
  This sub-plan's only "program" is a hand-written test image.
- **No compiler-driven end-to-end build.** `mfb build -target windows-x86_64` is
  not expected to produce a working `.exe` at the end of 47-C; the backend stays
  `executable: false` until 47-D. The writer is reached from tests and from a
  47-D-era `write_executable`.
- **No external toolchain in the shipped `mfb`.** `link.exe`, `dumpbin`, `clang`,
  and MSVC are **development-time oracles only** — their outputs are transcribed
  into test constants by a human, never invoked by a build or by `cargo test`.
- **No `.reloc` / ASLR.** Fixed `ImageBase`, `IMAGE_FILE_RELOCS_STRIPPED` set,
  `DYNAMIC_BASE` clear. Data directory `[5]` stays zero. (Master §5 decision.)
- **No exports, no resources, no TLS directory, no debug directory, no
  Authenticode.** `EncodedImage.signing_metadata` and `EncodedImage.rpaths` have
  no PE analog in this sub-plan; `rpaths` must be *rejected* rather than silently
  dropped (see §7).
- **No `.bss`.** `EncodedImage.data` is fully materialized in the file, as it is
  for ELF; `SizeOfUninitializedData` stays 0.
- **Determinism is a constraint, not a nicety.** `TimeDateStamp = 0`,
  `CheckSum = 0`, and all import grouping/ordering derived from
  `image.imports` order — never from a `HashMap` iteration (bug-87 landed a
  regalloc tie-break for exactly this class of nondeterminism).

## 2. Current State

**Baseline note (2026-07-20):** the claims below were read at `c39c2bc3d`, which is now
25 commits behind HEAD. A re-audit found them substantially accurate (26 verified, 5
line-drifts) — the drifts are recorded in §Corrections. Re-read before relying on any
single line number. Every claim below was read in the tree at `c39c2bc3d`.

**The linkable-image type is shared and OS-neutral.** `EncodedImage`
(`src/arch/aarch64/encode/mod.rs:17`) carries `text`, `data`, `rodata_size`
(`:24` — the page-aligned read-only prefix of `data`, bug-187), `symbols`,
`relocations`, `imports`, `entry`, `initializers`, `signing_metadata`, `rpaths`.
The x86 encoder re-exports these verbatim rather than redeclaring them
(`src/arch/x86_64/encode/mod.rs:42`). The ELF writer takes `&EncodedImage`
(`src/os/linux/link/mod.rs:56`), and so will the PE writer.

**Writers are parallel siblings, not a trait.** `src/os/mod.rs:3`–`:6` declares
exactly `icon`, `linux`, `macos`, `note` — there is no container abstraction to
implement, and adding `pub(crate) mod windows;` is the whole wiring. Each of
`src/os/{linux,macos}/mod.rs` exposes the same three wrappers
(`write_native_object_plan`, `validate_native_object_plan`,
`write_linked_executable`) over a private `object` + `link` pair. Both write into
`project_dir.join(BUILD_DIR)` (`src/os/mod.rs:15`); Linux emits
`build/<name>-<flavor>.out` (`src/os/linux/link/mod.rs:70`), macOS
`build/<name>.out` (`src/os/macos/mod.rs:137`).

**The ELF writer's shape, which the PE writer mirrors.**
`encode_executable_bytes` (`src/os/linux/link/mod.rs:113`) does, in order:
clone `text`; resolve the entry symbol and require it be in `Text` (`:120`–`:126`);
append import stubs when `imports` is non-empty (`:127`–`:131`); compute the data
segment offset (`:132`); `patch_relocations` (`:134`); pick a container shape.
`patch_relocations` (`:172`) is a flat `match (binding, kind)` with an explicit
error arm (`:378`). `ImportLocations` (`:389`) is two `HashMap<String, u64>` —
`stubs` (call target per symbol) and `got_entries` (GOT slot per symbol) —
populated in `image.imports` order by `append_import_stubs` (`:397`), which
reserves a **fixed 12-byte slot per import** so the layout math is
arch-independent (`:405`).

**The x86 PLT stub already exists and is exactly the shape a PE IAT thunk
needs.** `emit_import_stub` for `arch == "x86_64"`
(`src/os/linux/link/mod.rs:504`–`:522`) emits `FF 25 disp32` — `jmp
*disp32(%rip)` — jumping through the GOT slot, `disp32` relative to `stub + 6`,
padded to 12 bytes with `0xCC`. Swap "GOT slot" for "IAT slot" and this is a PE
import thunk verbatim.

**The x86 encoder emits a DIRECT `call rel32` for an external call — it has no
indirect form.** This is the master's flagged question (§5), and the answer is
*no*. `"bl"` (`src/arch/x86_64/encode/emitter.rs:694`–`:720`) chooses between two
byte sequences on a **name-prefix heuristic**, `target.starts_with("_mfb_")`
(`:706`):

- internal → `vec![0xE8, 0, 0, 0, 0]`, disp field at 1 (`:708`);
- external → `vec![0xB8, 8, 0, 0, 0, 0xE8, 0, 0, 0, 0]`, disp field at 6
  (`:710`) — a `mov eax, 8` SysV variadic-vector-count marker followed by
  `E8 rel32`.

Both record `RelocIntent::Call` (`:717`), which `reloc_kind`
(`src/arch/x86_64/reloc.rs:26`) maps to `"call_pc32"`, and `record_reloc`
(`src/arch/x86_64/encode/emitter.rs:138`–`:159`) binds `internal` when the target
is a defined symbol and `external` when it is in `self.imports`. **There is no
`FF 15` (`call [rip+disp32]`) encoding anywhere in `src/arch/x86_64/`** — grep for
`0xFF` finds only `blr` (`call r/m64`, register-indirect, `:721`–`:731`).

Two consequences that shape §5's design:

1. The form choice happens in `encode_instruction`, a free function with **no
   access to the encoder's import map**; `self.imports` is only consulted later,
   in `emit_instruction`/`record_reloc` (`:107`, `:147`). The one existing
   import-driven rewrite (`0x8D` `lea` → `0x8B` `mov`, `:101`–`:111`) is
   deliberately **size-preserving** for this reason.
2. Sizing is not a separate table: `instruction_size`
   (`src/arch/x86_64/encode/sizing.rs:11`) calls the same `encode_instruction`
   and returns `bytes.len()`, and `encode` (`src/arch/x86_64/encode/mod.rs:83`–
   `:93`) uses it to lay out every text symbol *before* emission. So any change
   that makes an external call a different **length** (5+5 bytes → 6) cannot be a
   post-hoc patch; it must be threaded into `encode_instruction` and therefore
   into `instruction_size`, i.e. a new `encode_with_abi`-style entry point.

**The `"windows"`-rejection test the master points at is at a different line and
means something else.** `src/os/linux/mod.rs:141`–`:146`
(`write_native_object_plan_propagates_lowering_error`, not `:87`) sets
`plan.target = "windows"` and asserts the **Linux** object-plan lowering fails.
Its real subject is `NativeObjectPlan::validate` (`src/os/linux/object.rs:183`),
which accepts only `linux-aarch64` / `linux-x86_64` / `linux-riscv64` (`:186`)
and only `container == "elf"` (`:195`). `src/os/macos/mod.rs:116`–`:122` is the
exact mirror, rejecting `linux-aarch64`. **This test must stay and must keep
passing**: it is the guard that a Windows plan can never be lowered into an ELF
container. Nothing about it changes; the Windows path gets its own sibling test.

**Standing gates.** `AGENTS.md` forbids editing a test or golden to fit a change
without proof. `.ai/compiler.md:21` makes updating the embedded `mfb spec` part of
the same change as the compiler work, and `:67` requires
`scripts/test-accept.sh target/debug/mfb target/accept-actual` after any change
that can affect generated binaries. The embedded spec is auto-discovered by
`build.rs` — `src/docs/spec/mod.rs:39`–`:42` states a new topic file needs no code
edit, only a brand-new *package* does. `src/docs/spec/linker/` currently holds
`01`–`12`, so the next free number is **`13_windows-x86_64.md`**.

## 3. Design Overview

Four layers, landing in increasing-risk order. Nothing here touches
`src/arch/**` or either existing writer.

1. **`src/os/windows/object.rs` — the `container:"pe"` object plan.** A structural
   pre-flight gate mirroring `src/os/linux/object.rs`: the same
   `NativeObjectPlan` field set, `container: "pe"`, `validate` accepting only
   `target == "windows-x86_64"`. Zero risk (JSON only, not consumed by the
   writer), and it gives the backend something to answer `-nobj` with.

2. **`src/os/windows/link/pe.rs` — headers and section table.** Pure layout
   arithmetic over `(text, data, idata)` byte blobs: DOS header + stub, PE
   signature, COFF header, PE32+ optional header, N section headers. The whole
   file is one deterministic function of three lengths and the entry RVA, so it
   is testable without any relocation machinery.

3. **`src/os/windows/link/mod.rs` — imports, thunks, relocations.** Build
   `.idata` from `image.imports`, append one IAT thunk per imported function to
   `.text`, then run the relocation patcher. This is where the correctness risk
   concentrates.

4. **Wiring + proof.** `src/os/mod.rs` gains `pub(crate) mod windows;`; the
   47-B backend's `write_executable` calls through; the hand-built
   `ExitProcess(42)` image is run on Windows/Wine.

**The external-call answer (master §5, resolved).** The encoder emits
`B8 08 00 00 00 E8 rel32` for an external call
(`src/arch/x86_64/encode/emitter.rs:710`) — a *direct* `rel32`. PE has no PLT, but
nothing stops **this linker from synthesizing one**: for each imported function,
append a 12-byte thunk to `.text` — `FF 25 disp32` (`jmp [rip+disp32]`) targeting
that symbol's IAT slot, `disp32` relative to `thunk + 6` — and resolve the
`external` `call_pc32` `rel32` to the thunk. This is byte-for-byte the code
`src/os/linux/link/mod.rs:504`–`:522` already emits for the ELF GOT, and it reuses
`append_import_stubs`' proven fixed-slot layout discipline. The result is one
extra `jmp` per OS call, which is precisely what a PLT costs on Linux today.

*Rejected: teaching the encoder `FF 15` (`call [rip+disp32]`) in 47-C.* It is the
"purer" PE form and saves an indirection, but (a) the form is chosen in
`encode_instruction` on a name prefix with no import map in scope
(`emitter.rs:706`), (b) `instruction_size` is derived from that same function
(`sizing.rs:11`) and drives symbol layout (`mod.rs:83`), so a length change
demands a new ABI-parameterized encoder entry point — i.e. it belongs with 47-B's
`X86Abi` threading, not in the container writer, and (c) it would put a byte-level
change inside `src/arch/x86_64/` in the one sub-plan whose non-goal is "no
existing target's bytes move". Deferred to a follow-up once `X86Abi` exists; the
thunk is correct and shippable in the meantime. **Guard:** 47-C adds a test
pinning the current external `bl` bytes, so if a later change makes the call
indirect, the thunk assumption fails loudly instead of producing a `.exe` that
jumps to a thunk address as if it were a function pointer.

*Rejected: merging `.idata` into `.rdata`.* MSVC does this and the loader
supports it, but a separate, writable `.idata` makes the IAT's writability
explicit and the layout trivially inspectable in tests. Revisit only if a tool
complains.

*Rejected: a shared ELF/Mach-O/PE container trait.* Master §3 already rejected it;
restated here so the implementer doesn't re-litigate. PE joins as a third sibling.

*Rejected: `.reloc` + `DYNAMIC_BASE` in 47-C.* Master §5. Fixed base is the
simplest correct image. The consequence, which must be encoded deliberately: set
`IMAGE_FILE_RELOCS_STRIPPED` in COFF `Characteristics` and leave `DYNAMIC_BASE`
clear in `DllCharacteristics`, so the loader is *told* the image is base-fixed
rather than left to discover it.

## 4. PE32+ image layout (field-level)

All multi-byte fields little-endian. Offsets are file offsets unless marked RVA
(relative to `ImageBase`).

### 4.1 DOS header + stub

64-byte `IMAGE_DOS_HEADER`: `e_magic = 0x5A4D` (`MZ`) at 0; `e_lfanew` (u32) at
**0x3C** = file offset of the PE signature. Emit a conventional 64-byte DOS stub
(the `This program cannot be run in DOS mode.` real-mode program) so
`e_lfanew = 0x80`; every field between `e_magic` and `e_lfanew` may be zero
except `e_cblp`/`e_cp`/`e_cparhdr`/`e_maxalloc`/`e_sp`/`e_lfarlc`, which are
transcribed from the oracle. A stub-less image with `e_lfanew = 0x40` also loads,
but the stub costs 64 bytes and keeps every third-party PE tool happy.

### 4.2 PE signature + COFF file header

At `e_lfanew`: `0x00004550` (`"PE\0\0"`), then the 20-byte COFF header:

| Off | Field | Value |
|-----|-------|-------|
| +0  | `Machine` | `0x8664` (`IMAGE_FILE_MACHINE_AMD64`) |
| +2  | `NumberOfSections` | 2–4 (see §4.4) |
| +4  | `TimeDateStamp` | **0** (determinism) |
| +8  | `PointerToSymbolTable` | 0 |
| +12 | `NumberOfSymbols` | 0 |
| +16 | `SizeOfOptionalHeader` | `0xF0` (240) |
| +18 | `Characteristics` | `EXECUTABLE_IMAGE 0x0002` \| `LARGE_ADDRESS_AWARE 0x0020` \| `RELOCS_STRIPPED 0x0001` = `0x0023` |

### 4.3 Optional header (PE32+, 240 bytes)

Standard fields (24 bytes; PE32+ **omits** PE32's `BaseOfData`):

`Magic = 0x020B`; `MajorLinkerVersion`/`MinorLinkerVersion` (cosmetic, pin a
constant); `SizeOfCode` = section-aligned `.text` virtual size;
`SizeOfInitializedData` = sum of the other sections' virtual sizes;
`SizeOfUninitializedData = 0`; `AddressOfEntryPoint` = RVA of the entry symbol;
`BaseOfCode` = `.text` RVA.

Windows-specific fields (88 bytes):

| Field | Value |
|-------|-------|
| `ImageBase` (u64) | `0x0000_0001_4000_0000` — `link.exe`'s x64 EXE default |
| `SectionAlignment` | `0x1000` |
| `FileAlignment` | `0x200` |
| `MajorOperatingSystemVersion` / `Minor` | `6` / `0` |
| `MajorImageVersion` / `Minor` | `0` / `0` |
| `MajorSubsystemVersion` / `Minor` | `6` / `0` |
| `Win32VersionValue` | 0 |
| `SizeOfImage` | `align(last_section.rva + last_section.virtual_size, SectionAlignment)` |
| `SizeOfHeaders` | `align(e_lfanew + 4 + 20 + 240 + 40*N, FileAlignment)` |
| `CheckSum` | **0** (not required for an EXE; determinism) |
| `Subsystem` | `3` (`IMAGE_SUBSYSTEM_WINDOWS_CUI`) |
| `DllCharacteristics` | `NX_COMPAT 0x0100` \| `TERMINAL_SERVER_AWARE 0x8000` = `0x8100`; `DYNAMIC_BASE 0x0040` **clear** |
| `SizeOfStackReserve` / `Commit` (u64) | `0x100000` / `0x1000` |
| `SizeOfHeapReserve` / `Commit` (u64) | `0x100000` / `0x1000` |
| `LoaderFlags` | 0 |
| `NumberOfRvaAndSizes` | `16` |

Then 16 × 8-byte data directories `{VirtualAddress: u32, Size: u32}`. Indices
that matter here: **`[1]` Import** = (import-directory-table RVA, its byte size
including the zero terminator); **`[12]` IAT** = (first IAT RVA, total bytes of
all IATs); **`[5]` Base Relocation** = `(0, 0)`. Every other entry `(0, 0)`.
Note `[12]` is redundant for loading but is what a signer/loader uses to find the
region it must make writable — omitting it is a classic "loads fine today" bug.

`24 + 88 + 128 = 240 = 0xF0`, which is the `SizeOfOptionalHeader` asserted above;
a unit test should assert the emitted optional header is exactly 240 bytes rather
than trusting the arithmetic.

### 4.4 Section table and sections

40-byte `IMAGE_SECTION_HEADER` each: `Name` (8 ASCII bytes, NUL-padded),
`VirtualSize`, `VirtualAddress` (RVA), `SizeOfRawData`, `PointerToRawData`,
`PointerToRelocations = 0`, `PointerToLinenumbers = 0`,
`NumberOfRelocations = 0`, `NumberOfLinenumbers = 0`, `Characteristics`.

| Section | Contents | Characteristics |
|---------|----------|-----------------|
| `.text` | `image.text` + appended IAT thunks | `CNT_CODE 0x20` \| `MEM_EXECUTE 0x2000_0000` \| `MEM_READ 0x4000_0000` = `0x6000_0020` |
| `.rdata` | `image.data[..rodata_size]` | `CNT_INITIALIZED_DATA 0x40` \| `MEM_READ` = `0x4000_0040` |
| `.data` | `image.data[rodata_size..]` | `0x40` \| `MEM_READ` \| `MEM_WRITE 0x8000_0000` = `0xC000_0040` |
| `.idata` | import directory + ILTs + IATs + hint/name table | `0xC000_0040` (the loader **writes** the IAT) |

Layout rules, in this order:

- `VirtualAddress` is `SectionAlignment`-aligned and strictly increasing;
  the first section starts at `align(SizeOfHeaders, 0x1000)` = `0x1000`.
- `PointerToRawData` is `FileAlignment`-aligned; `SizeOfRawData` is the
  `FileAlignment`-aligned content length (`VirtualSize` is the *unaligned* one).
- **A zero-length section must be omitted, not emitted with size 0.** With
  `rodata_size == 0` there is no `.rdata`; with no imports there is no `.idata`.
  `NumberOfSections` follows. (`EncodedImage.rodata_size == 0` explicitly means
  "no read-only partition" — `src/arch/aarch64/encode/mod.rs:20`–`:24`.)
- `EncodedSection::Data` symbol offsets are into the *combined* `image.data`
  (`layout_data_objects` produced one blob), so if `.rdata` and `.data` are split
  across two sections their RVAs must remain contiguous *in the same
  `SectionAlignment` progression* — the resolver maps a `Data` symbol at offset
  `o` to `rdata_rva + o` when `o < rodata_size` and `data_rva + (o -
  rodata_size)` otherwise. Since `rodata_size` is page-aligned by construction,
  placing `.data` at `rdata_rva + rodata_size` keeps both formulas equal to
  `data_base_rva + o`; **assert that** rather than assume it.

### 4.5 `.idata`

Built once, in `image.imports` order, grouping by `library` on **first
appearance** (never via `HashMap` iteration — bug-87). Sub-blocks, in order:

1. **Import Directory Table** — one 20-byte `IMAGE_IMPORT_DESCRIPTOR` per DLL:
   `OriginalFirstThunk` (RVA of that DLL's ILT), `TimeDateStamp = 0`,
   `ForwarderChain = 0`, `Name` (RVA of the NUL-terminated DLL name),
   `FirstThunk` (RVA of that DLL's IAT) — followed by **one all-zero 20-byte
   descriptor** as the terminator. Data directory `[1].Size` includes it.
2. **Import Lookup Tables**, one per DLL: `u64` per import, then a `0` u64
   terminator. Import-by-name: bit 63 clear, bits 30..0 = RVA of the hint/name
   entry, bits 62..31 zero. (Import-by-ordinal — bit 63 set, low 16 bits the
   ordinal — is not used; `EncodedImport` has no ordinal field.)
3. **Import Address Tables**, one per DLL, **byte-identical to the ILTs at
   emission time**. The loader overwrites each entry with the resolved function
   address. Their RVAs are what the thunks and data directory `[12]` point at.
4. **Hint/Name table**: per import, `u16 Hint` (0) + the ASCII symbol name + NUL,
   **padded to an even offset**. Then the NUL-terminated DLL name strings.

The writer records, per imported symbol, its IAT slot RVA — the PE analog of
`ImportLocations.got_entries` (`src/os/linux/link/mod.rs:389`–`:395`).

### 4.6 IAT thunks and relocation patching

Two-stage, mirroring `encode_executable_bytes`
(`src/os/linux/link/mod.rs:113`–`:170`):

1. **Reserve.** Resolve the entry symbol; require `EncodedSection::Text`
   (`:120`–`:126`). If `imports` is non-empty, append **12 bytes per
   `ImportKind::Function` import** to `.text`. This is done before any layout
   arithmetic so `.text`'s size — and therefore every later section's RVA — is
   final. Because the thunk needs the IAT RVA and the IAT lives after `.text`,
   compute the whole section layout from *lengths* first, then fill bytes:
   `.text` length is known once the thunk count is (`imports.len() * 12`), and
   `.idata` length is a pure function of the import set.
2. **Emit each thunk** at `text_rva + offset`: `FF 25` then
   `disp32 = iat_slot_rva − (thunk_rva + 6)`, padded to 12 with `0xCC`. Use the
   `i32::try_from` reach check `src/os/linux/link/mod.rs:514`–`:518` already
   applies — a truncated `disp32` is a jump to a wrong address, exactly the
   silent-failure class bug-168 hardened the other backends against.
3. **Patch relocations**, a flat `match (binding, kind)` with a terminal error
   arm:
   - `("internal", "call_pc32")` → `rel32(symbol_va, site)`;
   - `("data", "data_pc32")` → `rel32(symbol_va, site)`;
   - `("external", "call_pc32")` → `rel32(thunk_va, site)`;
   - `("external", "got_pc32" | "data_pc32")` → **imported data global**: the
     instruction is `mov reg,[rip+disp32]` (the `0x8D`→`0x8B` rewrite,
     `src/arch/x86_64/encode/emitter.rs:101`–`:111`), so `rel32` targets the
     symbol's **IAT slot** directly, no thunk. Correct by construction, since a
     PE IAT slot holds the resolved address just as a GOT slot does. The floor
     imports no data globals, so this arm is provable only by unit test until a
     consumer exists — implement it anyway rather than erroring, and test it.
   - anything else → `Err("windows-x86_64 linker does not support relocation {binding} {kind}")`.
   `rel32` is `target − (site + 4)` where `site` is the address of the disp32
   field (`src/os/linux/link/mod.rs:599`–`:609`) — the PE writer needs its own
   copy rather than importing the Linux one, since the error text names the
   linker.

## 5. Why a wrong PE can look fine, and what gets oracle-diffed

The repo has been burned by exactly this shape before: a wrongly-linked musl
binary runs identically to a correct one (plan-56), so only `DT_NEEDED`
distinguishes them. PE has a longer list of fields the loader tolerates and other
consumers do not:

- `TimeDateStamp` non-zero — runs fine, makes every build nondeterministic.
- `CheckSum` zero/garbage — runs fine for an EXE, fails driver/signing paths.
- `SizeOfCode`, `SizeOfInitializedData`, `BaseOfCode`, linker-version bytes —
  advisory; wrong values load fine and mislead every inspection tool.
- `SizeOfImage` too large — loads fine (extra reserved pages); too small silently
  truncates the last section's mapping.
- Data directory `[12]` (IAT) omitted — loads fine via `[1]`, breaks signing and
  any loader that pre-protects the IAT region.
- `RELOCS_STRIPPED` clear with no `.reloc` — loads fine at the preferred base,
  fails the moment anything relocates the image.
- `.idata` marked read-only — loads fine on current Windows, is undefined.
- A thunk `disp32` truncated by more than ±2 GiB — assembles fine, jumps into
  hyperspace.

Therefore: **"it exits 42 on Wine" is necessary and not sufficient.** The
byte-level tests assert the following field set against constants transcribed from
`dumpbin /headers /imports` run **by a developer, once**, on a `link.exe`-built
console `hello.exe` — the same discipline plan-30-A applies with `otool -l`
(`planning/plan-30-A-ios-target.md:173`,`:177`). The test module records the exact
`dumpbin` command line beside the constants so they are reproducible, not
hand-guessed:

`e_magic`; `e_lfanew`; the `PE\0\0` bytes at `e_lfanew`; COFF `Machine`,
`SizeOfOptionalHeader`, `Characteristics`, `TimeDateStamp`; optional-header
`Magic`, `ImageBase`, `SectionAlignment`, `FileAlignment`,
`MajorSubsystemVersion`, `Subsystem`, `DllCharacteristics`,
`SizeOfStackReserve`/`Commit`, `SizeOfHeapReserve`/`Commit`,
`NumberOfRvaAndSizes`, `CheckSum`; the derivation identities `SizeOfHeaders ==
align(header_bytes, 0x200)` and `SizeOfImage == align(last_rva + last_vsize,
0x1000)`; data directories `[1]`, `[12]` non-zero and `[5]` zero; each section's
name, characteristics word, RVA alignment, and `PointerToRawData` alignment; the
import-descriptor field order plus the zero terminator; ILT/IAT `u64` name-RVA
encoding with bit 63 clear; hint/name entries at even offsets.

`dumpbin`/`link.exe` are never invoked by `cargo test`, by
`scripts/test-accept.sh`, or by a build. They produce numbers a human copies in.

## 6. Compatibility / Format Impact

- **New:** `src/os/windows/` (three modules + tests); `pub(crate) mod windows;`
  in `src/os/mod.rs`; a `container: "pe"` `.nobj` variant accepted only for
  `target == "windows-x86_64"`; a `build/<name>.exe` artifact name (single file,
  no flavor suffix); a new spec topic `src/docs/spec/linker/13_windows-x86_64.md`
  and a line in `src/docs/spec/linker/spec.md`'s reading order.
- **Unchanged:** `EncodedImage` and every sibling type (no new field —
  everything the writer needs is already there); `src/arch/**` (zero bytes move);
  `src/os/linux/**` and `src/os/macos/**`; the ELF/Mach-O `.nobj` schemas and
  their goldens; `src/os/linux/object.rs:186`'s Linux-target allowlist and the
  `"windows"`-rejection test at `src/os/linux/mod.rs:141` (both keep working
  exactly as they do — see §2).
- **Deliberately rejected inputs:** a non-empty `image.rpaths` and a non-`None`
  `image.signing_metadata` are hard errors from the PE writer ("windows-x86_64
  linker does not support …"), not silent drops. No Windows consumer produces
  either yet; failing loudly is what keeps a future `vendor`-using Windows build
  from shipping an executable that cannot find its DLLs.

## Phases

### Phase 1 — `container:"pe"` object plan + module wiring

The zero-risk half: a JSON structural plan and the module seam, with no image
bytes and nothing calling into the writer yet.

- [ ] Add `pub(crate) mod windows;` to `src/os/mod.rs` (beside `linux`/`macos`,
      `:3`–`:6`).
- [ ] New `src/os/windows/mod.rs`: `write_native_object_plan`,
      `validate_native_object_plan` wrappers mirroring
      `src/os/linux/mod.rs:19`–`:34`, writing `<name>.nobj`.
- [ ] New `src/os/windows/object.rs`: `NativeObjectPlan` with
      `container: "pe"`, `image_base: 0x1_4000_0000`, and a `validate` accepting
      only `target == "windows-x86_64"` and `container == "pe"` — the structural
      twin of `src/os/linux/object.rs:106`,`:183`.
- [ ] Tests: `src/os/windows/mod.rs` `#[cfg(test)]` module — the `.nobj` contains
      `"container": "pe"`; a `linux-x86_64` target is rejected (the mirror of
      `src/os/macos/mod.rs:116`); and a re-assert that
      `src/os/linux/mod.rs:141`'s `"windows"` rejection still holds unchanged.

Acceptance: `cargo test os::windows` green; the two existing
`write_native_object_plan_propagates_lowering_error` tests
(`src/os/linux/mod.rs:141`, `src/os/macos/mod.rs:116`) still pass untouched.
Commit: —

### Phase 2 — PE32+ headers and section table (no imports)

An import-less image: layout arithmetic only, fully unit-testable, byte-diffable
against the oracle constants.

- [ ] New `src/os/windows/link/pe.rs`: DOS header + 64-byte stub, PE signature,
      COFF header, PE32+ optional header (all §4.3 fields), and a section-header
      emitter — plus `align()` and `put_u16/u32/u64` helpers (the PE writer keeps
      its own copies; `src/os/linux/link/mod.rs:611`–`:644` is the shape).
- [ ] New `src/os/windows/link/mod.rs`: `write_executable(project_dir, name,
      image) -> PathBuf` writing `build/<name>.exe` via `BUILD_DIR`
      (`src/os/mod.rs:15`), and `encode_executable_bytes(image) -> Vec<u8>` doing
      entry resolution (`EncodedSection::Text` required), section planning
      (§4.4, omitting empty sections), and the `.text`/`.rdata`/`.data` payloads.
      Reject non-empty `rpaths` and `Some(signing_metadata)` with a named error.
- [ ] Add `write_linked_executable` to `src/os/windows/mod.rs`.
- [ ] Tests: new `src/os/windows/link/tests.rs`, mirroring
      `src/os/linux/link/tests.rs`'s hand-built-image style — an image whose
      `_main` is `mov ecx,42; ret` writes bytes starting `MZ`; `e_lfanew` reads
      `0x80` and `PE\0\0` sits there; every §5 header field matches its oracle
      constant; the optional header is exactly 240 bytes; `SizeOfHeaders` and
      `SizeOfImage` satisfy their alignment identities; a `rodata_size == 0`
      image emits **no** `.rdata` section and `NumberOfSections` agrees; a
      `rodata_size > 0` image emits `.rdata` before `.data` with contiguous RVAs
      and the correct characteristics words; an entry symbol resolving to
      `EncodedSection::Data` is an error.

Acceptance: `cargo test os::windows::link` green with every §5 header field
asserted against a transcribed oracle constant; `scripts/artifact-gate.sh` shows
all four existing targets byte-identical.
Commit: —

### Phase 3 — `.idata`, IAT thunks, relocation patching (highest-risk work)

Where the format risk concentrates: everything the loader must agree with.

- [ ] `src/os/windows/link/mod.rs`: build `.idata` per §4.5 — descriptors + zero
      terminator, ILTs, IATs, hint/name table — grouping by `library` in
      `image.imports` first-appearance order, and record each symbol's IAT slot
      RVA. Fill data directories `[1]` and `[12]`; assert `[5]` stays zero.
- [ ] Append one 12-byte `FF 25 disp32` + `0xCC` padding thunk per
      `ImportKind::Function` import (§4.6), reach-checked with `i32::try_from`
      exactly as `src/os/linux/link/mod.rs:514`–`:518` does.
- [ ] Add `patch_relocations` with the five arms of §4.6 and a terminal
      `Err(...)` arm naming binding + kind.
- [ ] Tests (`src/os/windows/link/tests.rs`): a two-DLL image
      (`kernel32.dll` + `bcrypt.dll`, three symbols) produces descriptors in
      first-appearance order with a zero terminator; ILT and IAT are
      byte-identical and both `0`-terminated; every ILT entry has bit 63 clear
      and its low 31 bits are the hint/name RVA; hint/name entries land on even
      offsets; each thunk's `disp32` resolves to its own IAT slot; an `external
      call_pc32` `rel32` lands on the thunk, an `external got_pc32` lands on the
      IAT slot directly; `internal call_pc32` and `data data_pc32` resolve to
      text/data symbol VAs; an unsupported `(binding, kind)` pair errors with
      both names in the message; encoding the **same** image twice produces
      byte-identical output (determinism).
- [ ] Tests (`src/arch/x86_64/encode/tests.rs`): pin the current external-`bl`
      bytes `[0xB8,8,0,0,0,0xE8,0,0,0,0]` (already asserted at `:470`) with a
      comment naming plan-47-C — the thunk design depends on that call staying
      **direct**; if it ever becomes `FF 15`, this test must fail before a `.exe`
      does.

Acceptance: `cargo test os::windows::link` green including the two-DLL import and
determinism cases; `dumpbin /imports` on a locally emitted test `.exe` lists both
DLLs with the expected symbol names.
Commit: —

### Phase 4 — Runtime proof + backend wiring + spec

- [ ] Wire the 47-B `src/target/win_x86_64/mod.rs` backend's `write_executable`
      and `write_native_object_plan` to `crate::os::windows::*`. The backend's
      `BackendCapabilities.executable` (`src/target.rs:94`) stays **false** —
      47-D flips it — so `crate::target::write_executable`'s gate
      (`src/target.rs:280`) keeps rejecting a real build with the existing
      "native executable output does not support …" message.
- [ ] Build the proof image by hand in a test-only helper: `.text` =
      `mov ecx, 42` + `call ExitProcess` (the encoder's external form) +
      `int3`, one `kernel32.dll` / `ExitProcess` import; write it to a temp dir
      and copy the `.exe` to a Windows/Wine runner.
- [ ] New `src/docs/spec/linker/13_windows-x86_64.md`: the PE emission contract —
      header field values, the section set and characteristics, the `.idata`
      layout, the synthesized IAT thunk (and why it exists), the fixed-`ImageBase`
      / `RELOCS_STRIPPED` decision, and the determinism rules. Use
      `[[src/os/windows/link/mod.rs:symbol]]` citations; `scripts/fix_citations.py`
      keeps them honest. No `src/docs/spec/mod.rs` edit is needed — a new *topic*
      is auto-discovered (`src/docs/spec/mod.rs:39`–`:42`).
- [ ] Add the topic to the reading-order list in
      `src/docs/spec/linker/spec.md` (the `macos-aarch64`, `linux-*` bullet) and
      to the "the two in-tree linkers" sentence, which becomes three.
- [ ] Update `.ai/compiler.md` / any `src/docs/**` list that enumerates targets or
      containers, if the grep finds one.

Acceptance: the hand-built `.exe` runs on Windows x86-64 (or Wine) and exits
**42**; `mfb spec linker windows-x86_64` renders the new topic; `cargo test`,
`scripts/test-accept.sh target/debug/mfb target/accept-actual`, and
`scripts/artifact-gate.sh` all green with existing targets byte-identical.
Commit: —

## Validation Plan

- **Tests.** `src/os/windows/link/tests.rs` is the primary surface, following
  `src/os/linux/link/tests.rs`'s hand-built-`EncodedImage` style (41 tests there;
  aim for comparable field coverage, not comparable count). Negative cases are
  mandatory, not optional: entry symbol not in `Text`; unsupported
  `(binding, kind)`; non-empty `rpaths`; `Some(signing_metadata)`; a thunk
  displacement beyond ±2 GiB; a duplicate DLL name preserving first-appearance
  grouping. Plus `src/os/windows/mod.rs`'s object-plan tests and the pinned
  external-`bl` byte test in `src/arch/x86_64/encode/tests.rs`.
- **Runtime proof.** The `ExitProcess(42)` `.exe`, executed on a Windows x86-64
  box or under Wine, exiting `42`. This is the one thing no unit test can stand in
  for — and, per §5, the one thing that is not sufficient on its own.
- **Oracle diff.** `dumpbin /headers` and `dumpbin /imports` on a
  `link.exe`-produced console `hello.exe`, run once by a developer, transcribed
  into named constants in `src/os/windows/link/tests.rs` with the command line
  recorded in a comment. Never invoked by a build or by `cargo test`.
- **Regression guard.** `scripts/artifact-gate.sh` after every phase — this
  sub-plan must not move a single byte of macos-aarch64, linux-aarch64,
  linux-x86_64, or linux-riscv64 output. If it does, something reached into
  `src/arch/**` that shouldn't have.
- **Doc sync.** `src/docs/spec/linker/13_windows-x86_64.md` + the reading-order
  and "two in-tree linkers" edits in `src/docs/spec/linker/spec.md`, in the same
  change as the code (`.ai/compiler.md:21`).
- **Acceptance.** `cargo test`; `scripts/test-accept.sh target/debug/mfb
  target/accept-actual`; `scripts/artifact-gate.sh`; `cargo fmt` (run a second
  pass — `repository/` is not a workspace member and needs its own); `cargo
  clippy --all-targets` clean, with no new blanket `#![allow(dead_code)]` (the
  tree has zero and stays that way — every item in `src/os/windows/` must have a
  caller or a test).

## Open Decisions

- **`ImageBase`** — recommend `0x140000000`, `link.exe`'s x64 EXE default, so a
  side-by-side `dumpbin` diff against the oracle has one fewer difference to
  explain. Alternative: `0x400000` (the 32-bit default, and what
  `src/os/linux/link/mod.rs:9` uses for ELF), which is equally loadable. (§4.3)
- **DOS stub** — recommend the conventional 64-byte real-mode stub
  (`e_lfanew = 0x80`) for tool compatibility. Alternative: no stub
  (`e_lfanew = 0x40`), 64 bytes smaller and still loadable. (§4.1)
- **IAT thunks vs. an indirect `call [rip+disp32]`** — recommend the synthesized
  12-byte thunk in `.text` for 47-C (no `src/arch/**` change, reuses the proven
  ELF stub bytes), and revisit the direct `FF 15` form as a follow-up once 47-B's
  `X86Abi` gives the encoder an ABI-parameterized entry point. (§3)
- **`.idata` separate vs. merged into `.rdata`** — recommend separate and
  read-write. Alternative (what MSVC does): merge into `.rdata`, which needs the
  loader's temporary-writability behavior. (§3, §4.4)
- **Where the `.exe` proof runs** — recommend Wine for 47-C's single
  `ExitProcess` case (no new infra, and this image touches nothing Wine emulates
  imperfectly), deferring the native-Windows-runner question to the master's
  open decision for the surfaces that need it. (§Validation)

## Summary

The engineering risk is entirely in the image layout, and it is the quiet kind: a
PE with a wrong `TimeDateStamp`, a missing IAT data directory, a stale
`SizeOfImage`, or a read-only `.idata` **loads and runs correctly today** and
breaks later, elsewhere, for someone else. So the plan front-loads the two
things that make that risk visible — a field-by-field oracle diff transcribed
from `dumpbin`, and a byte-level test module in the style
`src/os/linux/link/tests.rs` already proves works — and treats the "it exits 42"
runtime check as the last confirmation rather than the evidence.

The master's one open technical question is answered and closed: the x86 encoder
emits a **direct** `E8 rel32` for an external call
(`src/arch/x86_64/encode/emitter.rs:706`–`:711`) and has no `FF 15` form at all,
so 47-C synthesizes a PLT-equivalent thunk in `.text` — the same `FF 25 disp32`
bytes `src/os/linux/link/mod.rs:504`–`:522` already emits for the ELF GOT — and
leaves `src/arch/x86_64/` untouched.

Untouched: `EncodedImage` and every sibling type, both existing writers, the
whole x86-64 encoder, every existing target's bytes, the NIR/plan/MIR pipeline,
and the entire language layer. `src/os/linux/object.rs`'s Linux-only target
allowlist and the `"windows"`-rejection test that guards it keep working exactly
as they do today — the Windows plan gets its own container, not a widened ELF one.


## Corrections

<!-- Filled in during execution. -->

- 2026-07-20 — **B does not depend on 47-B** the way the header said. Phases 1–3 touch
  only the new `src/os/windows/` leaf and one line of `src/os/mod.rs`; only Phase 4 needs
  the backend. Split into C1 (blocks on nothing) and C2.
- 2026-07-20 — **A↔B conflict to settle once.** B Phase 3 adds a test pinning the
  external-call bytes `[0xB8,8,0,0,0,0xE8,0,0,0,0]` (`emitter.rs:710`). That `B8 08` is
  `mov eax,8` — the **SysV variadic vector-count marker** (documented at
  `emitter.rs:696-705`), meaningless on Win64. 47-B's proposed `CALL_ARGS_WIN64` makes
  `rax` an argument slot, so A introducing `X86Abi` is the natural moment to drop the
  marker for Win64 — which this new pinning test would forbid. The `internal` guard at
  `:706` saves it today. Decide in whichever of A/B lands first.
- 2026-07-20 — Line drifts found in a re-audit (claims correct, citations stale):
  `record_reloc` is `emitter.rs:130` (draft said `:138`); `emit_instruction` is `:82`
  (draft `:107`); the relocation error arm is `linux/link/mod.rs:380` (draft `:378`);
  `ImportLocations` is `:390` (draft `:389`); `instruction_size` is `sizing.rs:10`
  (draft `:11`); the linux `.out` naming fn is `linux/link/mod.rs:56` (draft `:70`).
- 2026-07-20 — **`src/os/{linux,macos}/mod.rs` expose 6 and 4 public fns, not "the same
  three wrappers"** (draft `:112`). The three named do exist in both; scope the PE
  sibling against the real surface.
- 2026-07-20 — **The draft's correction of the master was right and is preserved:** the
  `"windows"`-rejection negative test is `src/os/linux/mod.rs:141`
  (`write_native_object_plan_propagates_lowering_error`), not the master's `:87`, and it
  **must stay** — the master's Phase B said to drop it.
