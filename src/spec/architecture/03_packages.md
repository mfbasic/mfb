# Package Dependencies

How installed `.mfp` package dependencies are added, verified, and linked into a build.

Package dependency handling is split between `src/main.rs`, `src/binary_repr.rs`,
and `src/target/shared/nir.rs`.

## Installing Packages

`mfb pkg add <url>` currently supports `file://` URLs that point to absolute
`.mfp` files. The command:

1. Reads and validates the MFP header.
2. Copies the package to `packages/<name>.mfp`.
3. Adds a dependency entry to `project.json`.
4. Pins the dependency to the installed package version.

The package entry written to `project.json` includes:

- `name`
- `version`, the installed package's version string
- `pin`, the concrete pinned package version (compared for exact string match)
- `source`, the original URL

Other `pkg` subcommands (`src/main.rs`) round out package management:

- `mfb pkg info <package>` prints metadata from a compiled `.mfp`.
- `mfb pkg publish <owner_name> <package>` builds, signs, and publishes a package
  project under a registered repository owner.
- `mfb pkg doc <name-or-path> [--out file]` renders HTML documentation from a
  compiled package via `src/doc.rs`.

## Verifying Packages

`mfb pkg verify` reads the manifest `packages` array and checks that each
declared package has a matching installed file under `packages/<name>.mfp`.
Pinned dependencies must match the installed package header version.

## Using Packages During Compilation

Executable builds load installed package files before IR lowering. The compiler
reads each package header and exported binary representation ABI metadata, then creates
external function signatures under qualified names such as:

```text
packageName.exportName
```

These signatures are passed into `ir::lower_project_with_external_functions`
so calls to package functions survive lowering with proper function types.[[src/ir.rs:lower_project_with_external_functions]]

For native executable builds, the package's bodies are not left as external
symbols. The native back end decodes each installed package's binary
representation back into IR, prefixes it with a per-package identity, merges it
into the application IR, and rewrites the consumer's `package.symbol` references
to the merged definitions, so package functions flow through the single
`IR → NIR → native` codegen as ordinary functions rather than imports. The
full decode-and-merge mechanic (and the four symbols it uses) is documented in
`./mfb spec architecture binary-representation`.

## See Also

* ./mfb spec architecture binary-representation — the canonical decode-and-merge path
* ./mfb spec package ir-section — the package identity hash derivation
