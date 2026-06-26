# Import And Library Selection

Platform import decisions are made during native planning, in the shared entry
`plan::lower_module_for_platform` (`src/target/shared/plan.rs`); the per-target
`plan::lower_module` wrappers (`src/target/macos_aarch64/plan.rs`,
`src/target/linux_aarch64/plan.rs`) delegate to it. The concrete
`(library, symbol)` selection lives in the per-target `plan.rs` platform object.
The linker does not pick libraries; it only materializes what the plan recorded.
[[src/target/shared/plan.rs:lower_module_for_platform]]

## The `PlatformImport` record

Each import is a `PlatformImport`:

```text
library      target library/soname the symbol must be resolved from
symbol       the dynamic symbol name codegen references
required_by  the internal symbol that caused the import
```
[[src/target/shared/plan.rs:PlatformImport]]

`platform_imports` collects imports, in order, from:

- the program entry support (process exit, entry error output),
- each function's operations and values (runtime helpers and native built-in
  calls),
- thread-owner cleanup,
- the native `LINK` load-time initializer, when the module declares `LINK`
  functions,
- app-mode toolkit imports, when the build is app mode.

`push_platform_import` deduplicates: an import is dropped only when all three
fields (`library`, `symbol`, `required_by`) already match an existing entry. The
same symbol required by two different functions is kept twice at this layer; the
linker later collapses imports to one dynamic dependency and one GOT slot per
distinct `(library, symbol)`.
[[src/target/shared/plan.rs:push_platform_import]]

## Target-specific library and symbol selection

The platform object maps each runtime helper or built-in call to a concrete
`(library, symbol)`:

- macOS uses `libSystem` for the C/POSIX/pthread/math surface, and Darwin C ABI
  symbol names carry a leading underscore (`_write`, `_read`, `_open`,
  `_clock_gettime`, `_pthread_create`, `_pow`, `_sin`).
- Linux uses unprefixed ELF symbol names (`write`, `read`, `open`,
  `clock_gettime`, `pthread_create`, `pow`, `sin`) and splits the surface across
  several sonames whose names depend on the flavor (see below).

Representative mappings (the symbol set per call is what that helper actually
uses):

- `io::print` / `io::write` require `write`.
- `io::readLine` requires `read`, `isatty`, `tcgetattr`, `tcsetattr`.
- `fs` file helpers require `open`, `read`, `write`, `close`, `fsync`, `lseek`,
  and error access.
- `datetime` now/monotonic helpers require `clock_gettime`.
- `thread::start` requires `pthread_create` plus the pthread mutex/cond
  primitives.
- `math::sin` and similar require the platform math symbol.

### Linux flavor differences

The Linux platform object selects sonames by flavor:

```text
                glibc                       musl
libc      libc.so.6                   libc.musl-aarch64.so.1
libm      libm.so.6                   libm.so.1
libpthread libpthread.so.0            (folded into libc.musl-aarch64.so.1)
```

So `thread::start` imports `pthread_create` from `libpthread.so.0` on glibc but
from `libc.musl-aarch64.so.1` on musl, and `math::sin` imports `sin` from
`libm.so.6` (glibc) or `libm.so.1` (musl). Because the two flavors choose
different sonames, each flavor is planned and linked independently from the same
NIR (see `linux-aarch64`).

## Native `LINK` bindings

A source-level `LINK` declaration binds a user-named external native function.
These are not resolved as ordinary platform imports. Instead
(`src/target/shared/code/link_thunk.rs`):

- The backend emits a per-program load-time initializer `_mfb_linker_init` that
  `dlopen`s each distinct `LINK` library and `dlsym`s each symbol into a writable
  global slot. This initializer requires `dlopen`/`dlsym` imports (from
  `libSystem` on macOS, from libc on Linux — recent glibc and musl both fold
  these into libc).
- The backend emits one marshaling thunk per `LINK` function
  (`_mfb_linker_<alias>_<name>`) that loads the resolved pointer from its global
  slot and calls through it, marshaling arguments and the return value across the
  C ABI.
- `_mfb_linker_init` is invoked from the entry bootstrap, not from a load-time
  initializer array.

[[src/target/shared/code/link_thunk.rs:emit_link_support]]

A call to a `LINK` function is a `CallKind::Import` referencing the internal
thunk symbol — an internal `branch26`, not an external relocation. The dynamic
dependency on the user library is expressed only through `dlopen` at run time, so
the `LINK` library name does not appear in the executable's dynamic dependency
metadata.
[[src/target/shared/plan.rs:CallKind]]

## Packages and transitive imports

A package export's body is merged into the program IR before lowering (see
`package-linking`), so any runtime helper it uses contributes its platform
imports exactly as if the app package had called the helper directly. The final
executable must include those imports even when the app package never names the
helper itself.

## See Also

* ./mfb spec language native-libraries — the source-level `LINK` syntax and
  marshaling contract
* ./mfb spec package native-bindings — how `LINK` bindings are encoded in the
  `.mfp` resource trailer
* ./mfb spec linker package-linking — how merged package bodies contribute their
  transitive platform imports
