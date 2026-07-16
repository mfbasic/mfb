# CLI Reference

The complete `mfb` command-line surface: every command, its flags, exit codes,
and the structured output of `pkg info`, plus the terminal-rendering rules shared
by the embedded `spec` and `man` help. This topic owns the reimplementable CLI
detail; the language/architecture specs only mention these commands in passing.

The first argument is the command; `mfb help`, `mfb --help`, `mfb -h`, or no
argument prints the usage block and exits `0`. `mfb --version` (or `-V`) prints
the version/build block and exits `0`. An unknown command prints `error: unknown
command '<cmd>'` followed by the usage block to stderr and exits
`2`.[[src/main.rs:main]]

Every flag longer than one character is spelled `--flag`. The single-dash
spellings of the `build`/`test` flags (`-ast`, `-ir`, `-br`, `-nir`, `-nplan`,
`-nobj`, `-ncode`, `-mir`, `-target`, `-regalloc`, `-app`) predate this and
remain accepted, undocumented aliases of the `--` forms; they parse identically
and are not listed below.[[src/cli/build.rs:from_flag]] The single-character
flags `-q`/`-v` keep their single dash (with `--quiet`/`--verbose` long forms).
The diagnostics quoted in this topic name the legacy single-dash spelling
verbatim, whichever form was typed.

The usage block is two-tier: the top-level screen advertises only the common
`pkg` (`add`/`update`/`install`/`verify`) and `repo` (`register`/`auth`)
commands; `mfb pkg --help` and `mfb repo --help` list each family in full, and
`pkg`/`repo` usage errors print those sub-help screens rather than the top-level
one.[[src/main.rs:PKG_HELP]][[src/main.rs:REPO_HELP]]

## Commands and Exit Codes

Every command dispatches from `main`.[[src/main.rs:main]] Exit codes follow one
convention: **2** for argument/usage errors (always printed with the usage
block), **1** for runtime failures, **0** for success. `audit` adds **3**.

| Command | Synopsis | Exit codes |
| --- | --- | --- |
| `help` | `mfb help`, `mfb --help`, `mfb -h` (or no args) | 0 |
| `--version` | `mfb --version` (or `-V`) | 0 |
| `init` | `mfb init <location>` | 0 ok; 2 missing/extra arg; 1 create/write failed |
| `init-pkg` | `mfb init-pkg <location>` | 0 ok; 2 missing/extra arg; 1 create/write failed |
| `build` | `mfb build [flags] [location]` | 0 ok; 2 bad flags; 1 build failed |
| `test` | `mfb test [--coverage] [--target os-arch] [--regalloc name] [location]` | 0 all cases passed; 1 a case failed or build error; 2 bad flags |
| `fmt` | `mfb fmt [--check] [--indent N] [location]` | 0 ok; 2 bad flags; 1 not-formatted (`--check`) or error |
| `doc` | `mfb doc [--out file] [location]` | 0 ok; 2 bad flags; 1 invalid DOC block or error |
| `pkg add` | `mfb pkg add <file://…​.mfp or <owner>#<pkg>[@version]>` | 0 ok; 2 usage; 1 failed |
| `pkg info` | `mfb pkg info <package>` | 0 ok; 2 usage; 1 failed |
| `pkg verify` | `mfb pkg verify [--proof]` | 0 ok; 2 usage; 1 failed |
| `pkg validate` | `mfb pkg validate <package>` | 0 valid; 2 usage; 1 invalid or failed |
| `pkg publish` | `mfb pkg publish <owner_name> <package>` | 0 ok; 2 usage; 1 failed |
| `pkg update` | `mfb pkg update [location]` | 0 ok; 2 usage; 1 conflict or failed |
| `pkg install` | `mfb pkg install [location]` | 0 ok; 2 usage; 1 stale lock or failed |
| `pkg check-abi` | `mfb pkg check-abi [location]` | 0 compatible; 2 usage; 1 breaking or failed |
| `pkg release-state` | `mfb pkg release-state <available\|deprecated\|yanked> [version]` | 0 ok; 2 usage; 1 failed |
| `pkg doc` | `mfb pkg doc <name-or-path> [--out file]` | 0 ok; 2 usage; 1 failed |
| `repo register` | `mfb repo register <owner_name>` | 0 ok; 2 usage; 1 failed |
| `repo auth` | `mfb repo auth <owner_name>` | 0 ok; 2 usage; 1 failed |
| `repo link` | `mfb repo link [--start] <owner_name>` | 0 ok; 2 usage; 1 failed |
| `repo trust` | `mfb repo trust <registry-id> <root-fingerprint>` | 0 ok; 2 usage; 1 failed |
| `org grant` / `org remove` | `mfb org grant <org> <member> <role> [--as <grantor>]` | 0 ok; 2 usage; 1 failed |
| `token issue` / `token revoke` | `mfb token issue <owner> <scope> <ttl-seconds>` | 0 ok; 2 usage; 1 failed |
| `pkg transfer` / `pkg transfer-accept` | `mfb pkg transfer <ident> <to-owner>` | 0 ok; 2 usage; 1 failed |
| `machine revoke` | `mfb machine revoke <owner_name> <auth-fingerprint>` | 0 ok; 2 usage; 1 failed |
| `key rotate` | `mfb key rotate <owner_name>` | 0 ok; 2 usage; 1 failed |
| `audit` | `mfb audit [--format text\|json] [--locked] [path]` | 0 clean; 1 error findings; 2 bad flags; 3 validation failed |
| `man` | `mfb man [package] [function]` | 0 ok; 2 unknown package/function or >2 args |
| `spec` | `mfb spec [topic] [subtopic] [--all] [--width N] [--color\|--no-color]` | 0 ok; 2 unknown topic, bad flag, or >2 positionals |

The usage block printed by `help` is the `USAGE` constant.[[src/main.rs:USAGE]]
`init` writes `project.json` (kind `executable`) + a `main.mfb` under the `src`
source root; `init-pkg` writes `project.json` (kind `package`) + a `lib.mfb`
under `src`. Both refuse to overwrite
an existing file (`write_new_file`).[[src/cli/init.rs:write_new_file]]

## `--version`

`mfb --version` prints exactly three lines and exits `0`:

```
MFBasic Compiler <crate version>
<UTC build date/time>
Commit: <short-hash>   |   Local Development
```

Line 1 is `CARGO_PKG_VERSION`. Line 2 is the build time as
`YYYY-MM-DD HH:MM:SS UTC`, or `unknown build date` if it was not
captured.[[src/cli/version.rs:format_version]]

Line 3 states provenance. It is `Commit: <short-hash>` **only** when the build
tree was both **clean** (`git status --porcelain` empty — no modified, staged,
or untracked path) and **pushed** (`git rev-list @{u}..HEAD` empty — no commit
the upstream lacks). Every other case is `Local Development`: a dirty tree, an
unpushed or upstream-less commit, a tree with no `.git`, or a host with no
`git`. The line can therefore understate provenance but never claim a commit a
reader could not fetch.[[build.rs:emit_build_metadata]]

The metadata is captured at **build** time (`MFB_BUILD_DATE`, `MFB_COMMIT`,
`MFB_LOCAL_DEV`), not resolved at runtime — the shipped binary may run far from
the tree it was built in, and a missing `.git` never fails the build. Because
cargo caches build-script output, the timestamp is when the build script last
re-ran rather than the instant of the final link.[[build.rs:watch_build_state]]

## `build` Flags

`parse_build_options` parses the flags.[[src/cli/build.rs:parse_build_options]] The
output-mode flags **combine**: any number of distinct output flags may be given
in one invocation, and every requested artifact file is written from a single
shared front-end pass, in flag order (repeating the same flag — in either
spelling — yields `mfb build got duplicate output flag `<arg>``, echoing the
argument as given). Each artifact is a `<name>.<ext>` file in
the project directory — identical byte-for-byte to the file a single-flag
invocation writes. With no output flag, `build` validates and emits the
project's primary artifact (`<name>/<name>.out` for executable — every executable
build emits into its own `<name>/` output directory — or `<name>.mfp` for
package).

| Flag | Output mode | Artifact / effect |
| --- | --- | --- |
| (none) | full build (empty `outputs`) | `.out` (executable) or `.mfp` (package) |
| `--ast` | `Ast` | `<name>.ast` (parsed AST dump) |
| `--ir` | `Ir` | `<name>.ir` |
| `--br` | `BinaryRepr` | `<name>.hex` (hex dump of this project's MFPC binary representation) |
| `--nir` | `NativeIr` | `<name>.nir` (native IR) |
| `--nplan` | `NativePlan` | `<name>.nplan` |
| `--nobj` | `NativeObjectPlan` | `<name>.nobj` |
| `--ncode` | `NativeCodePlan` | `<name>.ncode` |
| `--mir` | `Mir` | `<name>.mir` (target-neutral machine IR, virtual registers, no `target`/`arch`) |
| `--regalloc <bump,linear-scan>` / `--regalloc=…` | — | register-allocation strategy; default `linear-scan`. `bump` is the byte-identical reference oracle |
| `--target os-arch` / `--target=os-arch` | — | native target instead of host (`BuildTarget::parse`) |
| `--sign owner` / `--sign=owner` | — | sign the artifact as `owner` (one-off key + proof + attestation); at most one |
| `--app` | — | GUI app-mode runtime; at most one |
| `-q` / `--quiet` | — | print only the `Wrote … to` artifact line and diagnostics |
| `-v` / `--verbose` | — | additionally print a `phase <name> <N>ms` line per front-end stage |

`--target` requires a value (`mfb build -target requires os-arch`). `--sign`
requires a value, accepts at most one (`mfb build accepts at most one --sign
option`), and is only honored when no output flag is given (package/executable
builds); combined with any output flag it errors with `mfb build --sign is
only supported for package and executable builds`. Signing requires the
repository to be reachable: the build reads the local
**ident** key, generates a **one-off signing keypair**, fetches a server
**attestation** via `POST /signing`, mints the ident-signed **proof**, and
threads the bundle to the package writer; the one-off private key is discarded
with the build. The signed ident is the manifest `ident` (which must belong to
`owner`), else `<owner>#<name>`. See `./mfb spec package-manager signing`.
[[src/cli/build.rs:load_build_signing_info]]

`--app` is **executable-only** and requires a native target that supports app mode
(`macos-aarch64`, `linux-aarch64`, or `linux-x86_64`); otherwise it errors before any lowering
(`mfb build -app requires an executable project` / `mfb build -app requires a
macOS or Linux target`).[[src/cli/build.rs:build_project]] A duplicate `--app` yields
`mfb build accepts at most one -app option`. App mode selects
`NativeBuildMode::LinuxApp`/`MacApp`; console builds use `NativeBuildMode::Console`.

Native intermediate outputs (`--nir`/`--nplan`/`--nobj`/`--ncode`/`--mir`) are **rejected
for package projects** with the `PACKAGE_NATIVE_OUTPUT_UNSUPPORTED` diagnostic; a
package emits only `.mfp`. The `--regalloc` flag requires a value (`mfb build
-regalloc requires a strategy name`) and rejects an unknown one (`unknown
-regalloc strategy`). An unknown `-flag` yields `unknown build option
` `` `<arg>` `` ``; a second positional yields `mfb build accepts at most one
[location]`. The location defaults to `.`; the target defaults to the host.

`build` runs the pipeline parse → resolve → monomorphize → resolve (no DOC
re-validation) → validate entry point → syntaxcheck before emitting any artifact;
any stage failure exits `1`.[[src/cli/build.rs:build_project]] Build-mode and
build-flag *semantics* live in `./mfb spec architecture commands`.

**Verbosity** (`Verbosity`/`Reporter`) is orthogonal to the output mode and
never reaches codegen, so the emitted artifact bytes are identical at every
level.[[src/cli/build.rs:Reporter]] The default (`Normal`) prints one
deterministic context line to **stderr** before the pipeline runs — `Building
<name> (<kind>) for <target>` — followed by the `Wrote … to` artifact line on
**stdout**. `-q`/`--quiet` suppresses the summary, restoring an artifact-line-only
output. `-v`/`--verbose` additionally prints a `phase <name> <N>ms` timing line
(integer milliseconds, stderr) for each front-end stage — `parse`, `resolve`,
`verify`, `codegen+link` — as a lightweight build profiler. The two flags are
mutually exclusive (`mfb build accepts at most one of -q / -v`). Only the
`println!`/`eprintln!` progress is level-gated; the timing brackets always run so
`-v` and the default take an identical path into codegen. `mfb test`, `mfb pkg
publish`, and `mfb pkg check-abi` run the build quietly (their own report is the
output; the summary would be noise and, via `<target>`, non-portable across
machines).

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
dependency in `project.json` — for a **signed** package the dependency entry
also pins the header `identKey` on this first add (trust-on-first-use); the
pin, never the file-embedded key, is the trust anchor every later build
verifies against.[[src/cli/pkg.rs:add_package]] `info <package>`
prints the package report (below). `verify` checks each `project.json`
dependency. `validate <package>` checks an **existing** `.mfp` — "is this
package correct?" (below). `publish <owner> <package>` rebuilds and signs the
package then uploads it. `doc <name-or-path> [--out file]` renders a compiled
package's doc section (`run_pkg_doc`, default out
`doc.html`).[[src/cli/pkg.rs:run_pkg_doc]] Each subcommand's arity error and the
fallthrough `unknown pkg command` exit `2`; runtime failures exit `1`.

`run_repo_command` handles `register`, `auth`, and `link` (each scoped to one
`<owner_name>`).[[src/cli/repo.rs:run_repo_command]] `register` prints
`Registered owner <o> with auth fingerprint <f> and ident fingerprint <f>`;
`auth` prints `Authenticated owner <o> until <t>`. `link --start <owner>` (old
machine, needs a session) displays a one-time pairing code; `link <owner>`
(new machine) reads the code from stdin, registers this machine's own auth
key, and installs the decrypted ident keypair — the machine is then a full
equal. `mfb machine revoke <owner> <auth-fingerprint>` revokes a lost
machine's auth key with an ident-signed request (no session needed; requires
the ident key on this machine).[[src/cli/repo.rs:run_machine_command]]
`mfb key rotate <owner>` rotates the account ident: the new key is chained to
the old by an old-ident signature, consumers follow the chain via
`pkg verify`, and other linked machines must re-link.[[src/cli/repo.rs:run_key_command]]
The registry protocol, signing, and publish detail are
`./mfb spec package-manager repository-protocol`.

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
<name> @ <declared-version> : <status> (<actual-version>) [<trust-state>]
```

(the `(actual)` suffix appears only when the installed version is known). A
dependency entry that fails to parse prints `<invalid> @ <invalid> : Invalid
Package`. Compiled `.mfp` dependencies additionally get their trust state —
`[Verified]`, `[Unsigned]`, or `[Tampered]` — verified against
the dependency's pinned `identKey`; source-package dependencies get no state
suffix.[[src/cli/pkg.rs:verify_packages]]

With `--proof`, each Verified dependency additionally needs a
transparency-log inclusion proof for its publish entry, verified against the
signed, rollback-checked checkpoint; success appends
`(log index <n> ⊂ checkpoint size <s>)` to the line and a missing/unverifiable
proof appends `(no publish proof)` and fails the command.
`mfb pkg publish` prints `Publish logged at index <n> (leaf <hex>)` followed by
`Inclusion verified against checkpoint (size <s>, root <hex>)`, refusing to
upload at all if the checkpoint fetch detects a rollback or fork
(`REGISTRY_LOG_ROLLBACK`).[[src/cli/pkg.rs:publish_package_project]]

## `pkg validate` Output

`validate_package_file` resolves `<package>` like `pkg doc` (a direct `.mfp`
path, or `packages/<name>.mfp`) and prints one check line per verifiable link
of the trust chain, then `result: valid` or `result: INVALID (<n>
failed check(s))` (exit 1).[[src/cli/pkg.rs:validate_package_file]]

```text
Package validation report for <path>:
  container: OK (v1.0)
  ident: <ident>
  version: <version>
  signature type: <unsigned|Ed25519>
  payload hash: OK
  package signature: OK (signingKey <fp>)
  proof: OK (identKey <fp>)
  attestation: OK (repoFingerprint <fp>)
  ident pin: OK | <not declared in project.json>
  result: valid
```

An unsigned package reports `trust chain: <none> (unsigned package)` after the
payload-hash check. The proof and package signature are checked against the
**embedded** keys (internal consistency — validate answers "is this package
correct?", not "do I trust this publisher"); the attestation requires the
pinned `server.pub`, and the `ident pin` line compares against the working
project's pinned `identKey` when the package is declared there. This command
is not a pre-publish step; nothing is uploaded.

`pkg info` decodes the `.mfp` header and binary
representation info and prints fixed sections
in order.[[src/cli/pkg.rs:print_package_info]][[src/binary_repr/mod.rs:read_package_info]] Every empty string renders as
`<empty>` (`empty_marker`); the content hash is the lowercase hex of the package
content hash. The layout:

```text
Package: <name>
Ident: <ident>
Version: <version>
Ident Key: <identKey>
Signing Key: <signingKey>
Proof: <proof JSON or <none>>
Attestation: <attestation JSON or <none>>
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
  package binary hash: <hex>
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
TTY. The render `Style` carries `{ width, color }`.[[src/docs/render.rs:Style]]

Index and subtopic listings are emitted as a two-column GFM table
(`| Topic | Summary |`) fed through the same Markdown renderer so the summary
column reflows to the terminal width instead of running off it; literal `|` in a
cell is escaped (`escape_spec_cell`).[[src/cli/spec.rs:print_spec_listing]]

`show_man` mirrors `spec` but is not width-aware: zero args print the package
index, one arg a package's function/topic listing, two args a single function
page; an unknown package/function or more than two args exits `2`.[[src/cli/man.rs:show_man]]
The `man` listing heading is `TOPICS`/`topic` for the `types` package,
`COMPARISONS`/`language` for the `tour` package, and `FUNCTIONS`/`function`
otherwise.[[src/cli/man.rs:man_entry_heading]] Within a
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
* ./mfb spec package-manager — registry protocol, signing, and `pkg publish`/`repo` detail
