# End-to-End Build Flows

The end-to-end build sequences for executable and package projects.

The two end-to-end flows share the front end and diverge after IR: executables
go through the native back end, packages through the binary representation path.

## Source to Executable

For an executable project, `mfb build` performs this sequence:[[src/cli/build.rs:build_project]]

1. Parse command-line options and select target.
2. Read and validate `project.json`.
3. Determine project kind, defaulting to `executable`.
4. Parse all `.mfb` source files from manifest roots.
5. Resolve the parsed AST.
6. Monomorphize the AST.
7. Resolve the concrete AST.
8. Validate the executable entry point.
9. Source-syntax check the concrete AST — only lowering-erased
   source syntax (named-argument binding, EXIT/inline-TRAP boundaries, lambda
   capture escape, package metadata); all semantic rules are enforced later on
   the IR.
10. Read installed package files from `packages/<name>.mfp`.
11. Read package export signatures.
12. Lower the concrete AST to IR with external package function types, then run
    IR semantic verification — the single source of truth for
    every semantic rule, over both source-lowered and decoded-package IR.
13. Select the native backend for the requested target.
14. Validate backend support.
15. Lower IR to NIR.
16. Validate NIR and backend runtime capabilities.
17. Lower NIR to a native plan.
18. Validate the native plan.
19. Lower the native plan to an OS object plan for validation or `--nobj`.
20. Lower NIR and the native plan to a native code plan.
21. Validate the native code plan.
22. Encode AArch64 text, data, symbols, relocations, and imports.
23. Link/write the OS executable container.
24. Mark the output executable.
25. Print the output path(s).

The default output file is:

```text
<project>/<project-name>/<project-name>.out          (macOS)
<project>/<project-name>/<project-name>-glibc.out    (Linux)
<project>/<project-name>/<project-name>-musl.out     (Linux)
```

Every executable build emits into its own `<project-name>/` output directory, so
the executable and the `vendor/` directory its RPATH points at (see
`./mfb spec language native-libraries`) move as a single unit.

Linux builds always emit both flavor outputs in a single `mfb build` run, into the
one output directory.

## Source to Package

For a package project, `mfb build` performs this sequence:

1. Parse command-line options.
2. Read and validate `project.json`.
3. Determine project kind as `package`.
4. Parse all `.mfb` source files from manifest roots.
5. Resolve the parsed AST.
6. Monomorphize the AST.
7. Resolve the concrete AST.
8. Skip executable entry-point selection.
9. Source-syntax check the concrete AST — lowering-erased source
   syntax only; semantic rules are enforced later on the IR.
10. Read installed package files from `packages/<name>.mfp` and their export
    signatures (packages may depend on other packages).
11. Lower the concrete AST to IR with external package function types, then run
    IR semantic verification over the lowered IR.
12. Build binary representation metadata from the manifest.
13. Lower IR to MFPC package binary representation.
14. Validate package metadata and MFPC payload magic.
15. Wrap binary representation in an MFP container — signed when `--sign` is
    given, otherwise unsigned.
16. Write the package file.
17. Print the output path.

The default output file is:

```text
<project>/<package-name>.mfp
```

Package projects do not support native intermediate outputs. Use plain
`mfb build` for `.mfp` emission or `--ast`, `--ir`, and `--br` for front-end and
binary representation inspection.

## See Also

* ./mfb spec architecture commands — the `mfb build` command and output modes these flows drive
* ./mfb spec architecture frontend — the shared parse/resolve/monomorphize/verify front end both flows enter
* ./mfb spec linker pipeline — the native back-end stage sequence the executable flow feeds
* ./mfb spec package binary-representation — the `.mfp` payload the package flow emits
