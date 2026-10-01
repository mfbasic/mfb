# Commands and Build Modes

The **build modes** the CLI selects, and how each is carried through the
intermediate outputs. This topic owns the modes, not the command surface: the
full command list, every flag, its arity rules, and its exit statuses are owned by
`./mfb spec tooling cli-reference`, which is the single place to change when the
CLI changes.

## Build modes

`mfb build --app` or manifest `"mode": "app"` selects GUI app mode: the executable and native intermediate
outputs target a windowing app runtime instead of the console runtime — AppKit on
macOS, GTK4 on Linux, and Win32 on Windows. Shared lowering treats these modes uniformly
(`NativeBuildMode::is_app`); the target OS selects the toolkit. `--app` is valid
only for executable projects and only when `--target` resolves to a native target
that supports app mode (`macos-aarch64`, `linux-aarch64`, `linux-x86_64`, or `windows-x86_64`); it
is rejected otherwise.

App mode is recorded as the `buildMode` field in `--nir`, `--nplan`, and
`--ncode` output — `"console"`, `"macos-app"`, `"linux-app"`, or `"windows-app"`. The selected mode also controls runtime lowering and platform imports. [[src/target.rs:is_app]]

## Artifact selection

Intermediate-output flags accept single-dash aliases (`-ast`, `-ir`,
`-nir`, and so on); the `--` spelling is canonical. [[src/cli/build/mod.rs:from_flag]]

Distinct output flags may be combined; repeating a flag is an error.
[[src/cli/build/options.rs:parse_build_options]] If no output flag is supplied,
`mfb build` emits:

- Platform-specific executable artifacts under `build/` for `kind = "executable"`;
  see `./mfb spec architecture artifacts`.
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
