# Windows x86-64

The `windows-x86_64` backend writes a PE32+ (`AMD64`) console `.exe` directly,
with no host linker (`link.exe`, `lld-link`, `ld`, or any toolchain). The compiler
lowers the program into an `EncodedImage` — the same type the ELF and Mach-O
writers consume — and this backend binds it into a finished executable image and
writes it to disk itself. Console builds emit one file, no flavor suffix, inside
the project's `build/` directory: [[src/os/windows/mod.rs:write_linked_executable]]

```text
build/<project>.exe
```

App-mode builds (`mfb build --app`) emit the same single `.exe` linked against the
GUI subsystem and carrying a `.rsrc` resource section (see **App mode** below). The
image is deterministic: encoding the same `EncodedImage` twice produces
byte-identical output. [[src/os/windows/link/mod.rs:write_executable]]

## Container layout

The image is a fixed-base PE32+ with no relocation table. Constants: image base
`IMAGE_BASE = 0x0001_4000_0000` (`link.exe`'s default x64 EXE base), section
alignment `0x1000` (4 KiB), file alignment `0x200` (512 bytes).
[[src/os/windows/link/pe.rs:IMAGE_BASE]]

Because the base is fixed, `IMAGE_FILE_RELOCS_STRIPPED` is set in the COFF
characteristics and `IMAGE_DLLCHARACTERISTICS_DYNAMIC_BASE` is left **clear** —
there is no `.reloc` section and the loader must map the image at `IMAGE_BASE`, so
every RVA the linker computes is a final virtual address. This is the PE analog of
the static ELF's `ET_EXEC` at a fixed base; unlike Linux, no ASLR variant ships.
[[src/os/windows/link/pe.rs:write_image]]

The file opens with a 64-byte `IMAGE_DOS_HEADER` plus a conventional 64-byte DOS
stub (the "This program cannot be run in DOS mode." bytes `link.exe` emits), so
`e_lfanew = 0x80`. The `PE\0\0` signature, COFF header, and PE32+ optional header
follow, then the section table, then each section body at its
`FileAlignment`-aligned `PointerToRawData`.

## PE headers

The COFF header is `Machine = 0x8664` (AMD64), `TimeDateStamp = 0` (determinism),
`SizeOfOptionalHeader = 240` (`0xF0`, asserted at emit), and
`Characteristics = 0x0023` (`EXECUTABLE_IMAGE | LARGE_ADDRESS_AWARE |
RELOCS_STRIPPED`).

The PE32+ optional header (`Magic = 0x020B`) carries:

```text
AddressOfEntryPoint  RVA of the entry symbol in .text
ImageBase            0x0001_4000_0000
SectionAlignment     0x1000     FileAlignment        0x200
MajorSubsystemVersion 6         MajorOperatingSystemVersion 6
Subsystem            3 (WINDOWS_CUI console) / 2 (WINDOWS_GUI app mode)
DllCharacteristics   0x8100 (NX_COMPAT | TERMINAL_SERVER_AWARE; DYNAMIC_BASE clear)
SizeOfStackReserve   0x0080_0000 (8 MiB)   SizeOfStackCommit 0x0010_0000 (1 MiB)
SizeOfHeapReserve    0x0010_0000           SizeOfHeapCommit  0x0000_1000
CheckSum             0 (determinism)       NumberOfRvaAndSizes 16
```

The 8 MiB stack reserve matches the worker-thread stacks; committing a full 1 MiB
up front (rather than the usual single guard page) means a function with a large
frame never skips the stack guard page, so this codegen emits no inline `__chkstk`
probe. Two derivation identities hold: `SizeOfHeaders == align(header_bytes,
0x200)` and `SizeOfImage == align(last_rva + last_vsize, 0x1000)`. Of the 16 data
directories only `[1]` Import and `[12]` IAT are non-zero (`[2]` Resource is
non-zero in app mode); `[5]` Base Relocation stays zero.
[[src/os/windows/link/pe.rs:write_image]]

## Sections

Sections are emitted in file order, zero-length sections omitted, each with its
COFF characteristics word:

```text
.text   0x6000_0020  CODE | EXECUTE | READ          program text + IAT thunks
.rdata  0x4000_0040  INITIALIZED_DATA | READ        read-only constants (rodata_size)
.data   0xC000_0040  INITIALIZED_DATA | READ | WRITE program constants + main-arena global
.idata  0xC000_0040  INITIALIZED_DATA | READ | WRITE import tables (the loader writes the IAT)
.rsrc   0x4000_0040  INITIALIZED_DATA | READ        app-mode resources (GUI builds only)
.mfbnote 0x4000_0040 INITIALIZED_DATA | READ        MFBasic provenance marker (always)
```

`.rdata` and `.data` come from one `image.data` blob split at `rodata_size`; they
are laid out contiguously so a data symbol's RVA is the same whichever partition
it lands in, and one `data_base_rva` serves both. A program with only writable
data emits `.data` and no `.rdata`, and vice versa.

`.mfbnote` is the sole **unconditional** section: it is placed last (after `.rsrc`
when present) and carries the `MFBasic\0` owner plus the shared 16-byte descriptor
so every `.exe` is identifiable as MFBASIC-produced. It has no data-directory
entry. See **./mfb spec linker provenance-marker**.

## Imports: `.idata` and the IAT thunk

When the image has imports the linker builds a single `.idata` section, in this
order: the import directory table (one 20-byte `IMAGE_IMPORT_DESCRIPTOR` per DLL
plus a zero-terminator descriptor), then per-DLL Import Lookup Tables (ILTs),
per-DLL Import Address Tables (IATs, byte-identical to the ILTs at emit time), the
hint/name table, and the DLL name strings. Each ILT/IAT entry is an import-by-name:
a `u64` holding the RVA of its hint/name entry with **bit 63 clear**; each table is
zero-terminated. Data directory `[1]` points at the descriptor table and `[12]` at
the first IAT. [[src/os/windows/link/mod.rs:build_idata]]

The loader overwrites each IAT slot with the resolved function address at load
time. To reach an imported function from `.text` without a load-time text fixup,
the linker appends one 12-byte thunk per `Function` import to `.text`:

```text
FF 25 <disp32>        jmp [rip + disp32]   ; disp32 = iat_slot_rva − (thunk_rva + 6)
CC CC CC CC CC CC     int3 padding to 12 bytes
```

The thunk jumps *through* the IAT slot, so it needs no relocation of its own — the
one indirection the loader already patches. A `Data`-kind import still occupies an
IAT slot but gets no thunk. [[src/os/windows/link/mod.rs:append_thunks]]

## Relocations

x86-64 references are RIP-relative `rel32` in a single instruction, so the linker
patches the disp32 field in place with `rel32 = target_rva − (site_rva + 4)`. The
`(binding, kind)` arms are: [[src/os/windows/link/mod.rs:patch_relocations]]

```text
internal call_pc32   -> defined symbol's RVA (text or data)
data     data_pc32   -> defined symbol's RVA
external call_pc32   -> the import's FF 25 thunk RVA   (the PE analog of a PLT stub)
external data_pc32 / got_pc32 -> the import's IAT slot RVA
```

An `external call` binds to the symbol's thunk, so a call site stays a plain
direct `call rel32` in `.text`; the built-in import surface is function-only
today, so the data arm exists for completeness. The entry symbol must resolve to
`.text`. Any other `(binding, kind)` pair, or a `rel32`/thunk displacement outside
the `±2 GiB` reach, is a hard link error (see **Failure rules**).

## Native `LINK` bindings and vendored DLLs

User `LINK` bindings are **not** ordinary imports (see **Import selection** — they
are a runtime `dlopen`-style load, not an `.idata` entry, so a missing library is
a catchable runtime error rather than a load-time start failure). Windows has no
`dlopen`, so `_mfb_linker_init` uses the Win32 loader: `LoadLibraryExA` in place
of `dlopen`, `GetProcAddress` in place of `dlsym`. These come from `kernel32.dll`
(plus `shlwapi.dll` for `PathRemoveFileSpecA`) as ordinary `.idata` imports,
declared by `win_x86_64::link_imports` only when the program has `LINK` bindings.

Windows also has no rpath, so a **vendored** library is located at run time rather
than through an image tag. The initializer builds the absolute path
`<exe_dir>\vendor\<name>` — `GetModuleFileNameA` → `PathRemoveFileSpecA` →
`lstrcatA "\vendor\"` → `lstrcatA <name>` into a writable scratch buffer
(`_mfb_linker_win_pathbuf`) — and calls
`LoadLibraryExA(path, NULL, LOAD_WITH_ALTERED_SEARCH_PATH)`, so the DLL and its
own dependencies resolve from the exe-relative `build/vendor/` directory. A
`system` locator is loaded by bare name (`LoadLibraryExA(name, NULL, 0)`, default
search). The scratch buffer and the `\vendor\` string are emitted only for a build
that vendors at least one library; a non-vendoring build is byte-identical to one
predating the feature. See `./mfb spec language native-libraries` for the
cross-platform vendor-search table. [[src/target/win_x86_64/code.rs:emit_link_dlopen]]

## Determinism

The image is reproducible: `TimeDateStamp` and `CheckSum` are zero, and imports
are grouped by DLL in **first-appearance order** over `image.imports`, never via
`HashMap` iteration — so the descriptor, ILT, IAT, and name-string order are
stable. Encoding the same `EncodedImage` twice yields identical bytes.
[[src/os/windows/link/mod.rs:group_imports_by_dll]]

## App mode

`mfb build --app` sets the GUI flag, which changes exactly two things versus a
console build: the PE `Subsystem` becomes `WINDOWS_GUI` (2) so the loader does not
attach a console, and a `.rsrc` section is emitted carrying the DPI-awareness
manifest (always), plus the app icon and version info when the build supplies them
(data directory `[2]` Resource points at it). A console build emits no `.rsrc` and
is byte-identical to a build with no app resources. The `.exe` is a single file in
both modes — there is no `.app`/AppImage bundle equivalent on Windows.
[[src/os/windows/link/rsrc.rs:build_rsrc]]

## Object plan and failure rules

`NativeObjectPlan::lower_plan` emits a `container: "pe"`, `image_base:
0x1_4000_0000` `.nobj` plan whose `validate` accepts only
`target == "windows-x86_64"` — the structural gate the ELF/Mach-O plans have.
[[src/os/windows/object.rs:validate]]

The linker fails rather than emit a broken executable:

- The entry symbol not resolving to `.text` is
  `entry symbol '…' does not resolve to text`.
- An external call with no thunk is
  `windows linker cannot bind external call '…' from …`; the data variant is
  `windows linker cannot bind external data '…' from …`.
- An unsupported `(binding, kind)` relocation pair is
  `windows linker does not support relocation … …`.
- A relocation offset past the end of `.text` is
  `windows linker: relocation offset … out of range`; a displacement beyond
  `±2 GiB` names the overflowing `rel32`/IAT-thunk delta.

[[src/os/windows/link/mod.rs:patch_relocations]] [[src/os/windows/link/mod.rs:write_executable]]

## See Also

* ./mfb spec linker symbols-and-relocations — the neutral relocation kinds and
  import/GOT bindings this backend realizes as thunks and IAT slots
* ./mfb spec linker static-and-dynamic-output — the import-free vs. imports-present
  image shapes
* ./mfb spec linker import-selection — the per-call `(library, symbol)` mapping
* ./mfb spec architecture x86_64-instruction-set — the x86-64 encoder these
  relocations patch
