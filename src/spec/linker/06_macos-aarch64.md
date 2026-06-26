# macOS aarch64

The macOS backend (`src/os/macos/link.rs`) writes a Mach-O executable directly,
with no host linker. Console builds emit one file:

```text
<project>.out
```

App-mode builds (`mfb build -app`) emit a `.app` bundle (see below). The Mach-O
bytes are identical in both cases.

## Container layout

Constants: VM base `0x1_0000_0000`, page size `0x4000` (16 KiB), import stub size
12 bytes. Segments are emitted in this order: [[src/os/macos/link.rs:VM_BASE]]

```text
__PAGEZERO     vm 0, size = VM base, no file backing, no access
__TEXT         RX: Mach-O header + load commands, __text, __stubs
__DATA_CONST   RW->RO: __got, __mod_init_func   (only if imports or initializers)
__DATA         RW: __data (program constants + main-arena global) (only if data)
__MFB          __sign: executable signing metadata (only if signing_metadata)
__LINKEDIT     dyld info, symbol/string tables, code signature
```

`__DATA` is a writable (`initprot = RW`) segment, which is what gives the runtime
a writable global plane (the main arena pointer and `LINK`/`term` global slots
live here). `__DATA_CONST` holds the GOT and the `__mod_init_func` pointer array;
it is present only when the image has imports or initializers. The `__MFB`
segment is present only when the build carries executable signing metadata.

## Load commands

The header is `MH_EXECUTE` for `arm64`. The base load-command set always present
includes `LC_SEGMENT_64` for each emitted segment, `LC_LOAD_DYLINKER`
(`/usr/lib/dyld`), `LC_UUID`, `LC_BUILD_VERSION`, `LC_SOURCE_VERSION`, `LC_MAIN`
(entry = `_main`), `LC_SYMTAB`, `LC_DYSYMTAB`, `LC_FUNCTION_STARTS`,
`LC_DATA_IN_CODE`, and `LC_CODE_SIGNATURE`. Additionally:

- one `LC_LOAD_DYLIB` per imported library,
- `LC_DYLD_INFO_ONLY` when a `__DATA_CONST` is present (carrying the
  rebase/bind/export opcode streams).

## Relocations and imports

The linker patches internal `branch26`, `page21`, and `pageoff12` relocations to
final addresses. External relocations are redirected through the import
machinery: external `branch26` to the symbol's 12-byte stub
(`adrp x16` / `ldr x16, [x16, …]` / `br x16`), and external `page21`/`pageoff12`
to the symbol's `__got` slot.

GOT slots are zero-filled in the file and bound by dyld via the bind opcode
stream (`LC_DYLD_INFO_ONLY`), which sets a dylib ordinal (1-based, in
first-seen library order), the symbol name, and the slot offset. The
`__mod_init_func` pointers are rebased (not bound) by dyld so the load slide is
applied before `_main` runs.

## Supported dylibs

The linker resolves a fixed set of library names to install paths:

```text
libSystem    /usr/lib/libSystem.B.dylib
Network      /System/Library/Frameworks/Network.framework/Network
AppKit       /System/Library/Frameworks/AppKit.framework/AppKit
Foundation   /System/Library/Frameworks/Foundation.framework/Foundation
libobjc      /usr/lib/libobjc.A.dylib
libz         /usr/lib/libz.1.dylib
```
[[src/os/macos/link.rs:dylib_path]]

Console builds draw their POSIX/pthread/math surface from `libSystem` using
Darwin C ABI symbol names (leading underscore: `_write`, `_read`, `_open`,
`_close`, `_pthread_create`, `_pow`, `_sin`, `_cos`, …); `net::` adds `Network`.
App-mode builds add `AppKit`/`Foundation`/`libobjc` for the toolkit bootstrap. A
library name outside this set is a linker error. Threading uses libSystem pthread
creation (see `mfb spec threading`); raw Mach thread creation is not the worker
ABI.

## Ad-hoc code signing

arm64 macOS refuses to run an unsigned binary, so the linker always emits an
ad-hoc code signature, even for a static, import-free image. The
`LC_CODE_SIGNATURE` command points into `__LINKEDIT` at a SuperBlob
(magic `0xfade_0cc0`) wrapping a CodeDirectory (magic `0xfade_0c02`):

- hash type SHA-256 (32-byte hashes),
- identifier `mfb.<project>`,
- one page hash per 4096-byte page of the unsigned image (note: the
  CodeDirectory page size is 4096, distinct from the 16 KiB Mach-O page size).

The image is encoded once unsigned to compute the hashes, then re-encoded with
the signature in place. [[src/os/macos/link.rs:code_signature]]

This ad-hoc signature is distinct from the optional `__MFB,__sign` segment, which
carries MFBASIC's own executable signing metadata when the build supplies it.

## App-mode bundle

`mfb build -app` produces:

```text
<project>.app/
  Contents/
    Info.plist
    MacOS/<project>      (the Mach-O executable, byte-identical to <project>.out)
```

`Info.plist` sets `CFBundleName`, `CFBundleExecutable`, `CFBundleIdentifier`
(`dev.mfbasic.<project>`), `CFBundlePackageType` `APPL`, and `NSPrincipalClass`
`NSApplication`. In app mode `_main` is an AppKit bootstrap that creates the
window and spawns a worker thread running the language entry; console mode uses
`_main` as the ordinary program entry. The worker bootstrap's runtime mechanics
are owned by ./mfb spec threading os-integration.

## See Also

* ./mfb spec linker symbols-and-relocations — internal/external relocation
  bindings, import stubs, and the GOT
* ./mfb spec linker static-and-dynamic-output — the static-vs-dynamic image
  choice and initializers
* ./mfb spec threading os-integration — the app-mode worker-thread bootstrap
  runtime detail
