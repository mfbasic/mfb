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
- `version`, as an exact `=<version>` requirement
- `pin`, as the concrete package version
- `source`, as the original URL

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

For native executable builds, package exports also become NIR imports with
generated symbols:

```text
_mfb_pkg_<package>_<export>
```

For binary representation merging, package binary representation is decoded and appended to the
application binary representation function/type/constant/import/export structures.
