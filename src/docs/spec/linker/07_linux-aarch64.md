# Linux aarch64

The Linux backend is cross-compiled and writes ELF64 aarch64 executables
directly. It does not invoke `ld`, `gold`, `lld`, `gcc`, `clang`, or any host
linker. [[src/os/linux/link.rs]] [[src/os/linux/link/elf.rs:encode_dynamic_elf]]

A console build emits two flavors, one per dynamic loader / library naming:

```text
<project>-glibc.out
<project>-musl.out
```

An app-mode build (`mfb build -app`) emits a single glibc binary, `<project>.out`
(the `app_mode` flag selects single-output, glibc-only). Each flavor is planned
and linked independently from the same NIR, because the sonames it imports differ.

## Container layout

Constants: image base `0x400000`, text file offset `0x1000`, page size `0x1000`
(4 KiB). The ELF header is `ET_EXEC`, `EM_AARCH64`, entry =
`text_vmaddr + entry_offset`.

A dynamic image (imports present, `encode_dynamic_elf`) has seven program
headers, or eight when the image has a read-only constant partition:

```text
PT_PHDR      the program header table
PT_INTERP    the dynamic loader path
PT_LOAD      RX text (image base)
PT_LOAD      R  rodata (only if a constant partition is present)
PT_LOAD      RW data
PT_DYNAMIC   the .dynamic section
PT_GNU_STACK the non-executable-stack marker (all sizes 0)
PT_NOTE      the MFBasic provenance marker
```

A static image (no imports, `encode_static_elf`) has four: two `PT_LOAD`s — text
(R+X) and a writable data segment page-aligned to the `data_vmaddr` the
relocation patcher uses — plus `PT_GNU_STACK` and `PT_NOTE`, and no
`PT_INTERP`/`PT_DYNAMIC`. Every console build imports libc, so no current console
build produces a static image.

The `PT_NOTE` marker is unconditional and identical across every encoder and
arch; see `provenance-marker`.

## Dynamic metadata

For a dynamic image the linker builds `.dynstr`, `.dynsym` (entry 0 is the null
symbol), a SysV `.hash` table, `.rela`, and `.got`, then a `.dynamic` section
carrying at least:

The `.hash` table is one bucket over every imported symbol: `nbucket=1`,
`nchain=imports+1`, `bucket[0]=1`, then the chain — `chain[0]=0` for the null
symbol, `chain[i]` linking to symbol `i+1`, and `0` (`STN_UNDEF`) terminating it.
[[src/os/linux/link/elf.rs:encode_dynamic_elf]]

```text
DT_NEEDED (one per distinct imported library)
DT_HASH DT_STRTAB DT_SYMTAB DT_STRSZ DT_SYMENT
DT_PLTGOT DT_RELA DT_RELASZ DT_RELAENT DT_PLTREL DT_JMPREL DT_PLTRELSZ
DT_FLAGS_1 = 8 (DF_1_NODELETE)
DT_NULL
```

Each imported function gets a 12-byte stub and an 8-byte GOT slot, with a
relocation in `.rela`: `R_AARCH64_JUMP_SLOT` for `ImportKind::Function`,
`R_AARCH64_GLOB_DAT` for `ImportKind::Data` (addend always 0). External
`branch26` relocations are resolved to the stub; external `page21`/`pageoff12`
to the GOT slot. The linker emits one `DT_NEEDED` per distinct imported library.
[[src/os/linux/link/mod.rs:R_AARCH64_JUMP_SLOT]]

## Symbol versioning

When any import carries a `version`, the linker additionally emits `.gnu.version`
(`DT_VERSYM`) and `.gnu.version_r` (`DT_VERNEED`/`DT_VERNEEDNUM`): one `Verneed`
per library, one `Vernaux` per distinct `(library, version)` pair, with version
indices starting at 2 (1 = unversioned global). This is intended for versioned
exports such as OpenSSL 3's `OPENSSL_3.0.0`. The encode path emits all imports
unversioned, so a console/app build produces no `.gnu.version*` sections; the
versioning path is exercised by the linker tests (validated against the glibc
`GLIBC_2.17` aarch64 baseline). [[src/os/linux/link/elf.rs:encode_dynamic_elf]]

## Initializers

If the image carries `initializers`, the linker resolves each to its absolute
text address and emits a `.init_array` plus `DT_INIT_ARRAY`/`DT_INIT_ARRAYSZ`.
For a normal custom-entry MFBASIC binary the entry runs `_mfb_linker_init`
itself, so `initializers` is empty and no `.init_array` is produced.

## glibc flavor

```text
interpreter  /lib/ld-linux-aarch64.so.1
libc.so.6        C/POSIX runtime functions
libpthread.so.0  pthread_create for thread::start
```

Each soname an import names becomes a `DT_NEEDED` entry. The per-call
`(library, symbol)` mapping (e.g. `io::print`→`write` from `libc.so.6`,
`thread::start`→`pthread_create` from `libpthread.so.0`) is owned by ./mfb spec
linker import-selection. Imported symbols use plain ELF names with no leading
underscore. `libm.so` is **not** needed: every `math::` transcendental, `pow`,
`atan2`, `tan`, and `Float MOD` (`fmod`) lowers to an in-tree kernel.

## musl flavor

```text
interpreter  /lib/ld-musl-aarch64.so.1
libc.musl-aarch64.so.1   C/POSIX runtime functions and pthread_create
```

musl exposes the pthread entry points from libc, so the pthread surface
(`pthread_create` for `thread::start`) is imported from
`libc.musl-aarch64.so.1` rather than a separate pthread library. As on glibc,
`libm.so` is not needed — the `math::` kernels are in-tree. The per-call symbol
mapping is owned by ./mfb spec linker import-selection.

## Executable signing metadata

When the build supplies executable signing metadata, the linker emits it as a
`.mfb_sign` ELF section. Unlike macOS, Linux executables are not otherwise signed
by the linker.

## See Also

* ./mfb spec linker linux-x86_64 — the sibling x86-64 ELF backend
* ./mfb spec linker import-selection — the per-call `(library, symbol)` mapping
  and flavor soname selection
* ./mfb spec linker symbols-and-relocations — relocation kinds, import stubs, and
  the GOT
* ./mfb spec linker static-and-dynamic-output — the static-vs-dynamic image
  choice
