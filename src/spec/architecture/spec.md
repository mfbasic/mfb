# Compiler Architecture

How the `mfb` compiler turns an MFBASIC project into either a native executable or a compiled `.mfp` package.

This is an implementation architecture reference for compiler developers, not a
language reference. The language syntax is specified by the `language` spec and
the package/container formats by the `package` spec
(`./mfb spec language`, `./mfb spec package`). The project manifest fields the
build actually consumes are described in the `frontend` topic.

The compiler is a single Rust binary named `mfb`. The command-line entry point
is `src/main.rs`. It owns project-level orchestration, manifest validation,
package-management commands, build-mode selection, and high-level error
handling.

## Pipeline shape

The build pipeline has a shared source front end:

```text
project.json
  -> source discovery
  -> lexing
  -> parsing
  -> AST
  -> name resolution
  -> monomorphization
  -> name resolution again
  -> entry-point validation
  -> type checking
  -> IR
```

After IR, the pipeline splits:

```text
Executable build:
  IR
    -> native IR
    -> native plan
    -> native code plan
    -> encoded aarch64 image
    -> OS executable container/link step
    -> <project>.out

Package build:
  IR
    -> MFPC architecture-independent binary representation
    -> unsigned MFP container
    -> <package>.mfp
```

Diagnostic and validation output is emitted during the front-end passes. Build
artifacts are written into the project directory.

## Reading order

The subtopics below follow the pipeline. `frontend` covers everything from
manifest loading through type checking; `ir` is the shared hinge; then the path
splits into `binary-representation` (packages) and `native` (executables). The
`flows` topic walks both end to end, and `artifacts`, `modules`, `boundaries`,
and `extending` are quick references.

## See Also

* ./mfb spec language — the MFBASIC language reference
* ./mfb spec package — the `.mfp` container and binary-representation format
* ./mfb spec memory — the runtime value memory model
* ./mfb spec linker — the native object and link pipeline
* ./mfb spec threading — thread compilation and the runtime contract
* ./mfb man — built-in package and function help
