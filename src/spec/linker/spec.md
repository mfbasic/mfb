# MFBASIC Native Linking

This document specifies how MFBASIC native executables are linked. It is the
implementation contract for the target backends and the two in-tree linkers, not
a developer-facing guide. It complements the sibling specs:

- `mfb spec architecture` — the command pipeline, packages, and binary
  representation (`.mfp`) format that feed native lowering.
- `mfb spec memory` — runtime memory layouts (arenas, heap values, the
  fallible-call ABI) the emitted code assumes.
- `mfb spec threading` — the worker-thread ABI and the platform thread imports
  the linker must satisfy.

MFBASIC does not rely on a host platform linker for native executable builds.
There is no `ld`, `gold`, `lld`, `gcc`, or `clang` invocation. The compiler
lowers the program, its installed packages, runtime helpers, and platform
imports into an in-memory native image, then the target-specific linker encodes
the final executable container (Mach-O or ELF) and writes it to disk itself.

Both backends target aarch64 only. The macOS backend emits one Mach-O
executable; the Linux backend emits two ELF executables (glibc and musl
flavors) for console builds, or one for app-mode builds.
[[src/os/macos/link/macho.rs:encode_unsigned_mach_o]]

## Reading order

The topics below follow the native linking pipeline.

- `pipeline` lays out the stages from IR project to executable file and names the
  concrete types produced at each stage.
- `object-plan` describes the `.nobj` object model — a parallel,
  JSON-serializable description of the planned image used as a structural
  validation gate before the real linker runs. It is not consumed by the linker.
- `import-selection` covers how platform imports and libraries are chosen during
  native planning, including per-flavor library selection and native `LINK`
  bindings.
- `symbols-and-relocations` specifies symbol naming, the relocation kinds and
  bindings, import stubs and the GOT, and the import/initializer capabilities of
  the encoded image.
- `static-and-dynamic-output` contrasts an import-free image with the
  dynamic-loading metadata an image with imports requires.
- `macos-aarch64` and `linux-aarch64` specify the two concrete backends — Mach-O
  with ad-hoc code signing for macOS, and ELF with its glibc and musl flavors and
  symbol versioning for Linux. Each covers its app-mode output.
- `package-linking` describes how installed `.mfp` package exports reach the
  executable (merged into IR, not linked as external symbols).
- `failure-rules` lists the conditions under which the linker must fail rather
  than emit a broken executable, with the concrete diagnostics it raises.

## See Also

* ./mfb spec architecture — the full build pipeline the linker completes
* ./mfb spec package — package inputs merged before linking
* ./mfb spec memory — the value layouts the emitted code realizes
* ./mfb spec threading — thread linking requirements
* ./mfb spec language — the source language being compiled
