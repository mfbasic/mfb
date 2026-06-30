# Commands and Build Modes

The build-related CLI commands and the build modes they select.

The CLI supports these build-related commands:

- `mfb init <location>` creates an executable project with `project.json` and
  `src/main.mfb`.
- `mfb init-pkg <location>` creates a package project with `project.json` and
  `src/lib.mfb`.
- `mfb build [location]` validates and emits the primary artifact for the
  project kind.
- `mfb build -ast [location]` writes `<name>.ast`.
- `mfb build -ir [location]` writes `<name>.ir`.
- `mfb build -br [location]` writes `<name>.hex`, a hexadecimal dump of MFPC
  binary representation.
- `mfb build -nir [location]` writes `<name>.nir`.
- `mfb build -nplan [location]` writes `<name>.nplan`.
- `mfb build -nobj [location]` writes `<name>.nobj`.
- `mfb build -ncode [location]` writes `<name>.ncode`.
- `mfb build -target os-arch [location]` selects a native target instead of
  the host target.
- `mfb build -regalloc name [location]` selects the native backend's
  register-allocation strategy. The default is `linear-scan` (liveness-driven,
  with spilling); `bump` selects the byte-identical legacy reference allocator.
  An unknown name is rejected with the list of available strategies.[[src/target/shared/code/regalloc/mod.rs:parse_kind]]
- `mfb build --sign owner [location]` signs the emitted artifact with the
  registered repository owner's key. For package projects this produces a signed
  `.mfp` container; for executable projects it records signing metadata. At most
  one `--sign` may be supplied. Without it, packages are emitted unsigned. The
  on-disk signature-header byte encoding is documented in
  `./mfb spec package container-format`.
- `mfb build -app [location]` selects GUI app mode: the executable and native
  intermediate outputs target a windowing app runtime instead of the console
  runtime — AppKit on macOS, GTK4 on Linux. Shared lowering treats both uniformly
  (`NativeBuildMode::is_app`); the target OS selects the toolkit. `-app` is valid
  only for executable projects and only when `-target` resolves to a native target
  that supports app mode (`macos-aarch64` or `linux-aarch64`); it is rejected
  otherwise. App mode is recorded as the `buildMode` field in `-nir`, `-nplan`,
  and `-ncode` output (`"console"`, `"macos-app"`, or `"linux-app"`).[[src/target.rs:is_app]]

The output flags are mutually exclusive. If no output flag is supplied,
`mfb build` emits:

- `<name>.out` for `kind = "executable"`.
- `<name>.mfp` for `kind = "package"`.

Native intermediate outputs are rejected for package projects. Package projects
are emitted through the package binary representation path instead.

## Formatting

`mfb fmt [--check] [--indent N] [location]` formats every `.mfb` file selected
by the project manifest (or a single `.mfb` file) in place, normalizing block
indentation and keyword capitalization. The formatter is purely lexical
(`src/fmt.rs`): it re-tokenizes raw text to preserve comments, blank lines, and
string contents. `DOC` and `LINK` blocks are re-indented from their own nesting
but keep their text and casing (prose bodies; the contextual `return` in `ABI`
lines). `--indent` sets the indent width (default `2`); `--check` writes nothing
and exits non-zero with an `FMT_CHECK_FAILED` diagnostic when any file is not
already formatted.[[src/fmt.rs:format_source]]

## Other commands

The CLI also exposes non-build commands handled in `src/main.rs`: `mfb help`;
`mfb pkg add|info|verify|publish|doc` (package management, see the `packages`
topic); `mfb repo register|auth` (repository-owner key registration and
authentication); `mfb doc [--out file] [location]` and `mfb pkg doc` (HTML
documentation rendering via `src/doc.rs`); `mfb audit [--format text|json]
[--locked] [path]` (project audit reporting via `src/audit`); `mfb man [package]
[function]` (built-in help, `src/docs/man`); and `mfb spec` (this embedded
specification, `src/docs/spec`). These are not part of the build pipeline.

## See Also

* ./mfb spec package container-format — the on-disk signature-header encoding
