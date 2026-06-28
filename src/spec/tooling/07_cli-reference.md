# CLI Reference

The complete `mfb` command-line surface: every command, its flags, exit codes,
and the structured output of `pkg info`, plus the terminal-rendering rules shared
by the embedded `spec` and `man` help. This topic owns the reimplementable CLI
detail; the language/architecture specs only mention these commands in passing.

The first argument is the command; `mfb help` or no argument prints the usage
block and exits `0`. An unknown command prints `error: unknown command '<cmd>'`
followed by the usage block to stderr and exits `2`.[[src/main.rs:main]]

## Commands and Exit Codes

Every command dispatches from `main`.[[src/main.rs:main]] Exit codes follow one
convention: **2** for argument/usage errors (always printed with the usage
block), **1** for runtime failures, **0** for success. `audit` adds **3**.

| Command | Synopsis | Exit codes |
| --- | --- | --- |
| `help` | `mfb help` (or no args) | 0 |
| `init` | `mfb init <location>` | 0 ok; 2 missing/extra arg; 1 create/write failed |
| `init-pkg` | `mfb init-pkg <location>` | 0 ok; 2 missing/extra arg; 1 create/write failed |
| `build` | `mfb build [flags] [location]` | 0 ok; 2 bad flags; 1 build failed |
| `fmt` | `mfb fmt [--check] [--indent N] [location]` | 0 ok; 2 bad flags; 1 not-formatted (`--check`) or error |
| `doc` | `mfb doc [--out file] [location]` | 0 ok; 2 bad flags; 1 invalid DOC block or error |
| `pkg add` | `mfb pkg add <url>` | 0 ok; 2 usage; 1 failed |
| `pkg info` | `mfb pkg info <package>` | 0 ok; 2 usage; 1 failed |
| `pkg verify` | `mfb pkg verify` | 0 ok; 2 usage (takes no args); 1 failed |
| `pkg publish` | `mfb pkg publish <owner_name> <package>` | 0 ok; 2 usage; 1 failed |
| `pkg doc` | `mfb pkg doc <name-or-path> [--out file]` | 0 ok; 2 usage; 1 failed |
| `repo register` | `mfb repo register <owner_name>` | 0 ok; 2 usage; 1 failed |
| `repo auth` | `mfb repo auth <owner_name>` | 0 ok; 2 usage; 1 failed |
| `audit` | `mfb audit [--format text\|json] [--locked] [path]` | 0 clean; 1 error findings; 2 bad flags; 3 validation failed |
| `man` | `mfb man [package] [function]` | 0 ok; 2 unknown package/function or >2 args |
| `spec` | `mfb spec [topic] [subtopic] [--all] [--width N] [--color\|--no-color]` | 0 ok; 2 unknown topic, bad flag, or >2 positionals |

The usage block printed by `help` is the `USAGE` constant.[[src/main.rs:USAGE]]
`init` writes `project.json` (kind `executable`) + `src/main.mfb`; `init-pkg`
writes `project.json` (kind `package`) + `src/lib.mfb`. Both refuse to overwrite
an existing file (`write_new_file`).[[src/cli/init.rs:write_new_file]]

## `build` Flags

`parse_build_options` parses the flags.[[src/cli/build.rs:parse_build_options]] The
output-mode flags are **mutually exclusive** — a second one yields `mfb build
accepts only one output mode`. With no output flag, `build` validates and emits
the project's primary artifact (`<name>.out` for executable, `<name>.mfp` for
package).

| Flag | Output mode | Artifact / effect |
| --- | --- | --- |
| (none) | `Validate` | `.out` (executable) or `.mfp` (package) |
| `-ast` | `Ast` | `<name>.ast` (parsed AST dump) |
| `-ir` | `Ir` | `<name>.ir` |
| `-br` | `BinaryRepr` | `<name>.hex` (hex dump of this project's MFPC binary representation) |
| `-nir` | `NativeIr` | `<name>.nir` (native IR) |
| `-nplan` | `NativePlan` | `<name>.nplan` |
| `-nobj` | `NativeObjectPlan` | `<name>.nobj` |
| `-ncode` | `NativeCodePlan` | `<name>.ncode` |
| `-target os-arch` / `-target=os-arch` | — | native target instead of host (`BuildTarget::parse`) |
| `--sign owner` / `--sign=owner` | — | sign artifact with owner's repo key; at most one |
| `-app` | — | GUI app-mode runtime; at most one |

`-target` requires a value (`mfb build -target requires os-arch`). `--sign`
requires a value, accepts at most one (`mfb build accepts at most one --sign
option`), and is only honored for the default `Validate` mode (package/executable
builds); combined with a native-output flag it errors with `mfb build --sign is
only supported for package and executable builds`. The signing key is resolved
through `load_build_signing_info`, which cross-checks the local private key
against the repository's signing key/fingerprint.[[src/cli/build.rs:load_build_signing_info]]

`-app` is **executable-only** and requires a native target that supports app mode
(`macos-aarch64` or `linux-aarch64`); otherwise it errors before any lowering
(`mfb build -app requires an executable project` / `mfb build -app requires a
macOS or Linux target`).[[src/cli/build.rs:build_project]] A duplicate `-app` yields
`mfb build accepts at most one -app option`. App mode selects
`NativeBuildMode::LinuxApp`/`MacApp`; console builds use `NativeBuildMode::Console`.

Native intermediate outputs (`-nir`/`-nplan`/`-nobj`/`-ncode`) are **rejected for
package projects** with the `PACKAGE_NATIVE_OUTPUT_UNSUPPORTED` diagnostic; a
package emits only `.mfp`. An unknown `-flag` yields `unknown build option
` `` `<arg>` `` ``; a second positional yields `mfb build accepts at most one
[location]`. The location defaults to `.`; the target defaults to the host.

`build` runs the pipeline parse → resolve → monomorphize → resolve (no DOC
re-validation) → validate entry point → typecheck before emitting any artifact;
any stage failure exits `1`.[[src/cli/build.rs:build_project]] Build-mode and
build-flag *semantics* live in `./mfb spec architecture commands`.

## `fmt`, `doc`, `audit` Flags

`fmt` (`run_fmt_command`) takes `--check`, `--indent N` / `--indent=N` (default
`2`, parsed by `parse_indent` — non-negative integer, else exit 2), and one
optional `[location]` (file or project dir, default `.`).[[src/cli/fmt.rs:run_fmt_command]]
Without `--check` it rewrites files in place and prints `Formatted <path>` per
change; with `--check` it writes nothing, prints `Not formatted: <path>`, and on
any unformatted file emits the `FMT_CHECK_FAILED` diagnostic and exits `1`. A
second positional yields `mfb fmt accepts exactly one [location]` (exit 2). Format
rules: `./mfb spec tooling fmt`.

`doc` (`run_doc_command`) takes `--out <file>` (default `doc.html`) and one
optional `[location]` (default `.`).[[src/cli/doc.rs:run_doc_command]] It renders
HTML from project or single-file source; an invalid DOC block returns exit `1`
(diagnostics already on stderr). Rendering model: `./mfb spec tooling doc-html`.

`audit` parses with `audit::parse_options` (`--format text|json`, default text;
`--locked`; one optional `path`).[[src/audit/mod.rs:parse_options]] An invalid
`--format` or unknown flag exits `2`. `audit::run` exits **3** if project
validation fails, **1** if any finding has `Severity::Error`, else **0**.[[src/audit/mod.rs:run]]
Format/finding catalogue: `./mfb spec tooling audit-format`.

## `pkg` and `repo` Subcommands

`run_pkg_command` matches the subcommand by name.[[src/cli/pkg.rs:run_pkg_command]]
`add <url>` resolves a `file://` `.mfp` URL (only scheme supported; must be
absolute and end `.mfp`), copies it into `packages/`, and records a pinned
dependency in `project.json`. `info <package>` prints the package report (below).
`verify` checks each `project.json` dependency. `publish <owner> <package>`
rebuilds and signs the package then uploads it. `doc <name-or-path> [--out file]`
renders a compiled package's doc section (`run_pkg_doc`, default out
`doc.html`).[[src/cli/pkg.rs:run_pkg_doc]] Each subcommand's arity error and the
fallthrough `unknown pkg command` exit `2`; runtime failures exit `1`.

`run_repo_command` requires exactly one `<owner_name>` for `register` or
`auth`.[[src/cli/repo.rs:run_repo_command]] `register` prints `Registered owner <o>
with auth fingerprint <f>`; `auth` prints `Authenticated owner <o> until <t>`.
The registry protocol, signing, and publish detail are
`./mfb spec package-manager` (coming).

## `pkg verify` Output

`verify_packages` prints one line per declared dependency.[[src/cli/pkg.rs:verify_packages]]
The status comes from `package_dependency_status`: name/ident mismatch →
`Invalid Package`, version mismatch → `Needs Update`, else `OK`; an empty
expected version always matches.[[src/cli/pkg.rs:package_dependency_status]] A
dependency missing both a `packages/<name>.mfp` and a source-package
`packages/<name>/project.json` is `Invalid Package`. The line is formatted by
`package_verify_line`:[[src/cli/pkg.rs:package_verify_line]]

```text
<name> @ <declared-version> : <status>
<name> @ <declared-version> : <status> (<actual-version>)
```

(the `(actual)` suffix appears only when the installed version is known). A
dependency entry that fails to parse prints `<invalid> @ <invalid> : Invalid
Package`.

## `pkg info` Output

`print_package_info` decodes the `.mfp` header (`read_mfp_header`) and binary
representation info (`binary_repr::read_package_info`) and prints fixed sections
in order.[[src/cli/pkg.rs:print_package_info]] Every empty string renders as
`<empty>` (`empty_marker`); the content hash is the lowercase hex of the package
content hash. The layout:

```text
Package: <name>
Ident: <ident>
Version: <version>
Ident Key: <identKey>
Ident Fingerprint: <fp>
Signing Fingerprint: <fp>
Author: <author>
URL: <url>
Path: <path>

Container:
  format: MFP
  version: <containerMajor>.<containerMinor>
  binary representation version: <brMajor>.<brMinor>
  flags: 0x<flags:08x>
  signature type: <unsigned|Ed25519|unknown (n)>
  signature length: <n>
  content hash: <hex>
  binary representation length: <n>

Manifest:
  name / ident / version / ident key / ident fingerprint
  signing fingerprint / author / url

Binary Representation:
  ABI format version: <n>
  types: <n>      constants: <n>   resources: <n>
  functions: <n>  globals: <n>     cleanups: <n>
  imports: <n>    exports: <n>

Exports:
  <KIND> <name>
    sigHash: <hash>
  (or "  <none>")

Package State:
  <LET|MUT> <name> AS <type>
    visibility: <visibility>
    audit: exported mutable package state    ; only when MUT && export

Resource Cleanups:
  <function> cleanup <id>
    pc: <start>..<end>
    resource register: <n>
    close function id: <n>
    audit: records secondary close failure   ; conditional

Imports:
  <package>
    ident / version / pin / flags
    used symbols:
      <name>
        sigHash: <hash>
```

`signature type` is mapped by `signature_type_name`: `0`→`unsigned`,
`1`→`Ed25519`, other→`unknown (n)`.[[src/cli/pkg.rs:signature_type_name]] Export
kinds are mapped by `package_export_kind_name`: `FUNC`, `SUB`, `TYPE`, `UNION`,
`ENUM`.[[src/cli/pkg.rs:package_export_kind_name]] The `.mfp` header format
(magic, container version, signature header, length-prefixed strings) is
`./mfb spec package container-format`.

## `spec` and `man` Terminal Rendering

`show_spec` parses `--all`, `--color`/`--no-color`, `--width N` /
`--width=N`, then up to two positionals (topic, subtopic).[[src/cli/spec.rs:show_spec]]
An unknown `--flag` or a third positional exits `2`; `--all` requires a topic and
cannot be combined with a subtopic. With no positional it prints the topic index;
with one it prints the package overview plus a subtopic listing (or, with `--all`,
the overview followed by every subtopic page separated by a full-width `─`
rule);[[src/cli/spec.rs:print_spec_all]] with two it renders that exact topic page.

Width resolution (`detect_terminal_width`): explicit `--width` (clamped 20..=1000)
→ env `COLUMNS` (clamped) → `TIOCGWINSZ` ioctl on stdout fd 1
(`terminal_width_from_ioctl`, macOS request `0x40087468`, Linux `0x5413`) →
classic **80** fallback (also used when stdout is piped).[[src/cli/spec.rs:detect_terminal_width]]
`parse_spec_width` clamps `--width` to `[20, 1000]` and rejects non-numeric
values.[[src/cli/spec.rs:parse_spec_width]] Color defaults to whether stdout is a
TTY. The render `Style` carries `{ width, color }`.[[src/spec/render.rs:Style]]

Index and subtopic listings are emitted as a two-column GFM table
(`| Topic | Summary |`) fed through the same Markdown renderer so the summary
column reflows to the terminal width instead of running off it; literal `|` in a
cell is escaped (`escape_spec_cell`).[[src/cli/spec.rs:print_spec_listing]]

`show_man` mirrors `spec` but is not width-aware: zero args print the package
index, one arg a package's function/topic listing, two args a single function
page; an unknown package/function or more than two args exits `2`.[[src/cli/man.rs:show_man]]
The `man` listing heading is `TOPICS`/`topic` for the `types` package and
`FUNCTIONS`/`function` otherwise.[[src/cli/man.rs:man_entry_heading]] Within a
package listing, value-reference entries (a synopsis qualified `package::name`
with an `AS <Type>` clause and no argument list, e.g. `math::pi`) are split into
a separate `CONSTANTS` section printed ahead of the `FUNCTIONS`/`TOPICS`
list.[[src/cli/man.rs:is_constant]]

## See Also

* ./mfb spec architecture commands — build modes and build-flag semantics
* ./mfb spec architecture flows — the end-to-end build pipeline these commands drive
* ./mfb spec tooling project-manifest — the `project.json` schema `build`/`fmt`/`audit` validate
* ./mfb spec tooling audit-format — the `mfb audit` JSON schema and finding catalogue
* ./mfb spec tooling fmt — the `mfb fmt` normalization rules
* ./mfb spec tooling doc-html — the `mfb doc` / `pkg doc` HTML rendering model
* ./mfb spec package container-format — the `.mfp` header and signature byte encoding read by `pkg info`
* ./mfb spec diagnostics rule-codes — the diagnostics these commands emit
* ./mfb spec package-manager — registry protocol, signing, and `pkg publish`/`repo` detail (coming)
