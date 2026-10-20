# Static Versus Dynamic Output

Linux selects a static or dynamic ELF encoder according to whether the image
has imports. Mach-O and PE use one container writer each, adding import metadata
when imports are present.
[[src/os/linux/link/mod.rs:encode_executable_bytes]]
[[src/os/macos/link/macho.rs:encode_unsigned_mach_o]]
[[src/os/windows/link/mod.rs:write_executable]]

## No imports — static image

Without imports, each format omits its library-import metadata:

- macOS: `encode_unsigned_mach_o` still emits `__PAGEZERO`/`__TEXT`/`__LINKEDIT`
  and a (possibly empty) symbol table, with no `LC_LOAD_DYLIB`, no
  GOT, and no dyld bind opcodes. Read-only constants or initializers can still
  require `__DATA_CONST`; initializers require rebase opcodes. `LC_LOAD_DYLINKER`
  remains present even without imports.
- Linux: `encode_static_elf` (AArch64/RISC-V) and `encode_static_elf_x86` each emit
  two `PT_LOAD`s — text (R+X) and a writable data segment — with no `PT_INTERP` and
  no `.dynamic` section. Data begins at `align(TEXT_FILE_OFFSET + text.len(),
  PAGE_SIZE)`, the same address `write_executable` patches data relocations
  against, and is writable because the entry stores `_mfb_rt_main_arena` into it.
  [[src/os/linux/link/elf/mod.rs:encode_static_elf]]

- Windows: the PE writer omits `.idata`, import thunks, and the Import/IAT
  directory entries, while retaining `.reloc` and `.mfbnote`.

The static ELF has no interpreter. An import-free Mach-O still names dyld and is
ad-hoc code-signed (see `macos-aarch64`). Like every emitted executable it also
carries the `MFBasic\0` provenance marker, which is independent of imports (see
`provenance-marker`).

## Imports present — dynamic image

For ELF and Mach-O, imports require:

- a dynamic dependency record per distinct imported library
  (`LC_LOAD_DYLIB` / `DT_NEEDED`),
- dynamic symbol and string tables covering every imported symbol,
- relocations that let the loader fill GOT entries
  (Mach-O bind opcodes / ISA-specific ELF jump-slot and data relocations),
- one import stub per encoded import record for generated calls to branch to,
- on Linux, the interpreter (`PT_INTERP`) and a `PT_DYNAMIC` segment.

Windows instead emits DLL import descriptors, name tables, ILTs, and IATs in
`.idata`, with a thunk for each function import (see `windows-x86_64`).

## Initializers

Mach-O emits `__mod_init_func` whenever `initializers` is non-empty, forcing a
`__DATA_CONST` segment even without imports. Dynamic ELF emits the corresponding
array and dynamic tags; static ELF and PE do not. The normal encoder leaves this
list empty. The format-specific execution contract is in
./mfb spec linker symbols-and-relocations.

## Entry imports and build modes

macOS imports `_signal` in both console and app mode. Console startup installs
SIGINT/SIGTERM handlers; both modes ignore SIGPIPE process-wide. Linux likewise
imports `signal` in both modes. macOS entry also imports `_exit` and
`_clock_gettime` (the arena start time); Linux's entry-import policy depends on its ISA's raw-syscall
support, as described in `import-selection`.
[[src/target/macos_aarch64/plan.rs:entry_imports]]
[[src/target/linux_common/plan.rs:entry_imports]]
[[src/codegen/engine/function/entry/mod.rs:lower_program_entry]]

Each backend validates its build mode: macOS accepts console/MacApp, Windows
console/WindowsApp, Linux aarch64/x86-64 console/LinuxApp, and RISC-V console only.
[[src/target/macos_aarch64/mod.rs:write_executable]]
[[src/target/win_x86_64/mod.rs:lower_validated_module]]
[[src/target/linux_aarch64/mod.rs:write_executable]]
[[src/target/linux_x86_64/mod.rs:write_executable]]
[[src/target/linux_riscv64/mod.rs:lower_validated_module]]

## No silent omission

The linker must not silently omit a required library, stub, or GOT slot. A
missing import, an unbacked external relocation, or an unsupported relocation is a
linker or codegen error, never a zero address or a placeholder (see
`failure-rules`).

## Section header table

The ELF writers emit a section header table only when something needs one;
the loader never reads it. All three writers end with one shared tail, and a
file has exactly one table, so signing and `--debug` share it.
[[src/os/linux/link/elf/sections.rs:append_elf_sections]]
[[src/os/linux/link/elf/sections.rs:append_section_table]]

| Build | Table |
| --- | --- |
| unsigned, no `--debug` | none: `e_shoff = 0`, `e_shnum = 0` |
| signed, no `--debug`, static | `NULL`, `.mfbsign`, `.shstrtab` |
| signed, no `--debug`, dynamic | `NULL`, `.dynstr`, `.dynamic`, `.mfbsign`, `.shstrtab` |
| `--debug` | `NULL`, `.text`, (`.rodata`), `.data`, (`.dynstr`, `.dynamic`), `.symtab`, `.strtab`, (`.mfbsign`), `.shstrtab` |

`e_shentsize` is 64 and `e_shstrndx` names `.shstrtab`, always the last entry.
Sections that describe loaded bytes point at them in place: `.text` at file
offset `0x1000` (its size includes the PLT stubs appended after the encoder's
text), and `.data` at the page-aligned data offset. The dynamic writer, which maps
the read-only data prefix as its own `PT_LOAD`, splits that prefix off as
`.rodata` (`SHF_ALLOC`); the static writers map all data writable and emit only
`.data` (`SHF_ALLOC | SHF_WRITE`). Every other body (`.symtab` 8-aligned,
`.strtab`, `.mfbsign` 8-aligned, `.shstrtab`) is appended past the last
`PT_LOAD`, then the 8-aligned header table follows. A signed build's seal runs
after the table is written, so the content signature covers it.

A dynamic image's table always describes `.dynstr` and `.dynamic` (`SHT_DYNAMIC`,
`sh_link` → `.dynstr`, `sh_entsize` 16), covering exactly `PT_DYNAMIC`: once an
ELF has section headers, readers look for the section that covers the dynamic
segment, and GNU readelf reports `no .dynamic section in the dynamic segment`
without it. No other part of the dynamic payload (`.dynsym`, `.rela`, `.got`,
the PLT) gets a header.

### Debug symbols

`.symtab` (`SHT_SYMTAB`, `sh_entsize` 24, `sh_link` → `.strtab`) holds a zero
entry, then one entry per symbol of the shared debug model (see
`./mfb spec linker symbols-and-relocations`), in its order. Every entry is
`STB_LOCAL`, so `sh_info` (one past the last local) is the entry count. Each entry
has symbol type `STT_FUNC` (text) or `STT_OBJECT` (data) in `st_info`, `st_other = 0`, `st_size`
from the model, and `st_value` from the relocation rule with no read-only window:
text is `text_vmaddr + offset`, data is `data_vmaddr + offset`. `st_shndx` is
`.text` for a function; for an object, `.rodata` when the offset is below the
read-only prefix and `.rodata` exists, otherwise `.data`.
[[src/os/linux/link/elf/sections.rs:symtab_bytes]]

Addresses are link-time ones. A dynamic image is `ET_DYN` at base 0, so tools add
the load slide; a static image is `ET_EXEC` at `0x400000`. gdb uses ELF names
verbatim, leading underscore included (`break _mfb_fn_main`).

## See Also

* ./mfb spec app macos-runtime — the window-driven finish path and AppKit
  bootstrap.
