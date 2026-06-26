# Pipeline

Native executable output flows through a fixed sequence of stages, each with a
concrete in-compiler type. The target backend (`src/target/macos_aarch64/`,
`src/target/linux_aarch64/`) owns the pipeline; the linker (`src/os/macos/`,
`src/os/linux/`) is its final target-specific stage.

```text
IrProject            (src/ir.rs)                         language IR
  -> NirModule       (src/target/shared/nir.rs)          native IR, packages merged in
  -> NativePlan      (src/target/shared/plan.rs)          symbol/import/call plan
       -> object plan validation gate (see object-plan)
  -> NativeCodePlan  (src/target/shared/code/mod.rs)      concrete aarch64 instructions
  -> EncodedImage    (src/arch/aarch64/encode.rs)         text/data/symbols/relocs/imports
  -> target linker   (src/os/<platform>/link.rs)          container encoding
  -> executable file(s)
```

## Stage producers

The backend `write_executable` (e.g. `src/target/macos_aarch64/mod.rs`) runs the
stages in order:

1. `lower::lower_project` lowers `IrProject` to a `NirModule`. Installed packages
   are decoded and merged here, not linked later (see `package-linking`).
2. `validate::validate_nir` and `validate::validate_capabilities` reject NIR the
   backend cannot lower.
3. `plan::lower_module` produces the `NativePlan`: the set of functions, data
   objects, calls, and platform imports with their final symbol names.
4. `native_plan.validate()` then `os::<platform>::validate_native_object_plan`
   run the structural checks (the object-plan gate).
5. `code::lower_module` produces the `NativeCodePlan`: concrete aarch64
   instructions per function plus relocation requests. `native_code.validate()`
   follows.
6. `arch::aarch64::encode::encode` assembles the `EncodedImage`: encoded `text`
   and `data` byte vectors, internal `symbols`, `relocations`, the import table,
   the entry symbol, an `initializers` list, and optional `signing_metadata`.
7. The backend attaches executable signing metadata
   (`image.signing_metadata = …`) and calls the platform linker
   (`os::<platform>::write_linked_executable`, or the app-bundle / per-flavor
   variants).

[[src/target/shared/lower.rs:lower_project]] [[src/arch/aarch64/encode.rs:encode]]

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
