# macOS aarch64

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
