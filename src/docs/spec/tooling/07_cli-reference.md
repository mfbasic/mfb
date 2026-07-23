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
and are not listed below.[[src/cli/build/mod.rs:from_flag]] The single-character
flags `-q`/`-v` keep their single dash (with `--quiet`/`--verbose` long forms).
The diagnostics quoted in this topic name the legacy single-dash spelling
verbatim, whichever form was typed.

Help is **per command**, not two-tier. The top-level screen groups the commands
and advertises only the common `pkg` (`add`/`update`/`install`/`verify`) and
`repo` (`register`/`auth`) members, and eleven commands then carry a `--help`
screen of their own: `init`, `init-pkg`, `pkg`, `repo`, `build`, `test`, `fmt`,
`audit`, `doc`, `man`, and `spec`. A usage error in one of those prints that
command's screen rather than the top-level one.
[[src/main.rs:PKG_HELP]] [[src/main.rs:BUILD_HELP]] [[src/main.rs:USAGE]]

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
| `pkg add` | `mfb pkg add <file://…​.mfp or <owner>#<pkg>[@version]> [--pin\|--no-pin]` | 0 ok; 2 usage; 1 failed |
| `pkg info` | `mfb pkg info <package>` | 0 ok; 2 usage; 1 failed |
| `pkg verify` | `mfb pkg verify [--proof]` | 0 ok; 2 usage; 1 failed |
| `pkg validate` | `mfb pkg validate <package>` | 0 valid; 2 usage; 1 invalid or failed |
| `pkg update` | `mfb pkg update [<owner>#<pkg>[@version]] [--pin\|--no-pin] [--yes]` | 0 ok; 2 usage; 1 conflict or failed |
| `pkg remove` | `mfb pkg remove <owner>#<pkg> [--yes]` | 0 ok; 2 usage; 1 failed |
| `pkg install` | `mfb pkg install [location]` | 0 ok (incl. a warned ABI-floor drift); 2 usage; 1 unrecoverable lock drift or failed |
| `pkg doc` | `mfb pkg doc <name-or-path> [--out file]` | 0 ok; 2 usage; 1 failed |
| `repo register` | `mfb repo register <owner_name>` | 0 ok; 2 usage; 1 failed |
| `repo auth` | `mfb repo auth <owner_name>` | 0 ok; 2 usage; 1 failed |
| `repo link` | `mfb repo link [--start] <owner_name>` | 0 ok; 2 usage; 1 failed |
| `repo trust` | `mfb repo trust <registry-id> <root-fingerprint>` | 0 ok; 2 usage; 1 failed |
| `repo publish` | `mfb repo publish <owner_name> [path]` | 0 ok; 2 usage; 1 failed |
| `repo check-abi` | `mfb repo check-abi [location]` | 0 compatible; 2 usage; 1 breaking or failed |
| `repo release-state` | `mfb repo release-state <available\|deprecated\|yanked> [version]` | 0 ok; 2 usage; 1 failed |
| `repo transfer` / `repo transfer-accept` | `mfb repo transfer <ident> <to-owner>` | 0 ok; 2 usage; 1 failed |
| `org grant` / `org remove` | `mfb org grant <org> <member> <role> [--as <grantor>]` | 0 ok; 2 usage; 1 failed |
| `token issue` / `token revoke` | `mfb token issue <owner> <scope> <ttl-seconds>` | 0 ok; 2 usage; 1 failed |
| `machine revoke` | `mfb machine revoke <owner_name> <auth-fingerprint>` | 0 ok; 2 usage; 1 failed |
| `key rotate` | `mfb key rotate <owner_name>` | 0 ok; 2 usage; 1 failed |
| `audit` | `mfb audit [--format text\|json] [--locked] [path]` | 0 clean; 1 error findings; 2 bad flags; 3 validation failed |
| `man` | `mfb man [package] [function] [--all]` | 0 ok; 2 unknown package/function, `--all` with a function, or >2 positionals |
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

`parse_build_options` parses the flags.[[src/cli/build/options.rs:parse_build_options]] The
output-mode flags **combine**: any number of distinct output flags may be given
in one invocation, and every requested artifact file is written from a single
shared front-end pass, in flag order (repeating the same flag — in either
spelling — yields `mfb build got duplicate output flag `<arg>``, echoing the
argument as given). Each artifact is a `<name>.<ext>` file in
the project directory — identical byte-for-byte to the file a single-flag
invocation writes. With no output flag, `build` validates and emits the
project's primary artifact (`build/<name>.out` for executable — every executable
build emits into the project's `build/` directory — or `<name>.mfp` for
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
| `--app-debug` | — | app mode, keeping the intermediate `build/<name>.AppDir` (Linux); implies `--app`; at most one |
| `--unsigned` | — | permit unsigned dependencies whose source is **not** local (see below) |
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
[[src/cli/build/signing.rs:load_build_signing_info]]

`--unsigned` relaxes exactly one check, and only in one direction. An unsigned
dependency whose source is local (`file:`/`local:`, or no source at all) is
**always** permitted; without this flag an unsigned dependency pulled from a
remote or registry source is refused — "package `<name>` is unsigned but its
source is not local; pass --unsigned to allow it". The flag makes that case
permitted too. It is the opt-out for the audit-1 PKG-01 supply-chain check, so it
weakens a security control and nothing else: it does not disable signature
*verification* of packages that do carry one.
[[src/cli/build/packages.rs:verify_and_report_packages]]

`--app` is **executable-only** and requires a native target that supports app mode
(`macos-aarch64`, `linux-aarch64`, or `linux-x86_64`); otherwise it errors before any lowering
(`mfb build -app requires an executable project` / `mfb build -app requires a
macOS or Linux target`).[[src/cli/build/mod.rs:build_project]] A duplicate `--app` yields
`mfb build accepts at most one -app option`. App mode selects
`NativeBuildMode::LinuxApp`/`MacApp`; console builds use `NativeBuildMode::Console`.

**Output shape per target.** A macOS `--app` build emits a single
`build/<name>.app` bundle. A Linux `--app` build emits **two** directly
executable AppImages (mode 0755) — `build/<name>-glibc.AppImage` and
`build/<name>-musl.AppImage`, one per libc world, mirroring the console build's
two flavored `.out` files — and no console `.out`; the intermediate AppDirs the
seals consume are deleted. Each AppImage is single-libc, not a fat binary: the
musl one needs a musl GTK4 host (Alpine's `gtk4.0`), the glibc one a glibc GTK4
host. `--app-debug` is the same build with both AppDirs retained beside the
AppImages, for inspecting the payloads that went in — the AppImage bytes are
identical either way. `--app-debug` **implies
`--app`**, so `--app --app-debug` is accepted and means the same thing; a
duplicate yields `mfb build accepts at most one --app-debug option`. On
`macos-aarch64` the flag is accepted and changes nothing, because a `.app` is a
directory and has no intermediate to keep — a flag that changed a build's
*validity* by target would be worse than one that changes nothing.
[[src/target.rs:finalize_app_bundle]]

The project `icon` (see `./mfb spec tooling project-manifest`) applies to Linux
app builds as well as macOS: it is rendered to the freedesktop hicolor PNG sizes
inside the AppDir, under the same "must decode and be exactly 1024×1024" rule the
`.icns` pipeline enforces.

Native intermediate outputs (`--nir`/`--nplan`/`--nobj`/`--ncode`/`--mir`) are **rejected
for package projects** with the `PACKAGE_NATIVE_OUTPUT_UNSUPPORTED` diagnostic; a
package emits only `.mfp`. The `--regalloc` flag requires a value (`mfb build
-regalloc requires a strategy name`) and rejects an unknown one (`unknown
-regalloc strategy`). An unknown `-flag` yields `unknown build option
` `` `<arg>` `` ``; a second positional yields `mfb build accepts at most one
[location]`. The location defaults to `.`; the target defaults to the host.

`build` runs the pipeline parse → resolve → monomorphize → resolve (no DOC
re-validation) → validate entry point → syntaxcheck before emitting any artifact;
any stage failure exits `1`.[[src/cli/build/mod.rs:build_project]] Build-mode and
build-flag *semantics* live in `./mfb spec architecture commands`.

**Verbosity** (`Verbosity`/`Reporter`) is orthogonal to the output mode and
never reaches codegen, so the emitted artifact bytes are identical at every
level.[[src/cli/build/mod.rs:Reporter]] The default (`Normal`) prints one
deterministic context line to **stderr** before the pipeline runs — `Building
<name> (<kind>) for <target>` — followed by the `Wrote … to` artifact line on
**stdout**. `-q`/`--quiet` suppresses the summary, restoring an artifact-line-only
output. `-v`/`--verbose` additionally prints a `phase <name> <N>ms` timing line
(integer milliseconds, stderr) for each front-end stage — `parse`, `resolve`,
`verify`, `codegen+link` — as a lightweight build profiler. The two flags are
mutually exclusive (`mfb build accepts at most one of -q / -v`). Only the
`println!`/`eprintln!` progress is level-gated; the timing brackets always run so
`-v` and the default take an identical path into codegen. `mfb test`, `mfb repo
publish`, and `mfb repo check-abi` run the build quietly (their own report is the
output; the summary would be noise and, via `<target>`, non-portable across
machines).

## `fmt`, `doc`, `audit` Flags

`fmt` (`run_fmt_command`) takes `--check`, `--indent N` / `--indent=N` (default
`2`, parsed by `parse_indent` — an integer in `0..=256`, else exit 2), and one
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
`add <target> [--pin|--no-pin]` takes either a `file://` `.mfp` URL (only scheme
supported; must be absolute and end `.mfp`) or an `<owner>#<package>[@version]`
registry ident. A `file://` add copies the file into `packages/` and records a
**pinned** dependency in `project.json` — for a **signed** package the dependency
entry also pins the header `identKey` on this first add (trust-on-first-use); the
pin, never the file-embedded key, is the trust anchor every later build
verifies against.[[src/cli/pkg.rs:add_package]] A `file://` add of a package that
**vendors native libraries** is refused: there is no registry to fetch the
library bytes from, so it would install in a silently unusable state.

`add` **resolves before it mutates**: the proposed `project.json` is resolved
first, and only a successful resolution writes `project.json`, `mfb.lock` and
`packages/`. A resolution failure — an unpublished anchor version, a diamond
conflict — leaves all three byte-identical.[[src/cli/resolve.rs:apply_manifest_change]]
Because `add` writes the lock, `mfb pkg install` runs immediately afterwards
with no intervening `mfb pkg update`.

`info <package>`
prints the package report (below). `verify` checks each `project.json`
dependency. `validate <package>` checks an **existing** `.mfp` — "is this
package correct?" (below). `doc <name-or-path> [--out file]` renders a compiled
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
`repo publish <owner> [path]` rebuilds and signs the package, uploads any
vendored native-library blobs the registry does not already have, then uploads
the package itself; `[path]` defaults to the current directory.
`mfb key rotate <owner>` rotates the account ident: the new key is chained to
the old by an old-ident signature, consumers follow the chain via
`pkg verify`, and other linked machines must re-link.[[src/cli/repo.rs:run_key_command]]
The registry protocol, signing, and publish detail are
`./mfb spec package-manager repository-protocol`.

## `pkg add` Pin Inference

Whether a dependency is **pinned** (locked to one version) or **floating**
(free to resolve upward) is inferred from the invocation, and either flag
overrides the inference. The rule: *an explicit `@version` implies a pin; an
explicit flag always wins.*[[src/cli/pkg.rs:infer_pin]]

| Invocation | `version` written | `pin` written | Printed suffix |
|---|---|---|---|
| `mfb pkg add alice#shape` | newest floating-eligible | `false` | `(floating)` |
| `mfb pkg add alice#shape@1.4.0` | `1.4.0` | `true` | `(pinned)` |
| `mfb pkg add alice#shape --pin` | newest floating-eligible | `true` | `(pinned)` |
| `mfb pkg add alice#shape@1.4.0 --no-pin` | `1.4.0` | `false` | `(floating, floor 1.4.0)` |
| `mfb pkg add alice#shape --pin --no-pin` | — | — | usage error, exit `2` |

`--pin` and `--no-pin` together are a **usage error** rather than
last-flag-wins: the two orderings would otherwise mean different things with no
way to tell from the command line which was intended.

**Under `pin: false`, `version` is an ABI floor, not the version you get.** The
resolver looks the recorded version up as the *anchor*, takes that release's ABI
map as the project's requirement set, and then selects the highest eligible
version whose exported ABI is a superset of it. A `pin: true` dependency bypasses
the search entirely and takes its exact version.[[src/cli/resolve.rs:select_node]]
`--no-pin` on an `@version` add is therefore meaningful rather than
contradictory: it is the one way to set the floor deliberately instead of
accepting "whatever was newest the day `add` ran".

A `file://` add is always `pin: true` and `--no-pin` on one is a usage error:
a local file has no registry version stream to float along. `--pin` on a
`file://` target is accepted as a redundant statement of the existing behavior.

## `pkg update` Targeted Form

```text
mfb pkg update                                  re-resolve everything declared
mfb pkg update <owner>#<pkg>                    raise to newest compatible
mfb pkg update <owner>#<pkg>@<version>          set exactly
        [--pin | --no-pin] [--yes]
```

The bare form re-resolves every declared dependency and rewrites `mfb.lock`. A
project that declares **no registry dependencies** — only `file://` packages, or
none at all — has nothing to resolve; the bare form reports that, removes a
stale `mfb.lock` if present, and exits `0`.

A positional argument is an **ident**, never a path. `mfb pkg update foo` cannot
mean both, so the `[location]` form does not exist; a target that is not declared
in `project.json` is an error naming `mfb pkg add`, raised before any network
access.

**Pin state is preserved.** The targeted form never changes `pin` unless `--pin`
or `--no-pin` is passed, so raising a floating dependency's ABI floor leaves it
floating and re-versioning a pinned one leaves it pinned. `--pin` and `--no-pin`
together are a usage error, matching `pkg add`.

Moving a **pinned** dependency changes a deliberate choice, so it is confirmed
first; `--yes` bypasses the prompt, and a non-interactive session without `--yes`
is an error rather than a silent guess. Declining exits `0` having changed
nothing. A floating dependency is not prompted — its version is a floor, not a
promise.

### Version selection and the ABI advisory

With no `@version`, candidates are filtered in three stages:

1. **Eligibility** — only `available` and `deprecated` releases.
   `yanked` is selectable *only* by exact pin, so a bare update never moves a
   dependency **onto** a yanked release.[[src/cli/pkg.rs:state_is_floating_eligible]]
2. **Newer** — strictly greater than the currently declared version.
3. **ABI** — the candidate's exported symbols must be a superset of the
   currently declared version's, read from the index's per-version `abiIndex`.

Stage 3 exists because a `pin: true` dependency bypasses ABI checking entirely
during resolution — `select_node` takes the exact declared version as given.
Without a pre-flight filter, a targeted update could move a pinned dependency
onto a release that dropped a symbol the project uses: resolution would succeed
and the **build** would fail.[[src/cli/resolve.rs:select_node]]

**What the advisory proves, and what it does not.** It proves the candidate still
exports everything the *currently declared version* exported. It does **not**
prove the candidate satisfies the union of every requirer's needs — that union is
assembled by the resolver from sibling packages' import tables and does not exist
until resolution runs. This is a pre-flight advisory; the resolver remains the
authority, and when it disagrees afterwards, it wins.

When the newest eligible release fails stage 3, an older compatible one is
**not** selected silently. The skipped version and the symbols it drops are
named:

```text
alice#shape 2.0.0 is available but drops symbols the currently declared 1.4.0
exports (foo, bar); selecting 1.6.0 instead. Use `@2.0.0` to take it anyway.
```

An explicit `@version` is always honored and skips the advisory entirely — the
user has named the version themselves, which is the escape hatch the message
points at.

## `pkg remove` and the Reverse-Dependency Cascade

```text
mfb pkg remove <owner>#<pkg> [--yes]
```

Removes the named package **and every package that transitively imports it**.

### Why the cascade exists

The resolver seeds nodes only from dependencies declared in `project.json`, and
an import edge naming an ident that is not declared is **silently dropped**
rather than reported.[[src/cli/resolve.rs:resolve]] So removing only the named
package would leave any importer of it with a dangling import that *resolves
cleanly* and fails later, at build time, with an error pointing at the importer
rather than at the removal that caused it. Cascading is what keeps
`project.json` internally consistent.

### How the closure is computed

Entirely **offline**, from packages already on disk:

1. Collect every declared registry dependency.
2. Read each one's import table from `packages/<name>.mfp`, keeping imported
   idents that contain `#`.
3. Reverse those edges into `ident → {idents that import it}`.
4. Walk transitively from the target with a worklist and a visited set, so an
   import **cycle** terminates and a diamond yields each ident exactly once.

When the closure is larger than the named target, the full set is printed with
the *direct* importer that pulled each entry in — so a multi-level cascade reads
as a chain — and confirmation is required. `--yes` bypasses the prompt; a
non-interactive session without it is an error. Declining exits `0` having
changed nothing. Removing a package that nothing imports is not prompted.

### The not-installed gate

If a declared dependency's `packages/<name>.mfp` is missing, its imports cannot
be read, so the closure may omit a package that imports the target. This is an
**error**, and **`--yes` does not bypass it** — it is a correctness gate, not a
confirmation:

```text
error: cannot determine what depends on alice#shape — alice#widget is declared
       in project.json but not installed (packages/widget.mfp is missing).
       Run `mfb pkg install` first.
```

Proceeding would print a confident list, remove less than it should, and leave
exactly the dangling-import state the cascade exists to prevent.

### File cleanup

After resolution succeeds, each removed package's `packages/<name>.mfp` and its
`packages/<name>.vendor/` directory are deleted.[[src/manifest/libraries.rs:imported_vendor_dir]]
A missing file is not an error — the goal state is "absent". A deletion *failure*
is a warning naming the path, not a command failure: `project.json` and
`mfb.lock` are already consistent by then, so failing would misreport a completed
removal. Cleanup never runs before resolution succeeds, so a failed removal
leaves the working tree untouched.

### Removing the last dependency

A project with no declared registry dependencies has nothing to lock. `mfb.lock`
is deleted rather than left describing a dependency set that no longer exists,
and resolution and installation are skipped — an absent lock is the same state a
freshly `mfb init`-ed project is in. `mfb pkg install` then reports `nothing to
install` and exits `0`, rather than directing the user to run `mfb pkg update`,
which would have nothing to do.

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
`mfb repo publish` prints `Publish logged at index <n> (leaf <hex>)` followed by
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

`show_man` mirrors `spec` and **is** width-aware — it wraps to
`detect_terminal_width()` exactly as `spec` does; what it lacks is the `--width`
*flag* to override that. `--all` renders the whole manual with no positionals, or
a whole package with one; combining `--all` with a function name is the one
rejected form. Otherwise: zero args print the package
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
* ./mfb spec package-manager — registry protocol, signing, and `repo publish`/`repo` detail
