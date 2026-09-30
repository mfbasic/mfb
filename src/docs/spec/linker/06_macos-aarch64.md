# macOS aarch64

The macOS backend writes a Mach-O executable directly, with no host linker.
Console builds emit one file, inside the project's `build/` directory:
[[src/os/macos/link/mod.rs]]

```text
build/<project>.out
```

App-mode builds (`mfb build --app`) emit a `.app` bundle (see below). The Mach-O
writer is shared; console and app builds supply different entry/bootstrap code.

## Container layout

Constants: VM base `0x1_0000_0000`, page size `0x4000` (16 KiB), import stub size
12 bytes. Segments are emitted in this order: [[src/os/macos/link/mod.rs:VM_BASE]]

```text
__PAGEZERO     vm 0, size = VM base, no file backing, no access
__TEXT         RX: Mach-O header + load commands, __text, __stubs
__DATA_CONST   RW->RO: __got, __mod_init_func, __const (if imports, initializers, or rodata)
__DATA         RW: __data (program constants + main-arena global) (only if data)
__MFB          .mfbsign: executable signing metadata (only if signing_metadata)
__LINKEDIT     dyld info, symbol/string tables, code signature
```

Between the last of those and `__LINKEDIT` sits the `LC_NOTE` provenance-marker
payload — a small file region owned by no segment (see `provenance-marker`).

`__DATA` is a writable (`initprot = RW`) segment, which is what gives the runtime
a writable global plane (the main arena pointer and `LINK`/`term` global slots
live here). `__DATA_CONST` holds the GOT, the `__mod_init_func` pointer array,
and the read-only `__const` block; it is present when the image has imports,
initializers, **or read-only constants**. That third disjunct means any program
with a string literal gets one.
[[src/os/macos/link/macho.rs:needs_data_const]] The `__MFB`
segment is present only when the build carries executable signing metadata.

## Load commands

The header is `MH_EXECUTE` for `arm64`. The base load-command set always present
includes `LC_SEGMENT_64` for each emitted segment, `LC_LOAD_DYLINKER`
(`/usr/lib/dyld`), `LC_UUID`, `LC_BUILD_VERSION`, `LC_SOURCE_VERSION`, `LC_MAIN`
(entry = `_main`), `LC_SYMTAB`, `LC_DYSYMTAB`, `LC_FUNCTION_STARTS`,
`LC_DATA_IN_CODE`, the `LC_NOTE` provenance marker (see `provenance-marker`), and
`LC_CODE_SIGNATURE`. Additionally:

- one `LC_LOAD_DYLIB` per imported library,
- one `LC_RPATH` per vendor search path, so a bare-filename `dlopen` of a
  vendored library resolves from the configured output directory. Absent for a
  build that vendors nothing,
- and **one of two mutually exclusive fixup encodings**: `LC_DYLD_INFO_ONLY`
  (rebase/bind/export opcode streams) when a `__DATA_CONST` is present, or else
  `LC_DYLD_CHAINED_FIXUPS` (`0x80000034`) plus `LC_DYLD_EXPORTS_TRIE`
  (`0x80000033`) when it is not. [[src/os/macos/link/macho.rs:encode_unsigned_mach_o]]

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
libSystem       /usr/lib/libSystem.B.dylib
Network         /System/Library/Frameworks/Network.framework/Network
AudioToolbox    /System/Library/Frameworks/AudioToolbox.framework/AudioToolbox
CoreAudio       /System/Library/Frameworks/CoreAudio.framework/CoreAudio
CoreFoundation  /System/Library/Frameworks/CoreFoundation.framework/CoreFoundation
CoreGraphics    /System/Library/Frameworks/CoreGraphics.framework/Versions/A/CoreGraphics
CoreText        /System/Library/Frameworks/CoreText.framework/Versions/A/CoreText
Metal           /System/Library/Frameworks/Metal.framework/Versions/A/Metal
QuartzCore      /System/Library/Frameworks/QuartzCore.framework/Versions/A/QuartzCore
AppKit          /System/Library/Frameworks/AppKit.framework/AppKit
Foundation      /System/Library/Frameworks/Foundation.framework/Foundation
libobjc         /usr/lib/libobjc.A.dylib
libz            /usr/lib/libz.1.dylib
```
[[src/os/macos/link/mod.rs:dylib_path]]

Console builds draw their POSIX/pthread surface from `libSystem` using
Darwin C ABI symbol names (leading underscore: `_write`, `_read`, `_open`,
`_close`, `_pthread_create`, …). `tls::` resolves Network.framework and its
server-side Security/CoreFoundation dependencies through `dlopen`/`dlsym` at
runtime. Its static imports come from libSystem: `_dlopen`, `_dlsym`,
`___error`, `_calloc`, and `_free`; `tls::listen` also selects file-reading
operations, and endpoint-address queries select `_inet_ntop`. [[src/target/macos_aarch64/plan.rs:runtime_imports]] There is no math
import surface — `pow`/`sin`/`cos` and the rest are in-tree kernels.
App-mode builds add the toolkit imports described in `import-selection`. A
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

The image is encoded once with a zero signature-size field to determine the
signature length, then encoded again with that length in its load command. The
final signature is computed over the second unsigned image and appended. [[src/os/macos/link/commands.rs:code_signature]]

This ad-hoc signature is distinct from the optional `__MFB,.mfbsign` segment, which
carries MFBASIC's own executable signing metadata when the build supplies it. On
a signed build the second (final-size) unsigned encode is sealed — the blob's
`contentSignature` placeholder filled over the image up to the
`LC_CODE_SIGNATURE` offset — before the page hashes are taken, so the ad-hoc
signature covers the sealed blob and `codesign -v` passes.
[[src/os/macos/link/macho.rs:encode_mach_o]]

## App-mode bundle

`mfb build --app` produces:

```text
build/<project>.app/
  Contents/
    Info.plist
    MacOS/<project>            (the Mach-O executable)
    Resources/AppIcon.icns     (multi-resolution app icon)
    Frameworks/<name>...       (vendored native libraries; only when the build vendors any)
```

The bundle lands in the project's `build/` directory.

`Contents/Frameworks/` is the platform-standard location for a bundle's private
shared libraries, and is created **only** when the build resolves a `vendor`
native-library locator.
The bundled executable then carries `LC_RPATH @executable_path/../Frameworks`; see
`./mfb spec language native-libraries`.

`Info.plist` sets `CFBundleName`, `CFBundleExecutable`, `CFBundleIdentifier`
(`dev.mfbasic.<project>`), `CFBundlePackageType` `APPL`,
`CFBundleShortVersionString` and `CFBundleVersion` (both the manifest `version`),
`CFBundleIconFile` (`AppIcon`), and `NSPrincipalClass` `NSApplication`. In app
mode `_main` is an AppKit bootstrap that creates the window and spawns a worker
thread running the language entry; console mode uses `_main` as the ordinary
program entry. The worker bootstrap's runtime mechanics are owned by ./mfb spec
threading os-integration.

### Bundle version

`CFBundleShortVersionString` (the release version users see) and `CFBundleVersion`
(the build version) both carry the manifest's required `version` field verbatim.
The writer emits both keys. The manifest
validates `version` as a required non-empty string
(`./mfb spec tooling project-manifest`), so app mode always has one to publish;
the backend rejects an app build with no version rather than inventing a default.
A version carrying XML metacharacters is escaped like the project name.

### App icon

`Contents/Resources/AppIcon.icns` is a complete multi-resolution icon family (16,
32, 128, 256, 512 at @1x and @2x) generated for every app build. The source is
the project's `mode`/`icon` manifest field (`./mfb spec tooling project-manifest`)
when set — required to be a decodable 1024×1024 PNG — otherwise the compiler's
embedded default icon. The source is scaled into the Big Sur content area (824 on
a 1024 grid) and squircle-masked, so an arbitrary square image reads as a native
macOS icon; every `.icns` entry is downsampled from that single shaped 1024
canvas. `image`/`icns` are compiler build-time dependencies only.
[[src/os/macos/icon.rs:build_icns]]

### Bundle generation contract

The bundle writer creates the directory tree under the project's `build/`
directory: `build/<project>.app/Contents/MacOS` is created with one
`create_dir_all` (so the intermediate `build` and `Contents` directories are
materialized too). The Mach-O is encoded by
the same `encode_executable_bytes` helper the console `build/<project>.out`
path uses,
so the executable written to `Contents/MacOS/<project>` is byte-identical to the
console output for the same image — only the on-disk layout, the `Info.plist`,
and the `Resources/AppIcon.icns` sidecar differ.

Byte identity applies only when both writers receive the same project name and
encoded image. Actual app and console builds have different bootstrap code.
Vendored libraries also select different `rpaths`: `@executable_path/vendor`
for console output and `@executable_path/../Frameworks` for a bundle. Those paths
change load commands and the code signature; a non-vendoring build has no
`LC_RPATH`.
[[src/target/macos_aarch64/mod.rs:write_executable]]

The executable file is then
chmod'd to `0o755` (the `Info.plist` and `AppIcon.icns` are written with default
permissions, not marked executable).
[[src/os/macos/link/mod.rs:write_app_bundle]] [[src/os/macos/link/mod.rs:write_executable_file]]

The project name is substituted into every `Info.plist` string field
(`CFBundleName`, `CFBundleExecutable`, and the `dev.mfbasic.<project>`
identifier) after XML-escaping, as is the version (`CFBundleShortVersionString`,
`CFBundleVersion`). The escaper replaces the five XML predefined entities —
`&`→`&amp;`, `<`→`&lt;`, `>`→`&gt;`, `"`→`&quot;`, `'`→`&apos;` — so a project
name or version containing metacharacters produces a well-formed plist.
[[src/os/macos/link/mod.rs:plist_escape]]

## See Also

* ./mfb spec linker symbols-and-relocations — internal/external relocation
  bindings, import stubs, and the GOT
* ./mfb spec linker static-and-dynamic-output — the static-vs-dynamic image
  choice and initializers
* ./mfb spec threading os-integration — the app-mode worker-thread bootstrap
  runtime detail
