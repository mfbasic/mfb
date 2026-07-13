# Package Dependencies

How installed `.mfp` package dependencies are added, verified, and linked into a build.

Package dependency handling is split across the CLI, the binary-representation
layer, and the native-IR layer.[[src/main.rs]][[src/binary_repr/]][[src/target/shared/nir/]]

## Installing Packages

`mfb pkg add <target>` accepts either a `file://` URL pointing to an absolute
`.mfp` file or an `<owner>#<package>[@version]` registry ident (resolved and
downloaded over the repository protocol, `./mfb spec package-manager repository-protocol`).
For the `file://` form the command:

1. Reads and validates the MFP header.
2. Copies the package to `packages/<name>.mfp`.
3. Adds a dependency entry to `project.json`.
4. Pins the dependency to the installed package version.

The package entry written to `project.json` includes:

- `name`
- `version`, the installed package's version string
- `pin`, the concrete pinned package version (compared for exact string match)
- `source`, the original URL

Other `pkg` subcommands round out package management:[[src/main.rs]]

- `mfb pkg info <package>` prints metadata from a compiled `.mfp`.
- `mfb pkg publish <owner_name> <package>` builds, signs, and publishes a package
  project under a registered repository owner.
- `mfb pkg doc <name-or-path> [--out file]` renders HTML documentation from a
  compiled package.[[src/doc.rs]]

## Verifying Packages

`mfb pkg verify` reads the manifest `packages` array and checks that each
declared package has a matching installed file under `packages/<name>.mfp`.
Pinned dependencies must match the installed package header version.

### Verify Status Model

For each declared dependency, the verifier locates the installed
package in one of two forms and reports a status:

1. A compiled package at `packages/<name>.mfp`. Its MFP header (`name`,
   `ident`, `version`) is read and compared against the dependency.
2. A source package at `packages/<name>/project.json`. The `name`, `version`,
   and `ident` fields are read from that manifest and compared against the
   dependency. If `ident` is absent it defaults to the manifest's `name`.

The compiled `.mfp` file is checked first; the source-package manifest is the
fallback. If neither exists, the status is `InvalidPackage`.[[src/cli/pkg.rs:verify_package_dependency]]

The dependency-status check produces one of three outcomes:[[src/cli/pkg.rs:package_dependency_status]]

- `InvalidPackage` — the installed `name` does not equal the declared name, the
  installed package could not be read or parsed, or both sides carry a non-empty
  `ident` and they differ. (An empty `ident` on either side is not a mismatch.)
- `NeedsUpdate` — name and ident agree, but the version does not match.
- `Ok` — name, ident, and version all agree.

Version matching is **exact string** comparison: an expected version matches
only when it is empty (no constraint) or byte-for-byte equal to the installed
version. Range syntax such as `^1.2.3` or `~1.2.3` is treated as a literal
string and therefore never matches a concrete version like `1.9.0` — it yields
`NeedsUpdate`.[[src/cli/pkg.rs:package_version_matches]] For a non-empty declared
version the `pin` flag does not change the result. The two paths diverge only for
an *empty* declared version: unpinned, an empty version is "no constraint" and
matches (`Ok`); pinned, an empty version can never equal an installed one and so
yields `NeedsUpdate`.[[src/cli/pkg.rs:package_dependency_status]]

The `pin` flag is enforced separately, during the build's binary-representation
merge of compiled packages. There, a pinned dependency whose `version` differs
from the installed `.mfp` header version is a hard error
(`package \`<name>\` is pinned to version <v>, but installed package is version
<w>`), aborting the build rather than reporting a status.[[src/manifest/package.rs:installed_package_files]]

## Using Packages During Compilation

Executable builds load installed package files before IR lowering. The compiler
reads each package header and exported binary representation ABI metadata, then creates
external function signatures under qualified names such as:

```text
packageName.exportName
```

These signatures are passed into IR lowering
so calls to package functions survive lowering with proper function types.[[src/ir/lower.rs:lower_project_with_external_functions]]

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
* ./mfb spec tooling project-manifest — the `project.json` `packages` array, `pin`, and version fields that `mfb pkg verify` reads
