# Linux aarch64

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

## glibc Flavor

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

## musl Flavor

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
