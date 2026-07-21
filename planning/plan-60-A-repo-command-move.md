# plan-60-A: Move publisher-side commands to `mfb repo`

Last updated: 2026-07-21
Overall Effort: large (3h–1d)
Effort: medium (1h–2h)
Depends on: nothing
Produces: the trimmed `run_pkg_command` match (consumer-side commands only) that
letters C/E/F extend with new arms; the `run_repo_command` arms owning
`publish`/`check-abi`/`release-state`/`transfer`/`transfer-accept`; the rewritten
`PKG_HELP`/`REPO_HELP`/`USAGE` constants; the reorganized CLI-reference spec table.

`mfb pkg` currently mixes two audiences: commands a package *consumer* runs
(`add`/`update`/`install`/`verify`/`validate`/`info`/`doc`) and commands a package
*publisher* runs against a registry (`publish`/`check-abi`/`release-state`/
`transfer`/`transfer-accept`). This letter moves all five publisher-side commands
under `mfb repo`, which already owns the identity/registry surface
(`register`/`auth`/`link`/`trust`).

**Behavioral outcome:** `mfb repo publish alice .` succeeds and `mfb pkg publish
alice .` exits 2 with an unknown-subcommand usage error naming the `repo` form.
The same holds for the other four commands. No aliases, no deprecation period.

This letter lands first and alone because it is the only one that churns a large
population of test call sites and spec rows (23 test arg-vectors + 5 spec table
rows, measured below) without changing any resolution logic. Mixing it into a
letter whose diff needs semantic review would bury the real change.

References:

- `src/docs/spec/tooling/07_cli-reference.md` — the CLI command/exit-code table
- `src/docs/spec/package-manager/01_repository-protocol.md` — registry protocol,
  publish/release-state/transfer authority rules
- `.ai/specifications.md` — the spec-sync obligation (CLI output is explicitly
  listed as a contract requiring a same-change spec update)

## Prerequisites

These are a precondition on the whole plan-60 feature, not a dependency to
negotiate. Sub-plans B–F point back here.

| Must be true | Command | Status |
|---|---|---|
| Working tree builds | `cargo build` → exit 0 | UNVERIFIED at authoring |
| Unit suite green at HEAD | `cargo test --bin mfb` → exit 0 | UNVERIFIED at authoring |
| Registry acceptance suite runnable and green at HEAD | `cargo test --test repo_acceptance` → exit 0 | UNVERIFIED at authoring |
| `mfb-repo` server binary builds (the acceptance suite shells out to it) | `cargo build --manifest-path repository/Cargo.toml --bin mfb-repo` → exit 0 | UNVERIFIED at authoring |

Everything below is written against the world where these hold. There are no
hedges for the world where they don't.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. Never act on a status you did not just verify.
>
> **If you stop, report the current status of *all* prerequisites** — not only
> the one that blocked you.

## Dependency graph

```
A ← nothing;  B ← A;  C ← B;  D ← C;  E ← C;  F ← C
```

A is the CLI-surface move. B extracts the shared primitives (confirm helper +
resolve-first pipeline) that C, E and F all consume. C redesigns `add` and is the
first command to produce a `pin: false` dependency from the CLI — which D's
warning path and E's pin-preservation rule both need in order to be testable at
all. D, E and F fan out from C and are independent of each other.

Execution is topological order over this graph, re-checking each letter's stated
preconditions.

## 1. Goal

- All five publisher-side commands are reachable only as `mfb repo <command>`.
- `mfb pkg <moved-command>` exits 2 with a usage error that names the new form.
- `PKG_HELP`, `REPO_HELP` and `USAGE` list each command under exactly one parent.
- The CLI-reference spec table matches the implemented surface.

### Non-goals (explicit constraints)

- **No aliases and no deprecation shims.** `mfb pkg publish` is a hard error.
- **No change to what the moved commands do.** Argument shapes, network
  behavior, signing, exit codes and printed output are byte-identical apart from
  the command word in usage/error strings. This letter is a re-parenting only.
- **No change to resolution, the lockfile, or `packages/` layout.**
- **No change to `mfb machine|key|org|token`** — they keep their own dispatch and
  keep printing `REPO_HELP`.

## 2. Current State

`run_pkg_command` (`src/cli/pkg.rs:24`) matches on positional argument slices.
The five publisher-side commands are dispatched at:

- `publish` — `src/cli/pkg.rs:93` (`[command, owner, package]`), implemented by
  `publish_package_project` (`src/cli/pkg.rs:121`)
- `transfer` — `src/cli/pkg.rs:60`, implemented at `src/cli/pkg.rs:302`
- `transfer-accept` — `src/cli/pkg.rs:66`, implemented at `src/cli/pkg.rs:319`
- `release-state` — `src/cli/pkg.rs:72` and `:75`, implemented at `src/cli/pkg.rs:340`
- `check-abi` — `src/cli/pkg.rs:81` and `:84`, implemented at `src/cli/pkg.rs:395`

`run_repo_command` (`src/cli/repo.rs:23`) matches on `args.first()` as a string
and dispatches with `match command { … }` — a *different* shape from `pkg`'s
slice matching. It resolves the repo URL and local key paths eagerly at
`src/cli/repo.rs:30-31`, before dispatch.

Both error enums map identically in `src/main.rs`: `Usage` → exit 2, `Failed` →
exit 1 (`src/main.rs:358-370` for pkg, `src/main.rs:377-389` for repo).

Help constants live in `src/main.rs`: `USAGE` at `:45` (lines 53–57 cover pkg,
60–62 cover repo), `PKG_HELP` at `:96`, `REPO_HELP` at `:120`.

### Measured populations

| What | Count | Command |
|---|---|---|
| `pkg publish` arg-vector sites in `tests/` | 19 | `grep -rn '"pkg", "publish"' tests/ \| wc -l` → 19 |
| `pkg transfer` arg-vector sites in `tests/` | 1 | `grep -rn '"pkg", "transfer"' tests/ \| wc -l` → 1 |
| `pkg transfer-accept` arg-vector sites in `tests/` | 1 | `grep -rn '"pkg", "transfer-accept"' tests/ \| wc -l` → 1 |
| `pkg check-abi` arg-vector sites in `tests/` | 1 | `grep -rn '"pkg", "check-abi"' tests/ \| wc -l` → 1 |
| `pkg release-state` arg-vector sites in `tests/` | 1 | `grep -rn '"pkg", "release-state"' tests/ \| wc -l` → 1 |
| **Total test arg-vector sites to rewrite** | **23** | sum of the above |
| CLI-reference **table rows** naming a moved command | 4 | `grep -n 'pkg publish\|pkg check-abi\|pkg release-state\|pkg transfer' src/docs/spec/tooling/07_cli-reference.md` → rows at :51, :54, :55, :63 — four rows covering five commands, because `transfer`/`transfer-accept` share the row at :63 |
| CLI-reference **prose** mentions of a moved command (same file, outside the table) | 3 | same command → :215 (`publish`/`check-abi` build quietly), :305 (`publish` log-index output), :458 (See Also link) |
| Usage-string assertions in `pkg.rs` unit tests | 5 | `src/cli/pkg.rs:1884-1891` — one assertion per moved command |
| `tests/repo_acceptance.rs` total lines | 2240 | `wc -l tests/repo_acceptance.rs` → 2240 |

### Verified properties

- **The five moved commands do not read `PkgCommandError`-specific behavior.**
  Verified by reading `src/main.rs:358-389`: `PkgCommandError` and
  `RepoCommandError` are two enums with identical `Usage`/`Failed` variants and
  identical exit-code mapping. Re-parenting therefore cannot change an exit code.
- **`run_repo_command` resolves repo paths before dispatch** (`src/cli/repo.rs:30-31`),
  whereas `run_pkg_command` does not. Read both. All five moved commands already
  call `local_paths_for_repo` (or reach the registry) on every successful path, so
  eager resolution does not add a failure mode to a path that previously
  succeeded — **but it does change *when* the error surfaces for a malformed
  argument**: `mfb pkg check-abi a b c` currently returns a usage error without
  touching the key store, while `mfb repo check-abi a b c` would fail on the key
  store first. Phase 2 handles this explicitly.
- **`mfb spec` is version-locked to the binary and embeds `src/docs/spec/**`**
  (`.ai/specifications.md`). CLI output is named in that file's list of contracts
  that must be spec-updated in the same change. Confirmed by reading the file —
  this is a hard gate, not optional cleanup.
- **UNVERIFIED: whether any doc outside `src/docs/spec/` and `planning/` invokes a
  moved command.** Historical `planning/old-plans/*` and `bugs/completed-bugs/*`
  hits are archived records and must NOT be rewritten. Phase 1 measures the live
  set.

## 3. Design Overview

Three mechanical pieces, deliberately ordered so the surface is settled before
anything that can be tested against it:

1. **Move the dispatch.** Delete the five arms from `run_pkg_command`; add
   equivalent arms to `run_repo_command`. The implementation functions move from
   `src/cli/pkg.rs` to `src/cli/repo.rs` or stay in `pkg.rs` and become
   `pub(crate)` — see §4 for which, and why.
2. **Rewrite the help constants and the spec table** so the documented surface
   matches the dispatched one.
3. **Rewrite the 23 test arg-vectors** and the 5 usage assertions.

**Design uncertainty: low.** No premise here can be falsified — the commands
already work, and this changes which word invokes them. Nothing needs a spike.

**Correctness risk: concentrated in the argument-shape translation** between
`pkg`'s slice matching and `repo`'s `match command` + destructuring style. A
slice pattern like `[command, state, version]` becoming `let [_, state, version]
= args` is where an arity check can silently loosen. Phase 2's tests exist
specifically to pin every arity boundary.

**Rejected alternative: keep `mfb pkg publish` as a hidden alias.** Rejected by
explicit decision — aliases mean the old form stays in muscle memory and in
copy-pasted CI scripts indefinitely, and the whole point is a surface where the
command word tells you which audience you are in.

**Rejected alternative: move the implementation functions into `repo.rs`.**
Deferred, not rejected outright — see §4.

## 4. Detailed Design

### 4.1 Where the implementation functions live

`publish_package_project`, and the `transfer`/`transfer-accept`/`release-state`/
`check-abi` implementations, currently sit in `src/cli/pkg.rs` alongside
consumer-side code. They call several `pkg.rs`-private helpers — notably
`hex_bytes` (`src/cli/pkg.rs`, used by `install_vendor_blobs` at `:725` and by
`project_hash` at `src/audit/collect/mod.rs:113`) and `install_vendor_blobs`
(`src/cli/pkg.rs:703`).

**Decision: leave the implementation functions in `src/cli/pkg.rs` and make them
`pub(crate)`.** Only the *dispatch* moves. Physically relocating five functions
plus their private helpers into `repo.rs` would produce a large, mostly-noise
diff in the same commit as a user-visible surface change, and would tangle with
letters C–F, which also edit `pkg.rs`. File organization is a separate concern
from command surface; if it is worth doing it is worth its own change.

Record this in the module doc comment of `src/cli/pkg.rs` so the split is
intentional and legible: dispatch for publisher commands lives in `repo.rs`,
implementations remain here.

### 4.2 Argument shapes after the move

| New command | Arity | Old dispatch |
|---|---|---|
| `mfb repo publish <owner> [path]` | 1 or 2 positional | `src/cli/pkg.rs:93` (was 2, mandatory) |
| `mfb repo check-abi [path]` | 0 or 1 | `src/cli/pkg.rs:81`, `:84` |
| `mfb repo release-state <state> [version]` | 1 or 2 | `src/cli/pkg.rs:72`, `:75` |
| `mfb repo transfer <owner>#<pkg> <to-owner>` | 2 | `src/cli/pkg.rs:60` |
| `mfb repo transfer-accept <owner>#<pkg>@<to-owner>` | 1 | `src/cli/pkg.rs:66` |

**`publish` gains an optional path defaulting to `.`** — the one behavioral
change in this letter, and the one the user explicitly asked for. `mfb repo
publish alice` must behave exactly as `mfb repo publish alice .` does.

Note the second positional is a **project directory**, not a package name:
`publish_package_project` takes it as `project_dir` (`src/cli/pkg.rs:121`). The
existing `PKG_HELP` text calls it `<package>`, which is wrong. The new help text
says `[path]`.

### 4.3 The eager-path-resolution difference

`run_repo_command` calls `local_paths_for_repo` at `src/cli/repo.rs:31` before
matching. To preserve the current property that a usage error never depends on
key-store state, **validate arity before the eager path resolution** — either by
hoisting an arity check above line 30, or by moving the `local_paths_for_repo`
call into the arms that need it.

Prefer moving it into the arms: it is a smaller behavioral change than
reordering validation for all of `repo`'s existing commands, and it keeps
`register`/`auth`/`link`/`trust` exactly as they are.

## Compatibility / Format Impact

**Changes (all CLI surface, no formats):**

- `mfb pkg publish|check-abi|release-state|transfer|transfer-accept` — removed.
  Now exit 2 via `run_pkg_command`'s existing unknown-subcommand arm
  (`src/cli/pkg.rs:111`).
- `mfb repo publish|check-abi|release-state|transfer|transfer-accept` — added.
- `mfb repo publish <owner>` — new optional-path form.

**Explicitly unchanged:** the `.mfp` byte format; `mfb.lock` contents and
`lockfileVersion`; `project.json` schema; the registry wire protocol; every exit
code; `packages/` layout; `mfb machine|key|org|token`.

## Phases

> **NOTE — keep the checkboxes current as you go. This is not bookkeeping; it is
> the only way anyone can see where the work actually is.**
>
> - Tick `- [x]` **in the same commit as the work it describes**.
> - Use `- [~]` for partially done and say in one line what remains.
> - Mark a task moot with `- [x] ~~text~~ — moot: <evidence>`.
> - Fill the phase's `Commit:` line with the hash the moment it lands.
>
> **An unticked box means NOT DONE.**

### Phase 1 — Census the live documentation surface

Settles the one UNVERIFIED population before any code moves, so Phase 3's scope
is known rather than discovered.

- [ ] Run `grep -rn 'pkg publish\|pkg transfer\|pkg check-abi\|pkg release-state' --include='*.md' . | grep -v '^./planning/old-plans/' | grep -v '^./bugs/completed-bugs/' | grep -v '^./planning/old-moved-to-src-spec/'` and record the exact file:line list in this plan's §2 table.
- [ ] Confirm which hits are **live docs** (must be rewritten) vs **archived
      records** (must NOT be rewritten). Archived plans and completed bug reports
      describe history and stay as written.
- [ ] Write the resulting live-doc count into the Measured populations table with
      its command.

Acceptance: §2's Measured populations table has no UNVERIFIED row, and the
live-doc file list is written into this plan.
Commit: —

### Phase 2 — Move the dispatch

- [ ] Delete the five publisher-side match arms and their usage-error arms from
      `run_pkg_command` (`src/cli/pkg.rs:60-92` and `:93`, `:105`).
- [ ] Make `publish_package_project` (`src/cli/pkg.rs:121`) and the four other
      implementation functions (`:302`, `:319`, `:340`, `:395`) `pub(crate)`.
- [ ] Add five arms to `run_repo_command` (`src/cli/repo.rs:33`) matching §4.2's
      arity table, each calling into `crate::cli::pkg::<fn>`.
- [ ] Give `publish` an optional second positional defaulting to `Path::new(".")`.
- [ ] Move the `local_paths_for_repo` call (`src/cli/repo.rs:31`) into the arms
      that use it, so arity errors do not depend on key-store state (§4.3).
- [ ] Update the module doc comment of `src/cli/pkg.rs` to record that publisher
      dispatch lives in `repo.rs` while implementations stay here (§4.1).
- [ ] Tests: rewrite the 5 usage assertions at `src/cli/pkg.rs:1884-1891` to
      assert the *unknown subcommand* error instead, and add matching arity
      assertions in `src/cli/repo.rs` — one per command, covering both too-few
      and too-many arguments, plus `repo publish alice` (1-arg form) reaching
      dispatch rather than erroring.

Acceptance: `cargo test --bin mfb` passes; `mfb pkg publish alice .` exits 2 with
a message naming `mfb repo publish`; `mfb repo publish alice` and `mfb repo
publish alice .` are observably equivalent (same stdout, same exit code) against
the acceptance harness registry.
Commit: —

### Phase 3 — Rewrite help constants and the spec

- [ ] `src/main.rs:96` `PKG_HELP`: remove the five moved commands; keep
      `add`/`info`/`doc`/`verify`/`validate`/`install`/`update`.
- [ ] `src/main.rs:120` `REPO_HELP`: add the five under a **Publishing** heading,
      with `publish <owner> [path]` — `[path]`, not `<package>` (§4.2).
- [ ] `src/main.rs:45` `USAGE`: verify lines 53–57 and 60–62 still name a correct
      representative subset; adjust the `Run 'mfb pkg --help'` / `mfb repo --help`
      pointers if the split changed which commands are worth surfacing.
- [ ] `src/docs/spec/tooling/07_cli-reference.md`: move the 4 table rows (`:51`,
      `:54`, `:55`, `:63` — five commands, since `transfer`/`transfer-accept`
      share `:63`) from the `pkg` block into the `repo` block, renaming each
      command and fixing `publish`'s argument to `<owner_name> [path]`. Exit-code
      columns are unchanged.
- [ ] `src/docs/spec/tooling/07_cli-reference.md`: update the 3 **prose** mentions
      outside the table — `:215` (`pkg publish`/`pkg check-abi` run the build
      quietly), `:305` (`pkg publish` prints the log index), `:458` (the See Also
      link naming `pkg publish`). These are in the same file as the table and are
      easy to miss when only the table is edited.
- [ ] `src/docs/spec/package-manager/spec.md:4` and
      `src/docs/spec/tooling/spec.md:14` — both name `mfb pkg publish` in prose.
      Update to `mfb repo publish`.
- [ ] `src/docs/spec/package-manager/01_repository-protocol.md:379` — names
      `mfb pkg publish`. Update. Re-check `:442` and `:445`, which name `mfb pkg
      verify` (**not** moved — leave alone).
- [ ] Update every live doc found in Phase 1; leave archived records untouched.
- [ ] Tests: rewrite the 23 test arg-vectors in `tests/repo_acceptance.rs` from
      `["pkg", "<cmd>", …]` to `["repo", "<cmd>", …]`.

Acceptance: `cargo build` regenerates the embedded spec; `cargo test --bin mfb
spec` passes; `mfb spec tooling --all` and `mfb spec package-manager --all` render
with no leaked `[[` markers; `cargo test --test repo_acceptance` passes;
`grep -rn '"pkg", "publish"' tests/` returns 0 hits.
Commit: —

## Validation Plan

- **Tests:** arity coverage for all five commands in `src/cli/repo.rs` unit tests
  (too-few, too-many, and for `publish` both the 1-arg and 2-arg forms);
  rewritten usage assertions in `src/cli/pkg.rs`; the 23 rewritten acceptance
  arg-vectors.
- **Coverage check:** the moved implementation bodies are marked `// coverage:off`
  (`src/cli/pkg.rs:117-120`) because they reach a live registry. That annotation
  must move with them or stay accurate — confirm the new dispatch arms in
  `repo.rs` are themselves in the coverage denominator, since a green unit suite
  otherwise proves nothing about the arms this phase adds.
- **Runtime proof:** against the `repo_acceptance` harness registry, publish a
  package with `mfb repo publish alice` (no path argument, from inside the
  package directory) and confirm the artifact appears in the registry index —
  this proves both the move and the new default-path behavior end to end.
- **Doc sync:** `src/docs/spec/tooling/07_cli-reference.md`,
  `src/docs/spec/tooling/spec.md`, `src/docs/spec/package-manager/spec.md`,
  `src/docs/spec/package-manager/01_repository-protocol.md`.
- **Acceptance:** `cargo build && cargo test --bin mfb && cargo test --test repo_acceptance`.

## Open Decisions

- **Should the unknown-subcommand error name the new location?** Recommended:
  yes — special-case the five moved names in `run_pkg_command`'s fallback arm
  (`src/cli/pkg.rs:111`) to emit `mfb pkg publish has moved to mfb repo publish`
  rather than a bare `unknown pkg command`. This is not an alias (it still exits
  2 and does nothing), and it is the difference between a five-second fix and a
  grep through the help text. (§4.2)

## Corrections

<!-- Filled in DURING execution. -->

## Summary

The engineering risk is almost entirely in the argument-shape translation from
`pkg`'s slice matching to `repo`'s `match` + destructure style (§4.3), where an
arity check can silently loosen without any test noticing. Everything else is
mechanical rewriting whose failure mode is a compile error.

Untouched: all resolution logic, `mfb.lock`, `project.json` handling, the `.mfp`
format, and the registry wire protocol. No letter after this one depends on
anything here except the trimmed `run_pkg_command` match that C, E and F extend.
