# Import And Library Selection

Platform import decisions are made during native planning, in the shared entry
`plan::lower_module_for_platform` (`src/target/shared/plan.rs`); the per-target
`plan::lower_module` wrappers (`src/target/macos_aarch64/plan.rs`,
`src/target/linux_aarch64/plan.rs`) delegate to it. The concrete
`(library, symbol)` selection lives in the per-target `plan.rs` platform object.
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
- The `math::` surface imports **nothing**: every `Float` transcendental, `pow`,
  `atan2`, `tan`, and the `Float MOD` (`fmod`) lowers to an in-tree NEON/GPR
  kernel, so no `math.*` call selects a platform math symbol and no build links
  `libm`.

Representative mappings (the symbol set per call is what that helper actually
uses):

- `io::print` / `io::write` require `write`.
- `io::readLine` requires `read`, `isatty`, `tcgetattr`, `tcsetattr`.
- `fs` file helpers require `open`, `read`, `write`, `close`, `fsync`, `lseek`,
  and error access.
- `datetime` now/monotonic helpers require `clock_gettime`.
- `thread::start` requires `pthread_create` plus the pthread mutex/cond
  primitives.
- `math::` calls require **no** platform symbol — `math::rand`/`math::seed` pull
  only `getentropy` (libc) for the startup seed; the transcendentals, `pow`,
  `atan2`, `tan`, and `Float MOD` are in-tree kernels.

### Linux flavor differences

The Linux platform object selects sonames by flavor:

```text
                glibc                       musl
libc      libc.so.6                   libc.musl-aarch64.so.1
libpthread libpthread.so.0            (folded into libc.musl-aarch64.so.1)
```

So `thread::start` imports `pthread_create` from `libpthread.so.0` on glibc but
from `libc.musl-aarch64.so.1` on musl. Because the two flavors choose different
sonames, each flavor is planned and linked independently from the same NIR (see
`linux-aarch64`). There is no `libm` row: the `math::` kernels are in-tree, so no
flavor lists `libm.so` as a needed library.

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
[[src/target/shared/plan/mod.rs:CallKind]]

## App-mode toolkit imports

When the build is app mode, the platform object contributes a fixed toolkit
import set (`app_mode_imports`) in addition to the per-helper imports above.
Every entry is `required_by` the entry symbol `_main`, because the app-mode
bootstrap — not any user helper — is what references them.

### macOS

macOS app mode injects a fixed list keyed to the Obj-C runtime, since every
AppKit/Foundation call goes through `objc_msgSend` rather than a direct C
import. The set is:

- libobjc runtime functions: `_objc_msgSend`, `_sel_registerName`,
  `_objc_autoreleasePoolPush`, `_objc_setAssociatedObject`,
  `_objc_getAssociatedObject`, `_objc_allocateClassPair`, `_class_addMethod`,
  `_objc_registerClassPair`.
- `_OBJC_CLASS_$_*` class symbols, referenced as external **data** read through
  the GOT. These serve a dual purpose: they yield the class pointers AppKit
  messaging needs, and naming them as imports force-loads the AppKit and
  Foundation frameworks. They are split across libraries by where the class
  lives: `_OBJC_CLASS_$_NSObject` from `libobjc`; the `NSApplication`,
  `NSWindow`, `NSScrollView`, `NSTextView`, `NSView`, `NSColor`,
  `NSLayoutManager`, `NSFont`, `NSMenu`, `NSMenuItem` classes from `AppKit`; and
  `NSString`, `NSMutableString`, `NSDictionary`, `NSMutableDictionary`,
  `NSNumber`, `NSAttributedString` from `Foundation`.
- AppKit `NSString` attribute-name globals: `_NSFontAttributeName`,
  `_NSForegroundColorAttributeName`, `_NSUnderlineStyleAttributeName`,
  `_NSStrokeWidthAttributeName`, plus the function `_NSRectFill` (all from
  `AppKit`).
- libSystem support: `_pthread_create`, `_pthread_attr_init`,
  `_pthread_attr_setstacksize`, `_pause`, `_getenv`, `_write`, `_pipe`,
  `_dup2`, `_strlen`, `_calloc`, `_bzero`, `_memmove`.

All names carry the Darwin leading underscore. See `macos-runtime` for how the
bootstrap consumes these (NSApplication/window setup, the worker thread, and the
window-input pipe wired to the reused console fd-0 readers).
[[src/target/macos_aarch64/plan.rs:app_mode_imports]]

### Linux

Linux app mode targets GTK, which is plain C, so each toolkit call is an
ordinary imported function (no `objc_msgSend` layer). The set adds these
`DT_NEEDED` sonames, each contributing one dynamic dependency:
`libgtk-4.so.1`, `libgobject-2.0.so.0`, `libglib-2.0.so.0`,
`libgio-2.0.so.0`, and `libcairo.so.2`. The imports cover GtkApplication and
window lifecycle, the scrolled `GtkTextView` transcript, key input controllers,
the `GtkDrawingArea` + Cairo `term::` surface, and GObject signal/idle wiring.

It also adds the worker-thread and window-input-pipe primitives from
libc/libpthread (`pthread_create`, `pthread_detach`, `pipe`, `dup2`, `getenv`,
`setenv`, `write`, `malloc`, `free`, `memcpy`, `memset`, `memmove`, `pause`),
and `__libc_start_main`. Because the entry cannot link `crt1.o`, the bootstrap
calls `__libc_start_main` directly so the C runtime and shared-library
constructors — the GLib/GObject type system — run before the real `main`. See
`linux-runtime` for the bootstrap detail.
[[src/target/linux_aarch64/plan.rs:app_mode_imports]]

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
