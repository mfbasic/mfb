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
so calls to package functions survive lowering with proper function types.

For native executable builds, the package's bodies are not left as external
symbols. `nir::merge_packages` (`src/target/shared/nir.rs`) decodes each
installed package's binary representation **back into IR**
(`binary_repr::read_package_ir_with_identity`), prefixes every package symbol
with a per-package identity (`ir::prefix_package_symbols`), merges the functions,
types, globals, and constants into the application IR, and rewrites the
consumer's `package.symbol` references to the identity-prefixed definitions
(`ir::apply_package_identity`). Package functions therefore flow through the
single `IR → NIR → native` codegen as ordinary merged functions (emitted under
the normal `_mfb_fn_…` symbol namespace), not as `_mfb_pkg_*` imports. The only
true NIR imports are native `LINK` thunks and platform symbols. This is the same
decode-and-merge path the binary-representation topic describes.
