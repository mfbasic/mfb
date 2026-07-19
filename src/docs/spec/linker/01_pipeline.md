# Pipeline

Native executable output flows through a fixed sequence of stages, each with a
concrete in-compiler type. The target backend owns the pipeline; the linker is
its final target-specific stage.
[[src/target/macos_aarch64/]] [[src/target/linux_aarch64/]] [[src/target/linux_x86_64/]] [[src/target/linux_riscv64/]] [[src/os/macos/]] [[src/os/linux/]]

```text
IrProject                                                language IR
  -> NirModule                                           native IR, packages merged in
  -> NativePlan                                          symbol/import/call plan
       -> object plan validation gate (see object-plan)
  -> NativeCodePlan                                      concrete native instructions
  -> EncodedImage                                        text/data/symbols/relocs/imports
  -> target linker                                       container encoding
  -> executable file(s)
```
[[src/ir/mod.rs]] [[src/target/shared/nir/mod.rs]] [[src/target/shared/plan/mod.rs]] [[src/target/shared/code/mod.rs]] [[src/arch/aarch64/encode/mod.rs]] [[src/os/macos/link/mod.rs]] [[src/os/linux/link/mod.rs]]

## Stage producers

The backend's executable writer runs the stages in order:

1. The lowering stage lowers `IrProject` to a `NirModule`. Installed packages
   are decoded and merged here, not linked later (see `package-linking`).
2. The NIR and capability validation stages reject NIR the backend cannot lower.
3. The planning stage produces the `NativePlan`: the set of functions, data
   objects, calls, and platform imports with their final symbol names.
4. The plan's own validation then the platform object-plan validation run the
   structural checks (the object-plan gate).
5. The code-lowering stage produces the `NativeCodePlan`: concrete native
   instructions per function plus relocation requests. A code-plan validation
   follows.
6. The encoder assembles the `EncodedImage`: encoded `text` and `data` byte
   vectors, internal `symbols`, `relocations`, the import table, the entry
   symbol, an `initializers` list, and optional `signing_metadata`.
7. The backend attaches executable signing metadata and calls the platform
   linker (or the app-bundle / per-flavor variants).

[[src/target/shared/lower.rs:lower_project]] [[src/arch/aarch64/encode/mod.rs:encode]]

## The linker's contract

The linker takes an `EncodedImage` and:

- patches every relocation once final virtual addresses are known,
- generates import stubs and the GOT for external symbols,
- emits the container format (Mach-O or ELF) with the segments, load
  commands/program headers, and dynamic-loading metadata the image requires,
- writes the file to disk and marks it executable (mode `0o755`).

The linker does not decide semantic language behavior. It only materializes the
symbols, imports, relocations, and initializers requested by earlier native
lowering stages, and fails (rather than emitting a broken image) when a request
cannot be satisfied (see `failure-rules`).

The entry symbol of every encoded image is `_main`; the language entry routine
is reached from there. App-mode builds repurpose `_main` as a toolkit bootstrap
that spawns a worker thread running the language entry (see the platform
topics).

## See Also

* ./mfb spec architecture native — the native back end that owns this pipeline
* ./mfb spec linker object-plan — the validation gate between `NativePlan` and code
* ./mfb spec linker symbols-and-relocations — the symbol, relocation, and import model the `EncodedImage` carries
* ./mfb spec linker package-linking — where installed packages are decoded and merged in
