# Import And Library Selection

Platform import decisions are made during native planning, in the shared
module-lowering entry; the per-target wrappers delegate to it. The concrete
`(library, symbol)` selection lives in the per-target platform object.
The linker does not pick libraries; it only materializes what the plan recorded.
[[src/target/shared/plan/lower.rs:lower_module_for_platform]]

## The `PlatformImport` record

Each import is a `PlatformImport`:

```text
library      target library/soname the symbol must be resolved from
symbol       the dynamic symbol name codegen references
required_by  the internal symbol that caused the import
```
[[src/target/shared/plan/mod.rs:PlatformImport]]

`platform_imports` collects imports from:

- the program entry support (process exit, entry error output),
- each function's operations and values (runtime helpers and native built-in
  calls),
- thread/resource cleanup and synchronization needed by implicit drops,
  arena registration, and environment access,
- the native `LINK` load-time initializer, when the module declares `LINK`
  functions,
- app-mode toolkit imports, when the build is app mode.

`push_platform_import` deduplicates: an import is dropped only when all three
fields (`library`, `symbol`, `required_by`) already match an existing entry. The
same symbol required by two different functions is kept twice at this layer;
library dependencies are deduplicated separately by the container writer.
Code lowering copies the import records without `required_by`, and the shared
encoder then keeps the first record of each `(library, symbol)` pair, in order, so
the `EncodedImage` lists every import once (bug-802; before, `nm -u` showed
`___error` three times when three helpers needed it). ELF and Mach-O allocate one
stub and GOT slot per encoded import; Windows groups the records by DLL when
building its IAT. The `.ncode` plan dump still lists the per-helper records.
[[src/codegen/engine/builder/mod.rs:lower_module_for_platform]]
[[src/arch/encode_plan.rs:encode_plan]]
[[src/os/linux/link/elf.rs:build]] [[src/os/macos/link/mod.rs:append_import_stubs]]
[[src/os/windows/link/mod.rs:group_imports_by_dll]]
[[src/target/shared/plan/symbols.rs:push_platform_import]]

## Target-specific library and symbol selection

The platform object maps each runtime helper or built-in call to a concrete
`(library, symbol)`:

- macOS uses `libSystem` for the C/POSIX/pthread surface, and Darwin C ABI
  symbol names carry a leading underscore (`_write`, `_read`, `_open`,
  `_clock_gettime`, `_pthread_create`).
- Linux uses unprefixed ELF symbol names (`write`, `read`, `open`,
  `clock_gettime`, `pthread_create`) and splits the surface across several
  sonames whose names depend on the flavor (see below).
- Windows uses unprefixed Win32 names from DLLs such as `kernel32.dll`,
  `bcrypt.dll`, and `ws2_32.dll`; it does not use the POSIX import mapping.
  [[src/target/win_x86_64/plan.rs:runtime_imports]]
- The transcendental `math::` surface imports no math library: every `Float`
  transcendental, `pow`,
  `atan2`, `tan`, and the `Float MOD` (`fmod`) lowers to an in-tree
  kernel, so no `math.*` call selects a platform math symbol and no build links
  `libm`.

Representative POSIX mappings (additional synchronization/error imports may be
required by a helper):

- `io::print` / `io::write` use `write` on macOS and Linux aarch64/riscv64.
  Linux x86-64 emits a raw write syscall instead.
  [[src/target/linux_x86_64/plan.rs:ABI]]
- `io::readLine` requires `read`, `isatty`, `tcgetattr`, `tcsetattr`.
- `fs` file helpers require `open`, `read`, `write`, `close`, `fsync`, `lseek`,
  and error access; Linux x86-64 uses raw syscalls for its write primitive.
- `datetime` now/monotonic helpers require `clock_gettime`.
- `thread::start` requires `pthread_create` plus the pthread mutex/cond
  primitives.
- `process::` calls require the fork/exec/pipe surface (`fork`, `execvp`, `pipe`,
  `dup2`, `close`, `waitpid`, `kill`, `read`, `write`, `poll`, `fcntl`) and error
  access. `process::detach` additionally requires `pthread_create` and
  `pthread_detach`: it reaps the one child it detached on a dedicated thread
  (`_mfb_rt_process_reaper`) rather than by changing the process-wide `SIGCHLD`
  disposition, which would auto-reap *every* child and break `process::waitFor`
  on the others.
  [[src/codegen/builtins/process/func_detach.rs:lower_process_detach_helper_posix]]
- `math::rand`/`math::seed` declare
  `getentropy` on POSIX for the startup seed; the transcendentals, `pow`,
  `atan2`, `tan`, and `Float MOD` are in-tree kernels.

### Linux flavor differences

The Linux platform object selects libc and pthread sonames by flavor and ISA:

```text
                glibc              musl
libc        libc.so.6          libc.musl-<arch>.so.1
pthread     libpthread.so.0    libc.musl-<arch>.so.1   (aarch64/riscv64)
pthread     libc.so.6          libc.musl-x86_64.so.1  (x86-64)
```

where `<arch>` is `aarch64`, `x86_64`, or `riscv64`. `LinuxAbi` supplies both
the musl libc name and the glibc pthread library; `LinuxPlan::libpthread`
selects the flavor-specific value.
[[src/target/linux_common/plan.rs:libpthread]] [[src/target/linux_x86_64/plan.rs:ABI]]

Because the two flavors choose different
sonames, each flavor is planned and linked independently from the same NIR (see
`linux-aarch64`). There is no `libm` row: the `math::` kernels are in-tree, so no
flavor lists `libm.so` as a needed library.

## Native `LINK` bindings

A source-level `LINK` declaration binds a user-named external native function.
These are not resolved as ordinary platform imports. Instead:

- On POSIX, the backend emits a native-library initializer `_mfb_linker_init` that
  `dlopen`s each distinct `LINK` library and `dlsym`s each symbol into a writable
  global slot. This initializer requires `dlopen`/`dlsym` imports (from
  `libSystem` on macOS, from libc on Linux — recent glibc and musl both fold
  these into libc).
- The backend emits one marshaling thunk per `LINK` function
  (`_mfb_linker_<alias>_<name>`) that loads the resolved pointer from its global
  slot and calls through it, marshaling arguments and the return value across the
  C ABI.
- On Windows the corresponding operations are `LoadLibraryExA` and
  `GetProcAddress`, declared by `link_imports`.
  [[src/target/win_x86_64/plan.rs:link_imports]]
- `_mfb_linker_init` is invoked from the entry bootstrap, not from a load-time
  initializer array.

[[src/codegen/link/thunk/link_thunk.rs:emit_link_support]]

A call to a `LINK` function is a `CallKind::Local` referencing the internal
thunk symbol — an internal call relocation (`branch26`, `call_pc32`, or
`riscv_call`, according to ISA). The dynamic
dependency on the user library is expressed through runtime loading
(`dlopen` or `LoadLibraryExA`), so
the `LINK` library name does not appear in the executable's dynamic dependency
metadata.
[[src/target/shared/plan/mod.rs:CallKind]]

## App-mode toolkit imports

When the build is app mode, `platform_imports` adds `app_mode_imports`, with
pointer-specific imports conditional on `module_uses_mouse` and, for the handlers that
publish the pointer to the canvas graphics state, on `module_uses_canvas`. Calls to the `app`
window surface additionally select `app_window_imports`. These bootstrap imports
are attributed to `_main`.
[[src/target/shared/plan/symbols.rs:platform_imports]]

### macOS

`app_mode_imports` supplies the Obj-C messaging/class/selector operations from
`libobjc`, AppKit and Foundation classes and attribute globals, CoreGraphics
image/color operations, and libSystem thread, pipe, and memory operations.
`_OBJC_CLASS_$_*` and attribute-name globals are addressed through GOT slots.
A program that installs a tracking area — one that uses a mouse member, or a canvas
program, whose view publishes the pointer — adds `_OBJC_CLASS_$_NSTrackingArea`, and a
canvas program adds `_OBJC_CLASS_$_NSEvent` for the held-button mask; window operations
add notification and mutex imports. Canvas helper imports can additionally select Metal,
QuartzCore, and CoreText. The exact per-helper mapping is `runtime_imports`;
framework install paths are specified in `macos-aarch64`.
[[src/target/macos_aarch64/plan.rs:app_mode_imports]]
[[src/target/macos_aarch64/plan.rs:app_window_imports]]
[[src/target/macos_aarch64/plan.rs:runtime_imports]]

### Linux

On aarch64 and x86-64, app mode selects ordinary C imports from
`libgtk-4.so.1`, `libgobject-2.0.so.0`, `libglib-2.0.so.0`,
`libgio-2.0.so.0`, `libcairo.so.2`, `libpango-1.0.so.0`, and
`libpangocairo-1.0.so.0`. It also selects libc/pthread support for the worker,
input pipe, memory, and `__libc_start_main` bootstrap. Library names for libc and
pthread follow the target/flavor mapping above. The controller constructors and
accessors are conditional: a program that uses a mouse member declares the reporting
set, a canvas program declares the set its drawing area's publishing controllers call,
and a symbol both reach is declared once. RISC-V rejects app mode.
[[src/target/linux_gtk/mod.rs:app_mode_imports]]
[[src/target/linux_riscv64/mod.rs:supports_app_mode]]

### Windows

`app_mode_imports` selects Win32 window/message-loop calls from `user32.dll`,
drawing operations from `gdi32.dll`, and thread, pipe, memory, and process support
from `kernel32.dll`. Mouse use adds its coordinate-conversion import, and a canvas
program adds `TrackMouseEvent`, which its window procedure arms `WM_MOUSELEAVE` with;
window operations add the `app_window_imports` set. The PE writer selects the GUI
subsystem and embeds resources as described in `windows-x86_64`.
[[src/target/win_x86_64/plan.rs:app_mode_imports]]
[[src/target/win_x86_64/plan.rs:app_window_imports]]

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
* ./mfb spec app macos-runtime — how the app-mode bootstrap consumes the macOS
  Obj-C/AppKit/Foundation toolkit imports
* ./mfb spec app linux-runtime — how the app-mode bootstrap consumes the Linux
  GTK toolkit imports and `__libc_start_main`
