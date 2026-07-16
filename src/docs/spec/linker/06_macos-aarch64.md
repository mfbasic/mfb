# macOS aarch64

The macOS backend writes a Mach-O executable directly, with no host linker.
Console builds emit one file, inside the project's `build/` directory:
[[src/os/macos/link.rs]]

```text
build/<project>.out
```

App-mode builds (`mfb build --app`) emit a `.app` bundle (see below). The Mach-O
bytes are identical in both cases.

## Container layout

Constants: VM base `0x1_0000_0000`, page size `0x4000` (16 KiB), import stub size
12 bytes. Segments are emitted in this order: [[src/os/macos/link/mod.rs:VM_BASE]]

```text
__PAGEZERO     vm 0, size = VM base, no file backing, no access
__TEXT         RX: Mach-O header + load commands, __text, __stubs
__DATA_CONST   RW->RO: __got, __mod_init_func   (only if imports or initializers)
__DATA         RW: __data (program constants + main-arena global) (only if data)
__MFB          __sign: executable signing metadata (only if signing_metadata)
__LINKEDIT     dyld info, symbol/string tables, code signature
```

Between the last of those and `__LINKEDIT` sits the `LC_NOTE` provenance-marker
payload — a small file region owned by no segment (see `provenance-marker`).

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
`LC_DATA_IN_CODE`, the `LC_NOTE` provenance marker (see `provenance-marker`), and
`LC_CODE_SIGNATURE`. Additionally:

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
[[src/os/macos/link/mod.rs:dylib_path]]

Console builds draw their POSIX/pthread surface from `libSystem` using
Darwin C ABI symbol names (leading underscore: `_write`, `_read`, `_open`,
`_close`, `_pthread_create`, …); `net::` adds `Network`. There is no math
import surface — `pow`/`sin`/`cos` and the rest are in-tree kernels.
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
the signature in place. [[src/os/macos/link/commands.rs:code_signature]]

This ad-hoc signature is distinct from the optional `__MFB,__sign` segment, which
carries MFBASIC's own executable signing metadata when the build supplies it.

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

The bundle lands in the project's `build/` directory alongside every other build
artifact.

`Contents/Frameworks/` is the platform-standard location for a bundle's private
shared libraries, and is created **only** when the build resolves a `vendor`
native-library locator — an empty `Frameworks/` in every bundle would be noise.
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
Both keys are mandatory: App Store upload validation (`altool`) rejects a bundle
missing either one, and Launch Services reports an unversioned app. The manifest
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

The bundle writer recreates the directory tree under the project's `build/`
directory: `build/<project>.app/Contents/MacOS` is created with one
`create_dir_all` (so the intermediate `build` and `Contents` directories are
materialized too). The Mach-O is encoded by
the same `encode_executable_bytes` helper the console `build/<project>.out`
path uses,
so the executable written to `Contents/MacOS/<project>` is byte-identical to the
console output for the same image — only the on-disk layout, the `Info.plist`,
and the `Resources/AppIcon.icns` sidecar differ.

That invariant is **qualified for a build that vendors native libraries**: the two
shapes load their dylibs from genuinely different places
(`build/vendor/` vs `Contents/Frameworks/`), so they carry different `LC_RPATH`
strings and differ by exactly that one load command. Identical bytes would mean
one of them is wrong. For every build that vendors nothing — which is every
project that does not use a `vendor` locator — the unqualified invariant holds:
no `LC_RPATH` is emitted at all and the two are byte-identical.

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
