# Commands and Build Modes

The **build modes** the CLI selects, and how each is carried through the
intermediate outputs. This topic owns the modes, not the command surface: the
full command list, every flag, its arity rules, and its exit statuses are owned by
`./mfb spec tooling cli-reference`, which is the single place to change when the
CLI changes.

(This topic used to restate that surface in full. Being a second copy, it drifted:
it never gained `mfb test`, `--version`, `-q`/`-v`, `--app-debug`, `--unsigned`,
or the `machine`/`key`/`org`/`token` commands, and it listed `repo register|auth`
long after `trust`, `link`, and `rotate` shipped. The duplication is the defect
that produced the drift, so it is removed rather than refreshed — bug-338-F1.)

## Build modes

`mfb build --app` selects GUI app mode: the executable and native intermediate
outputs target a windowing app runtime instead of the console runtime — AppKit on
macOS, GTK4 on Linux. Shared lowering treats both uniformly
(`NativeBuildMode::is_app`); the target OS selects the toolkit. `--app` is valid
only for executable projects and only when `--target` resolves to a native target
that supports app mode (`macos-aarch64`, `linux-aarch64`, or `linux-x86_64`); it
is rejected otherwise.

App mode is recorded as the `buildMode` field in `--nir`, `--nplan`, and
`--ncode` output — `"console"`, `"macos-app"`, or `"linux-app"`. That field is
the mode's whole observable footprint in the intermediates, which is what makes
it checkable from a dump. [[src/target.rs:is_app]]

## Artifact selection

Every build flag also accepts an undocumented single-dash alias (`-ast`,
`-target`, `-app`, …) for backwards compatibility; the `--` spelling is
canonical. [[src/cli/build.rs:from_flag]]

The output flags are mutually exclusive. If no output flag is supplied,
`mfb build` emits:

- `build/<name>.out` for `kind = "executable"` — every executable build emits
  into the project's `build/` directory.
- `<name>.mfp` for `kind = "package"`.

Native intermediate outputs are rejected for package projects. Package projects
are emitted through the package binary representation path instead.

## Formatting

`mfb fmt` is not a build mode and does not enter this pipeline: it is a purely
lexical rewrite of source text. Its guarantees, flags, and exit statuses are
owned by `./mfb spec tooling fmt`. [[src/fmt.rs:format_source]]

## Other commands

Everything outside the build pipeline — project scaffolding, package management,
repository/identity, docs, audit, formatting check mode, `mfb test`, and this
specification browser — is enumerated with its flags and exit statuses in
`./mfb spec tooling cli-reference`. [[src/main.rs]]

## See Also

* ./mfb spec package container-format — the on-disk signature-header encoding
