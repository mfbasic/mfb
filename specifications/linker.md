# MFBASIC Native Linking

Last updated: 2026-06-14

This document describes how MFBASIC native executables are linked. It
complements:

- `specifications/package_format.md`
- `specifications/threading.md`
- `specifications/memory_layouts.md`

MFBASIC does not rely on a host platform linker for native executable builds.
The compiler lowers the program, packages, runtime helpers, and platform imports
into a native code image, then the target-specific linker writes the final
executable file to disk.

## 1. Pipeline

Native executable output flows through these stages:

```text
IR project
  -> target NIR
  -> native plan
  -> native code plan
  -> encoded architecture image
  -> target linker
  -> executable file(s)
```

The target backend owns the pipeline. The linker is the final target-specific
stage: it takes an encoded image containing text, data, symbols, relocations,
and imports, patches relocations, emits the executable container format, and
writes the executable to disk.

The linker does not decide semantic language behavior. It only materializes the
symbols, imports, and relocations requested by earlier native lowering stages.

## 2. Import And Library Selection

Library decisions are made before final linking, primarily in the target native
plan and target codegen platform.

The native plan collects imports from:

- Program entry support, such as process exit and entry error output.
- Built-in runtime helpers used by the app package.
- Built-in runtime helpers used only inside package bytecode.
- Native built-in calls, such as math functions lowered to platform libraries.
- Platform runtime requirements, such as thread creation.

Each import has:

```text
library
symbol
requiredBy
```

`library` is the target library name that must appear in the final executable's
dynamic dependency metadata. `symbol` is the dynamic symbol name used by codegen.
`requiredBy` records which function or runtime helper caused the import.

Runtime helper import selection is target-specific. For example:

- `io::print` and `io::write` require a platform `write` function.
- `fs::readText` requires file functions such as `open`, `read`, `close`, and
  error access.
- `thread::start` requires a target thread creation primitive.
- `math::sin` requires the target math library symbol.

Package bytecode matters even when the app package does not directly call a
helper. If an imported package export uses `fs::readText`, `io::print`,
`thread::start`, or another runtime helper, the final executable must still
include the platform imports required by that package export.

## 3. Symbols And Relocations

Native code uses three broad kinds of symbols:

- Internal text symbols for app functions, package exports, and runtime helpers.
- Internal data symbols for constants and runtime data.
- External import symbols for platform libraries.

Relocations describe how instructions should be patched once final addresses are
known. Current aarch64 backends support:

- `branch26` for internal and external calls.
- `page21` and `pageoff12` for internal data addressing.
- Target-specific external data addressing where required.

External calls branch to linker-generated import stubs. The stubs load the
resolved dynamic address from a GOT-style table and branch to it.

## 4. Static Versus Dynamic Output

If an encoded image has no imports, a target may emit a simpler executable with
only internal text/data and no dynamic library metadata.

If imports are present, the target linker must emit dynamic loading metadata:

- Dynamic dependency records for every imported library.
- Dynamic symbol/string tables for every imported symbol.
- Relocations that let the runtime loader fill GOT entries.
- Import stubs that generated code can branch to.

The compiler must not silently omit a required library. Missing imports are
linker or codegen errors.

## 5. macOS aarch64

The macOS aarch64 backend emits one executable:

```text
<project>.out
```

The target linker writes a Mach-O executable directly. It supports:

- Internal `branch26` call relocations.
- Internal `page21` / `pageoff12` data relocations.
- External branch-call imports.
- External data/GOT-style imports required by libSystem integration.
- Mach-O import stubs and binding metadata for dynamic symbols.

macOS dynamic imports are currently restricted to:

```text
libSystem
```

Symbol names use the Darwin C ABI spelling, such as:

```text
_write
_read
_open
_close
_poll
_access
_stat
_getcwd
_confstr
_pthread_create
_pow
_sin
_cos
```

Threading uses libSystem pthread creation as documented in
`specifications/threading.md`; raw Mach thread creation is not the worker
thread ABI.

Math functions that require a platform library use libSystem symbols such as
`_pow`, `_exp`, `_log`, `_sin`, `_cos`, and related functions.

## 6. Linux aarch64

The Linux aarch64 backend is cross-compiled and emits executables directly. It
does not invoke `ld`, `gold`, `lld`, `gcc`, `clang`, or any host linker.

One Linux target build emits two executable flavors:

```text
<project>-glibc.out
<project>-musl.out
```

The backend lowers and links each flavor independently because their dynamic
loader and library names differ.

The Linux linker writes ELF64 aarch64 executables. It supports:

- Internal `branch26` call relocations.
- Internal `page21` / `pageoff12` data relocations.
- External `branch26` imports through generated stubs.
- `.dynamic`, `PT_DYNAMIC`, dynamic string/symbol tables, SysV hash, GOT, and
  `R_AARCH64_JUMP_SLOT` relocation records for imported functions.

Imported Linux functions use normal ELF symbol names without a leading
underscore, such as:

```text
write
read
open
close
poll
access
stat
getcwd
getenv
pthread_create
pow
sin
cos
```

### 6.1 glibc Flavor

The glibc executable uses:

```text
interpreter /lib/ld-linux-aarch64.so.1
```

Library selection:

```text
libc.so.6        C/POSIX runtime functions
libm.so.6        math functions such as pow, sin, cos, atan2
libpthread.so.0  pthread_create for thread.start
```

Examples:

- `io::print` imports `write` from `libc.so.6`.
- `fs::readText` imports file and errno functions from `libc.so.6`.
- `math::sin` imports `sin` from `libm.so.6`.
- `thread::start` imports `pthread_create` from `libpthread.so.0`.

The linker emits one `DT_NEEDED` entry for each distinct imported library used
by the executable.

### 6.2 musl Flavor

The musl executable uses:

```text
interpreter /lib/ld-musl-aarch64.so.1
```

Library selection:

```text
libc.musl-aarch64.so.1  C/POSIX runtime functions and pthread_create
libm.so.1               math functions such as pow, sin, cos, atan2
```

Musl exposes pthread entry points from libc for the current backend, so
`thread::start` imports `pthread_create` from `libc.musl-aarch64.so.1` rather
than from a separate pthread library.

Examples:

- `io::print` imports `write` from `libc.musl-aarch64.so.1`.
- `fs::readText` imports file and errno functions from
  `libc.musl-aarch64.so.1`.
- `math::sin` imports `sin` from `libm.so.1`.
- `thread::start` imports `pthread_create` from
  `libc.musl-aarch64.so.1`.

The linker emits one `DT_NEEDED` entry for each distinct imported library used
by the executable.

## 7. Package Linking

Package files are architecture-independent `.mfp` files. During executable
native linking, the backend reads installed package metadata and emits native
functions for reachable package exports.

Package export symbols are derived as:

```text
_mfb_pkg_<package>_<export>
```

Package-to-package calls resolve through package identity and ABI metadata, not
raw bytecode function ids. The linker must not assume package-local function ids
are globally unique.

When package bytecode uses runtime helpers, the final executable must include
the helper implementation and any platform imports required by that helper. This
is true even if the app package does not directly call the helper.

## 8. Failure Rules

The linker must fail rather than generate a broken executable when:

- An internal symbol cannot be resolved.
- A relocation kind is unsupported for the target.
- An external relocation references an import that has no generated stub.
- A target receives an import for a library it does not support.
- A required package export or ABI dependency cannot be resolved.

It is not valid to satisfy missing linker support with a placeholder helper, an
empty dynamic dependency, a zero address, or a runtime "unsupported" fallback.
