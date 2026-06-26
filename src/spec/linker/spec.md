# MFBASIC Native Linking

This document describes how MFBASIC native executables are linked. It
complements:

- `specifications/package_format.md`
- `specifications/threading.md`
- `specifications/memory_layouts.md`

Last updated: 2026-06-14

MFBASIC does not rely on a host platform linker for native executable builds.
The compiler lowers the program, packages, runtime helpers, and platform imports
into a native code image, then the target-specific linker writes the final
executable file to disk.

## Reading order

The topics below follow the native linking pipeline. `pipeline` lays out the
stages from IR project to executable file. `import-selection` covers how imports
and libraries are chosen before final linking; `symbols-and-relocations`
describes the symbol kinds and relocation patches; and `static-and-dynamic-output`
contrasts import-free output with the dynamic-loading metadata an image with
imports requires. The `macos-aarch64` and `linux-aarch64` topics specify the two
concrete backends (the latter with its glibc and musl flavors). `package-linking`
covers how `.mfp` package exports are linked into the executable, and
`failure-rules` lists the conditions under which the linker must fail rather than
emit a broken executable.
