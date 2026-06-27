# Static Versus Dynamic Output

Each linker chooses between a static and a dynamic encoding based on whether the
encoded image has imports.

## No imports — static image

If an image has no imports, the linker emits a simpler executable with only
internal text and data and no dynamic-loading metadata:

- macOS: `encode_unsigned_mach_o` still emits `__PAGEZERO`/`__TEXT`/`__LINKEDIT`
  and a (possibly empty) symbol table, with no `LC_LOAD_DYLIB`, no
  `__DATA_CONST`/GOT, and no dyld bind/rebase opcodes.
- Linux: `encode_static_elf` emits a single `PT_LOAD`, no `PT_INTERP`, and no
  `.dynamic` section. [[src/os/linux/link.rs:encode_static_elf]]

A static image needs nothing from a dynamic loader, but on macOS it is still
ad-hoc code-signed (see `macos-aarch64`).

## Imports present — dynamic image

If imports are present, the linker emits the full dynamic-loading metadata:

- a dynamic dependency record per distinct imported library
  (`LC_LOAD_DYLIB` / `DT_NEEDED`),
- dynamic symbol and string tables covering every imported symbol,
- relocations that let the loader fill GOT entries
  (Mach-O bind opcodes / ELF `R_AARCH64_JUMP_SLOT` and `R_AARCH64_GLOB_DAT`),
- one import stub per imported function for generated code to branch to,
- on Linux, the interpreter (`PT_INTERP`) and a `PT_DYNAMIC` segment.

## Initializers

Independently of imports, if the image carries `initializers` they are emitted as
a run-before-entry pointer array: Mach-O `__mod_init_func`
(`S_MOD_INIT_FUNC_POINTERS`, rebased by dyld) or ELF `.init_array`
(`DT_INIT_ARRAY`/`DT_INIT_ARRAYSZ`). On macOS this forces a `__DATA_CONST`
segment even when there are no imports. The current encode path leaves
`initializers` empty (see `symbols-and-relocations`).

## App-mode entry-bootstrap divergences

The entry-import set the platform plan emits depends on the build mode, not only
on the program's own imports. In console mode the macOS plan adds `_signal` to
install the SIGINT/SIGTERM handlers; in app mode (`MacApp`) that import is
omitted, because the bundle relies on its window-driven finish path rather than
console signal handlers. The always-present entry imports (`_exit`,
`_getentropy`, `_clock_gettime` for the memory-fill RNG seed) are unchanged
across modes. [[src/target/macos_aarch64/plan.rs:entry_imports]]

Build modes are also platform-exclusive: the macOS backend rejects a `LinuxApp`
build mode with an internal error, and the Linux backend rejects `MacApp` the
same way. The CLI selects the build mode from the target OS, so neither cross
combination is expected to reach a backend. [[src/target/macos_aarch64/mod.rs:write_executable]]

## No silent omission

The linker must not silently omit a required library, stub, or GOT slot. A
missing import, an unbacked external relocation, or an unsupported relocation is a
linker or codegen error, never a zero address or a placeholder (see
`failure-rules`).

## See Also

* ./mfb spec app macos-runtime — the window-driven finish path and AppKit
  bootstrap that make app mode omit the console `_signal` import.
