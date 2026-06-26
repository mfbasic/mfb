# Linux aarch64

The Linux backend (`src/os/linux/link.rs`) is cross-compiled and writes ELF64
aarch64 executables directly. It does not invoke `ld`, `gold`, `lld`, `gcc`,
`clang`, or any host linker.

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

A dynamic image (imports present, `encode_dynamic_elf`) has five program headers:

```text
PT_PHDR      the program header table
PT_INTERP    the dynamic loader path
PT_LOAD      RX text (image base)
PT_LOAD      RW data
PT_DYNAMIC   the .dynamic section
```

A static image (no imports, `encode_static_elf`) has a single `PT_LOAD` and no
`PT_INTERP`/`PT_DYNAMIC`.

## Dynamic metadata

For a dynamic image the linker builds `.dynstr`, `.dynsym` (entry 0 is the null
symbol), a SysV `.hash` table, `.rela`, and `.got`, then a `.dynamic` section
carrying at least:

```text
DT_NEEDED (one per distinct imported library)
DT_HASH DT_STRTAB DT_SYMTAB DT_STRSZ DT_SYMENT
DT_PLTGOT DT_RELA DT_RELASZ DT_RELAENT DT_PLTREL DT_JMPREL DT_PLTRELSZ
DT_FLAGS_1 = DF_1_PIE
DT_NULL
```

Each imported function gets a 12-byte stub and an 8-byte GOT slot, with a
relocation in `.rela`: `R_AARCH64_JUMP_SLOT` for `ImportKind::Function`,
`R_AARCH64_GLOB_DAT` for `ImportKind::Data` (addend always 0). External
`branch26` relocations are resolved to the stub; external `page21`/`pageoff12`
to the GOT slot. The linker emits one `DT_NEEDED` per distinct imported library.

## Symbol versioning

When any import carries a `version`, the linker additionally emits `.gnu.version`
(`DT_VERSYM`) and `.gnu.version_r` (`DT_VERNEED`/`DT_VERNEEDNUM`): one `Verneed`
per library, one `Vernaux` per distinct `(library, version)` pair, with version
indices starting at 2 (1 = unversioned global). This is intended for versioned
exports such as OpenSSL 3's `OPENSSL_3.0.0`. The current encode path emits all
imports unversioned, so production builds produce no `.gnu.version*` sections;
the path is exercised by the linker tests (validated against the glibc
`GLIBC_2.17` aarch64 baseline).

## Initializers

If the image carries `initializers`, the linker resolves each to its absolute
text address and emits a `.init_array` plus `DT_INIT_ARRAY`/`DT_INIT_ARRAYSZ`.
For a normal custom-entry MFBASIC binary the entry runs `_mfb_linker_init`
itself, so `initializers` is empty and no `.init_array` is produced.

## glibc flavor

```text
interpreter  /lib/ld-linux-aarch64.so.1
libc.so.6        C/POSIX runtime functions
libm.so.6        math functions (pow, sin, cos, atan2, …)
libpthread.so.0  pthread_create for thread::start
```

- `io::print` imports `write` from `libc.so.6`.
- `math::sin` imports `sin` from `libm.so.6`.
- `thread::start` imports `pthread_create` from `libpthread.so.0`.

Imported symbols use plain ELF names with no leading underscore (`write`, `read`,
`open`, `close`, `clock_gettime`, `pthread_create`, `pow`, `sin`, `cos`, …).

## musl flavor

```text
interpreter  /lib/ld-musl-aarch64.so.1
libc.musl-aarch64.so.1   C/POSIX runtime functions and pthread_create
libm.so.1                math functions (pow, sin, cos, atan2, …)
```

musl exposes the pthread entry points from libc, so `thread::start` imports
`pthread_create` from `libc.musl-aarch64.so.1` rather than a separate pthread
library.

- `io::print` imports `write` from `libc.musl-aarch64.so.1`.
- `math::sin` imports `sin` from `libm.so.1`.
- `thread::start` imports `pthread_create` from `libc.musl-aarch64.so.1`.

## Executable signing metadata

When the build supplies executable signing metadata, the linker emits it as a
`.mfb_sign` ELF section. Unlike macOS, Linux executables are not otherwise signed
by the linker.
