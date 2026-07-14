# CLI Modernization Plan

Last updated: 2026-07-14
Effort: medium (1h–2h)

Modernize the `mfb` command-line surface in one coordinated pass:

1. Move every intermediate-output emit flag to double-dash — `--ast`, `--ir`,
   `--br`, `--nir`, `--nplan`, `--nobj`, `--ncode`, `--mir` — keeping the
   single-dash spellings as accepted **but undocumented** aliases.
2. Move the behavioral flags `--target`, `--regalloc`, `--app` to double-dash,
   likewise keeping single-dash aliases; show only the `--` form in help.
3. Make top-level `mfb --help` / `mfb -h` print the usage screen (exit 0)
   instead of erroring.
4. Trim the top-level usage screen: **Package Management** shows only `add`,
   `update`, `install`, `verify`; **Repository & Auth** shows only `register`,
   `auth`. Everything else lives behind `mfb pkg --help` / `mfb repo --help`.
5. Add `mfb --version` printing a three-line version/build block, with the third
   line resolving to `Commit: <id>` when the build is from a clean, pushed tree
   and `Local Development` when the tree has uncommitted or unpushed work.

The single behavioral outcome: after this lands, `mfb --version`, `mfb --help`,
and the `--`-spelled flags all work; the old single-dash flag spellings still
work but no longer appear in any help/spec; and the top-level usage screen is
the trimmed two-tier layout.

References:

- `src/main.rs` — top-level dispatch (`fn main`, ~L244–505) and every `*_HELP`
  usage constant (`USAGE` L44–98, `PKG_HELP` L116–127, `REPO_HELP` L129–142,
  `BUILD_HELP` L144–167, `TEST_HELP` L169–181); `is_help_flag` L240–242.
- `src/cli/build.rs` — `BuildOutput::from_flag` (L116–130), `parse_build_options`
  (L132–207), `parse_test_options` (L731–776).
- `src/cli/pkg.rs` — `run_pkg_command` (L24–115); note its `Usage` errors
  interpolate `crate::USAGE` (`use crate::USAGE;` L15).
- `src/cli/repo.rs` — `run_repo_command` (L17–116) plus sibling families
  `run_machine_command`/`run_key_command`/`run_org_command`/`run_token_command`.
- `build.rs` (repo root, 259 lines, std-only, already uses
  `cargo:rerun-if-changed=`); `Cargo.toml` (`version = "0.1.0"`, no
  `[build-dependencies]` yet); `env!("CARGO_PKG_VERSION")` precedent at
  `src/audit/json.rs:119`.
- `src/docs/spec/tooling/07_cli-reference.md`,
  `src/docs/spec/architecture/01_commands.md`,
  `src/docs/spec/architecture/08_artifacts.md` — CLI/artifact specs to sync
  (spec-sync obligation per `.ai/specifications.md`).

## 1. Goal

- `mfb build <src> --ast --ir --br --nir --nplan --nobj --ncode --mir` and
  `mfb build <src> --target <t> --regalloc <s> --app` produce exactly today's
  behavior; the single-dash spellings of all these flags still work but appear
  in no help text or spec.
- `mfb --help` and `mfb -h` print the usage screen to stdout and exit 0.
- The top-level usage screen's Package Management section lists only
  `add`/`update`/`install`/`verify`, and Repository & Auth lists only
  `register`/`auth`; the full command sets are shown by `mfb pkg --help` and
  `mfb repo --help`.
- `mfb --version` prints:
  ```
  MFBasic Compiler <version>
  <UTC date/time built>
  <Commit: <commit_id>  |  Local Development>
  ```
  where line 3 is `Commit: <short-hash>` iff the build tree was clean **and**
  fully pushed to its upstream, else `Local Development`.

### Non-goals (explicit constraints)

- **No change to output filenames, artifact formats, or flag *semantics*.** Only
  spellings, help text, and dispatch change.
- **Single-char flags stay single-dash.** `-q`/`-v` (and their existing
  `--quiet`/`--verbose` long forms) are untouched; this plan does not add
  `--q`/`--v`.
- **No new package/repo subcommands and no change to their behavior** — only
  which ones the *top-level* screen advertises vs. the sub-help screen.
- **`mfb test` does not gain `-app`** (it never accepted it; keep it that way).
- **No new runtime dependencies.** The version/build metadata must be
  implementable with the existing std-only `build.rs` (git shelled out at build
  time); adding a `[build-dependencies]` crate is a rejected alternative (§4.5).

## 2. Current State

- **Emit flags.** `BuildOutput::from_flag` (`src/cli/build.rs:116-130`) is an
  exact `match` mapping `"-ast"`→`Ast`, `"-ir"`→`Ir`, `"-br"`→`BinaryRepr`,
  `"-nir"`→`NativeIr`, `"-nplan"`→`NativePlan`, `"-nobj"`→`NativeObjectPlan`,
  `"-ncode"`→`NativeCodePlan`, `"-mir"`→`Mir`, `_`→`None`. Called at
  `build.rs:144`; unrecognized `-`-prefixed tokens hit the "unknown build
  option" arm at `build.rs:189`.
- **Behavioral flags.** In `parse_build_options`: `-target`/`-target=`
  (`build.rs:149-155`), `-app` (`:167-171`), `-regalloc`/`-regalloc=`
  (`:174-180`). `parse_test_options` (`:731-776`) re-parses `-target`/`-target=`
  (`:741-747`) and `-regalloc`/`-regalloc=` (`:748-754`) independently; `-app`
  is not accepted by `test` (`app_mode: false` hardcoded, `:767`).
- **Help/parser mismatch (pre-existing bug this plan fixes).** `USAGE` (L86) and
  `BUILD_HELP` (L154–155) already *document* `--target`/`--app`, but the parser
  only accepts single-dash — so `mfb build --target …` currently hits "unknown
  build option". Moving the parser to accept `--` resolves this.
- **Top-level dispatch.** `fn main` (`src/main.rs:244-505`) matches
  `args.next().as_deref()`. `Some("help") | None` (L248) prints `USAGE`, exit 0.
  There is **no** `--help`/`-h`/`--version` arm at the top level, so those hit
  the catch-all `Some(command)` (L500) → `eprintln!("error: unknown command
  …")` + `exit(2)`. `is_help_flag` (L240–242, `--help`/`-h`) is consulted only
  *after* a recognized subcommand.
- **Usage screen.** `USAGE` (`src/main.rs:44-98`) is sectioned Project Setup /
  Package Management / Repository & Auth / Build & Development / Documentation.
  The Package Management block lists the full set; Repository & Auth lists
  register/auth/trust/link (plus the machine/key/org/token families dispatched
  as their own top-level commands at `main.rs:371-446`).
- **pkg/repo sub-help.** `PKG_HELP` (`main.rs:116-127`) is **stale** — documents
  ~5 of the 12 subcommands `run_pkg_command` (`src/cli/pkg.rs:24-115`) actually
  handles (missing validate/install/update/transfer/transfer-accept/
  release-state/check-abi). `run_pkg_command`'s `Usage` errors interpolate
  `crate::USAGE` (the *top-level* screen), so trimming USAGE would hide pkg
  subcommands from pkg error messages unless those interpolations are
  redirected. `REPO_HELP` (`main.rs:129-142`) is shared by repo/machine/key/org/
  token; `run_repo_command` error strings are short literals, not `REPO_HELP`.
- **Version/metadata.** No `--version` exists anywhere. Crate version is
  `Cargo.toml` `version = "0.1.0"`; `env!("CARGO_PKG_VERSION")` is used once
  (`src/audit/json.rs:119`). `build.rs` exists (root, std-only, generates doc
  tables, already uses `cargo:rerun-if-changed=`) but embeds **no** git hash,
  timestamp, or dirty/unpushed state. `.git` is present, so a build-time
  `git rev-parse` / `git status --porcelain` / `git rev-list @{u}..HEAD` is
  feasible with no new deps.

## 3. Design Overview

Five independent pieces, layered so the safe/aliased flag work lands first and
the build-metadata work (the only piece with cross-platform/build-cache subtlety)
lands last:

1. **Dual-accept flag parsing (emit + behavioral).** Because single-dash stays
   a working alias, no existing test, harness script, or user invocation breaks
   — the only forced churn is help/spec text. Risk: near-zero; the change is
   additive match arms.
2. **Top-level `--help`/`-h`/`--version` arms.** A small addition to the
   first-token match before the catch-all.
3. **Usage-screen re-tiering.** Trim `USAGE`'s pkg/repo sections; make
   `PKG_HELP`/`REPO_HELP` the complete-and-accurate lists; redirect
   `run_pkg_command`'s `Usage` interpolation from `USAGE` to `PKG_HELP` so error
   messages still show the full command set. Risk: the interpolation redirect is
   the easy-to-miss detail.
4. **Version formatting** (`print_version` in Rust, reading build-time env vars).
5. **Build metadata** (`build.rs` shells out to git). This is where correctness
   risk concentrates — dirty/unpushed detection must be conservative and the
   `cargo:rerun-if-changed=` set must force a rebuild when git state changes.

**Rejected alternatives.**
- *Hard-swap the flags (no alias).* The user asked for aliases; dual-accept also
  eliminates all harness/test churn. Rejected in favor of dual-accept.
- *A `[build-dependencies]` crate (`vergen`/`built`/`chrono`) for metadata.*
  Violates the no-new-deps non-goal; the existing std-only `build.rs` can shell
  out to `git` and `date -u`. Rejected.
- *Compute the dirty/unpushed state at runtime in the binary.* The shipped
  binary may run far from its source tree; capture at build time instead.

## 4. Detailed Design

### 4.1 Dual-accept emit flags (`src/cli/build.rs:116-130`)

Extend `from_flag` so each variant matches both spellings, e.g.
`"--ast" | "-ast" => Some(BuildOutput::Ast)`, for all eight. Nothing else in
`from_flag`, the caller, the duplicate check, or the fallthrough changes.

### 4.2 Dual-accept behavioral flags (`src/cli/build.rs`)

In `parse_build_options`: accept `"--target" | "-target"` (space form) and both
`--target=`/`-target=` prefixes; same for `--regalloc`/`-regalloc` and their
`=` forms; accept `"--app" | "-app"`. In `parse_test_options`: accept the `--`
and `-` forms of `target`/`regalloc`. Keep every existing error message and the
`-app` duplicate guard. `--app` is still rejected by `test` (unchanged).

### 4.3 Help/spec text → double-dash only

- `BUILD_HELP` (`main.rs:144-167`): show `--ast`/`--ir`/`--br`/`--mir`/`--nir`/
  `--nplan`/`--ncode` **and add the currently-missing `--nobj` line**; show
  `--target`/`--regalloc`/`--app`. `USAGE` L86: already `--target` (now
  correct). `TEST_HELP` (L169-181): `--target`/`--regalloc`.
- Specs: update the flag columns in `07_cli-reference.md:71-78`,
  `01_commands.md:13-21`, `08_artifacts.md:38-45` to `--` spellings. `grep -rn`
  `src/docs/spec/**` for the bare flag tokens and update any shown as invocable
  flags; leave artifact-name prose (`.ir`, `.mir`) alone.
- Harness scripts (`scripts/artifact-gate.sh`, `scripts/test-accept.sh`) may be
  left on single-dash (still valid via alias) — updating them to `--` is
  optional cleanup, not required, and either way goldens stay byte-identical.

### 4.4 Top-level `--help` / `-h` / `--version` (`src/main.rs` `fn main`)

Add first-token match arms **before** the catch-all (`main.rs:500`):

- `Some("--help") | Some("-h")` → `println!("{USAGE}")`, return (exit 0) — same
  as the existing `help`/`None` arm.
- `Some("--version")` → call `print_version()`, return. (Recommend also
  accepting `-V` — conventional; see Open Decisions.)

### 4.5 Usage re-tiering & sub-help accuracy

- **`USAGE` (`main.rs:44-98`):** Package Management section lists only `add`,
  `update`, `install`, `verify`. Repository & Auth lists only `register`,
  `auth`. Add a hint line under each (e.g. `Run 'mfb pkg --help' for all package
  commands.` / `Run 'mfb repo --help' for all repository & auth commands.`).
- **`PKG_HELP` (`main.rs:116-127`):** rewrite to the *complete, accurate* set
  from `run_pkg_command` (`pkg.rs:24-115`): add, info, doc, verify
  (`--proof`), validate, install, update, transfer, transfer-accept,
  release-state, check-abi, publish (positional `<owner> <pkg>`, correcting the
  stale `--owner`).
- **`REPO_HELP` (`main.rs:129-142`):** ensure it lists register, auth, trust,
  link, and the machine/key/org/token families (it is shared by those top-level
  commands).
- **Error interpolation:** change `run_pkg_command`'s `Usage` errors
  (`pkg.rs`, currently `crate::USAGE`) to interpolate `crate::PKG_HELP`, so pkg
  errors still show the full pkg command set after USAGE is trimmed. Audit
  `repo.rs` error strings similarly (they are short literals; update any that
  should point users at `mfb repo --help`).

### 4.6 `print_version()` (Rust side)

New `print_version()` (in `src/main.rs` or a small `src/cli/version.rs`) reading
three build-time env vars and `CARGO_PKG_VERSION`:

```
MFBasic Compiler {CARGO_PKG_VERSION}
{MFB_BUILD_DATE}
{ "Commit: " + MFB_COMMIT   if MFB_LOCAL_DEV == "0"
  "Local Development"        otherwise }
```

Use `option_env!` for the MFB_* vars with sane fallbacks (`"unknown"` date,
`Local Development` when metadata is absent) so a build without git still
produces valid output.

### 4.7 Build metadata (`build.rs`)

Add a metadata step to the existing `build.rs`:

- `MFB_BUILD_DATE`: `date -u +"%Y-%m-%d %H:%M:%S UTC"` (no new deps; macOS/Linux
  targets only, consistent with the project's supported platforms).
- `MFB_COMMIT`: `git rev-parse --short HEAD`.
- `MFB_LOCAL_DEV`: `"1"` if **either** `git status --porcelain` is non-empty
  (uncommitted work) **or** `git rev-list @{u}..HEAD` is non-empty / `@{u}` is
  unresolvable (HEAD ahead of, or with no, upstream = not pushed); else `"0"`.
- Emit each via `println!("cargo:rustc-env=MFB_…={…}");`.
- Add `cargo:rerun-if-changed=.git/HEAD`, the current branch ref file, and
  `.git/index` so commit/dirty changes force a rebuild of the metadata (the
  build.rs already uses this idiom). **Known caveat:** cargo build caching means
  the timestamp/state reflect the last time `build.rs` re-ran, not necessarily
  the instant of the final link — document this; it is acceptable for a
  `--version` stamp. If any git command fails (no `.git`, git absent), fall back
  to empty `MFB_COMMIT` + `MFB_LOCAL_DEV=1` (→ `Local Development`), never fail
  the build.

## Compatibility / Format Impact

- **User-visible additions:** `mfb --help`, `mfb -h`, `mfb --version` now work
  (previously exit 2). Double-dash spellings of all listed flags now work.
- **User-visible removals from docs only:** single-dash emit/behavioral flag
  spellings and the trimmed pkg/repo commands disappear from the *top-level*
  screen, but every single-dash flag still functions (alias) and every pkg/repo
  command is still dispatched and still shown by its sub-help. No hard break.
- **Unchanged:** all artifact filenames/formats, flag semantics, subcommand
  behavior, `-q`/`-v`, and `mfb test`'s flag set.
- **New build-time env vars** (`MFB_BUILD_DATE`, `MFB_COMMIT`, `MFB_LOCAL_DEV`)
  are internal to the build; no wire/artifact format changes.

## Phases

### Phase 1 — Dual-accept flag parsing (safe, aliased)

Land the parser changes first; single-dash aliases mean nothing downstream
breaks.

- [ ] Extend `BuildOutput::from_flag` (`src/cli/build.rs:116-130`) to accept
      `--x | -x` for all 8 emit flags.
- [ ] Extend `parse_build_options` and `parse_test_options` (`src/cli/build.rs`)
      to accept `--target`/`--target=`, `--regalloc`/`--regalloc=`, `--app`
      alongside their single-dash forms.
- [ ] Tests (`src/cli/build.rs`): update/extend `from_flag` round-trip
      (`:1288-1305`) and `parse_build_options`/`parse_test_options` tests
      (`:561-589`, `:1322-1335`, `:1420-1422`, `:1695-1696`) to assert **both**
      spellings map identically for every affected flag.

Acceptance: `cargo test` for the build-CLI module passes, with explicit asserts
that `--ast`==`-ast`, `--target`==`-target`, `--regalloc`==`-regalloc`,
`--app`==`-app` (and the `=` forms) all resolve to the same parsed result.
Commit: —

### Phase 2 — Top-level `--help` / `-h` / `--version` dispatch

- [ ] Add `Some("--help") | Some("-h")` arm (print `USAGE`, exit 0) and
      `Some("--version")` arm (call `print_version()`) before the catch-all in
      `fn main` (`src/main.rs:~500`).
- [ ] Add `print_version()` (§4.6) reading `option_env!` MFB_* vars +
      `CARGO_PKG_VERSION`.

Acceptance: `mfb --help` and `mfb -h` print the usage screen to stdout and exit
0; `mfb --version` prints the three-line block (line 3 present; exact commit/
Local-Development value validated in Phase 4).
Commit: —

### Phase 3 — Usage re-tiering + accurate sub-help

- [ ] Trim `USAGE` Package Management → add/update/install/verify and Repository
      & Auth → register/auth, each with a `Run 'mfb <x> --help' …` hint
      (`src/main.rs:44-98`).
- [ ] Rewrite `PKG_HELP` (`:116-127`) to the complete 12-command set and
      `REPO_HELP` (`:129-142`) to the full repo/machine/key/org/token set.
- [ ] Redirect `run_pkg_command` `Usage` errors from `crate::USAGE` to
      `crate::PKG_HELP` (`src/cli/pkg.rs`); audit `src/cli/repo.rs` error
      strings and point them at `mfb repo --help` where appropriate.

Acceptance: `mfb` / `mfb help` show the trimmed top-level screen; `mfb pkg
--help` lists all 12 pkg subcommands and `mfb repo --help` the full repo set; an
invalid pkg subcommand (e.g. `mfb pkg bogus`) prints the full pkg list, not the
trimmed top-level one.
Commit: —

### Phase 4 — Build metadata (highest-risk, last)

- [ ] Extend `build.rs` (root) to emit `MFB_BUILD_DATE`, `MFB_COMMIT`,
      `MFB_LOCAL_DEV` per §4.7, with `cargo:rerun-if-changed` on `.git/HEAD`,
      the branch ref, and `.git/index`, and a no-git fallback.
- [ ] Verify `print_version()` renders `Commit: <hash>` on a clean+pushed build
      and `Local Development` when the tree is dirty or unpushed.

Acceptance: on a clean, fully-pushed checkout `mfb --version` line 3 reads
`Commit: <short-hash>`; after touching a tracked file (or with a local unpushed
commit) a rebuilt `mfb --version` line 3 reads `Local Development`; a build in a
tree with no `.git` still succeeds and prints `Local Development`.
Commit: —

### Phase 5 — Doc & spec sync

- [ ] Update `BUILD_HELP`/`TEST_HELP`/`USAGE` flag spellings to `--` only
      (folded with Phase 1/3 edits where they touch the same constants).
- [ ] Update flag tables in `07_cli-reference.md`, `01_commands.md`,
      `08_artifacts.md`; add `mfb --version`/`mfb --help` to the CLI reference;
      `grep -rn src/docs/spec/**` to confirm no single-dash *flag* usage remains
      (artifact-name prose excepted).

Acceptance: `grep -rn` across help constants + `src/docs/spec/**` finds no
single-dash emit/behavioral *flag* invocation; the CLI-reference spec documents
`--version` and top-level `--help`.
Commit: —

## Validation Plan

- Tests: `from_flag`/`parse_build_options`/`parse_test_options` dual-spelling
  asserts (`src/cli/build.rs`); a `print_version` formatting unit test driving
  both the `MFB_LOCAL_DEV=0` (→ `Commit:`) and `=1` (→ `Local Development`)
  branches via injected values.
- Runtime proof: `mfb --help` exits 0 with usage; `mfb --version` shows the
  three-line block; `mfb build <s>.mfb --ast --ir --mir` and `mfb build <s>.mfb
  -ast -ir -mir` (alias) produce byte-identical artifacts; `mfb pkg bogus`
  prints the full pkg list.
- Doc sync: `src/main.rs` help constants + `07_cli-reference.md`,
  `01_commands.md`, `08_artifacts.md`, per `.ai/specifications.md`.
- Acceptance: `scripts/artifact-gate.sh` and the full `scripts/test-accept.sh`
  run green with byte-identical goldens (aliases keep harness invocations
  valid).

## Open Decisions

- **`-V` alias for `--version`.** Recommended: accept `-V` too (conventional).
  Alternative: `--version` only, per the literal request. (§4.4)
- **Build timestamp source.** Recommended: shell `date -u` in `build.rs`
  (no deps, macOS/Linux only). Alternative: format `SystemTime` epoch manually
  in pure std (portable but more code). (§4.7)
- **Harness scripts spelling.** Recommended: leave `scripts/*.sh` on single-dash
  aliases (zero churn, goldens stable). Alternative: migrate to `--` for
  consistency. (§4.3)

## Summary

The engineering risk lives almost entirely in Phase 4 (build-time git metadata:
conservative dirty/unpushed detection + a correct `rerun-if-changed` set +
graceful no-git fallback) and in the one easy-to-miss detail of Phase 3
(redirecting `run_pkg_command`'s error interpolation from `USAGE` to `PKG_HELP`
so trimming the top-level screen doesn't hide subcommands from error output).
The flag work is low-risk because single-dash aliases keep every existing
invocation, test, and harness script working; artifact formats, flag semantics,
and subcommand behavior are all untouched.
