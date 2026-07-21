# plan-60-F: `mfb pkg remove` with reverse-dependency cascade

Last updated: 2026-07-21
Effort: medium (1h–2h)
Depends on: plan-60-C
Produces: the `remove` command. Terminal letter — nothing consumes it.

There is no way to remove a dependency. Deleting the entry from `project.json` by
hand changes `projectHash` (`src/audit/collect/mod.rs:87`), which makes
`mfb pkg install` hard-error, and leaves `packages/<name>.mfp` and any
`packages/<name>.vendor/` directory orphaned on disk. This letter adds the
command, and makes it refuse to leave the project in a state that resolves
cleanly but cannot build.

**Behavioral outcome:** `mfb pkg remove alice#shape` removes the dependency,
re-resolves, rewrites `mfb.lock`, and deletes `packages/shape.mfp` and
`packages/shape.vendor/`. When another declared package still imports
`alice#shape`, the command first lists every package that will be removed as a
consequence and asks for confirmation; declining changes nothing.

This letter lands last. It is the only command in the feature that **deletes
files** and the only one that can remove something the user did not name — the
largest blast radius in plan-60.

References:

- plan-60-B §4.2 — `apply_manifest_change`; §4.1 — `confirm`; §4.3 — the
  zero-dependency lockfile policy this letter is the first to reach
- `src/cli/resolve.rs:247-262` — where undeclared import edges are silently
  dropped, the defect this letter's cascade exists to prevent
- `src/manifest/libraries.rs:443` — `imported_vendor_dir`
- `src/binary_repr/mod.rs:451` — `read_package_info`

## Prerequisites

See plan-60-A for the plan-wide prerequisite gate. In addition:

| Must be true | Command | Status |
|---|---|---|
| plan-60-B complete — `confirm`, `apply_manifest_change`, and the zero-dependency policy | `grep -c 'fn confirm' src/cli/mod.rs` → 1 and `grep -c 'fn apply_manifest_change' src/cli/resolve.rs` → 1 | NOT MET at authoring |
| plan-60-C complete — flag parsing for a `pkg` subcommand | `grep -c 'no_pin' src/cli/pkg.rs` → ≥ 1 | NOT MET at authoring |

If either is incomplete, this plan cannot start, full stop.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue.

## 1. Goal

- `mfb pkg remove <ident>` removes the dependency from `project.json`,
  re-resolves, rewrites `mfb.lock`, and deletes the package's installed files.
- When other declared packages import the target, all of them are listed and
  removed together after confirmation.
- Removing the last registry dependency succeeds rather than erroring.
- A target not declared in `project.json` is an error.

### Non-goals (explicit constraints)

- **No pruning of unrelated orphans.** `remove` deletes files belonging to the
  packages it removed, nothing else. A general `packages/` garbage collector is a
  separate concern.
- **No change to `resolve()`'s selection algorithm**, including the
  silently-dropped-edge behavior at `src/cli/resolve.rs:253`. This letter works
  *around* that behavior by computing the cascade itself; changing the resolver
  to error on undeclared edges would affect every existing project and is out of
  scope.
- **No change to `mfb.lock`'s format.**
- **No network requirement.** The cascade is computed from locally installed
  `.mfp` files (§4.2).

## 2. Current State

`run_pkg_command` (`src/cli/pkg.rs:24`) has no `remove` arm; an attempt falls
through to the unknown-subcommand error at `:111`.

`resolve()` seeds nodes only from dependencies declared in `project.json`
(`src/cli/resolve.rs:172-178`). When it re-reads a selected package's import
table, it applies each edge only if the imported ident is already a node:

```rust
if let Some(target) = nodes.get_mut(&imported_ident) {   // src/cli/resolve.rs:253
```

An edge naming an undeclared ident is silently discarded.

`imported_vendor_dir(project_root, declaring_unit)` (`src/manifest/libraries.rs:443`)
returns `packages/<declaring_unit>.vendor`. `install_vendor_blobs`
(`src/cli/pkg.rs:703`) populates it.

`read_package_info(path)` (`src/binary_repr/mod.rs:451`) reads a `.mfp` from disk
and returns package info including its import list — the same structure
`load_import_edges` (`src/cli/resolve.rs:426`) derives from a downloaded blob via
`package_info_from_mfp`.

### Measured populations

| What | Count | Command |
|---|---|---|
| `remove` dispatch arms today | 0 | `grep -c '"remove"' src/cli/pkg.rs` → 0 |
| Per-package on-disk artifacts to delete | 2 | `packages/<name>.mfp` (`src/cli/pkg.rs:713`) and `packages/<name>.vendor/` (`src/manifest/libraries.rs:443`) |
| Sites that silently drop an undeclared import edge | 1 | `src/cli/resolve.rs:253` |
| CLI-reference spec rows for `pkg` commands (a row must be added) | 11 | `grep -c '^| \`pkg ' src/docs/spec/tooling/07_cli-reference.md` → 11 (count taken before plan-60-A moves 5 of them to `repo`) |

### Verified properties

- **Removing the last registry dependency would error without plan-60-B §4.3.**
  Read `src/cli/resolve.rs:179-181`: `resolve()` returns
  `"project.json declares no registry dependencies to resolve"` when the seed set
  is empty. This letter is the first command that can reach that state, which is
  why the policy was settled in B rather than here.
- **A dangling dependency resolves cleanly and fails at build time.** Read
  `src/cli/resolve.rs:247-262`: after removing B while A still imports it, A is
  the only node, A's edge to B finds no node and is dropped, resolution succeeds,
  and `mfb.lock` looks correct. Nothing in the resolve or install path reports
  the missing package. This is the failure mode the cascade prevents.
- **File cleanup is hygiene, not correctness.** Read `verify_packages`
  (`src/cli/pkg.rs:983-1003`) — it iterates the **manifest's** `packages` array,
  not the `packages/` directory. Read `collect_files_recursive`
  (`src/cli/build.rs:1848`) — the only `read_dir` in `build.rs`, and it serves
  resource collection, not package discovery. So an orphaned `.mfp` is inert: it
  is not verified, not built against, and not reported. Deleting it is disk
  hygiene and avoids a stale file masquerading as an installed dependency to a
  human reader.
- **The cascade can be computed offline.** `read_package_info`
  (`src/binary_repr/mod.rs:451`) takes a filesystem path, and every declared
  dependency's `.mfp` is already at `packages/<name>.mfp` after a successful
  install. So `remove` needs no registry access to determine what breaks — which
  matters, because refusing to tell a user what a removal will break just because
  they are offline would be a poor trade.

## 3. Design Overview

Four pieces:

1. **The reverse-dependency closure** (§4.2), computed offline from installed
   `.mfp` files.
2. **The confirmation gate** (§4.3), shown only when the closure exceeds the
   named target.
3. **The manifest edit and re-resolution**, via `apply_manifest_change`.
4. **File cleanup** (§4.5), after resolution has succeeded.

**Design uncertainty: concentrated in §4.4**, the incomplete-install case — what
to do when a declared package's `.mfp` is missing and its imports therefore
cannot be read. The cascade is only as trustworthy as its input, and a silently
incomplete cascade is worse than none, because it would present a confident list
that is wrong.

**Correctness risk: concentrated in §4.5**, file deletion. It is the only
irreversible operation in plan-60. It runs last, only after resolution has
succeeded, and only against names the command itself removed.

**Rejected alternative: refuse the removal instead of cascading** — i.e.
`"alice#shape is still imported by alice#widget; remove that first"`. Rejected by
explicit decision. The cascade with a printed list gives the same information
plus a way to act on it in one step; refusal makes the user re-derive the
dependency order by hand.

**Rejected alternative: compute the cascade from the registry** using
`load_import_edges`. Rejected — it makes `remove` require network access and a
valid session to answer a question that local files already answer, and it would
report edges from the *resolved* versions rather than the *installed* ones, which
is the wrong truth for "what will break on disk".

## 4. Detailed Design

### 4.1 Command shape

```
mfb pkg remove <owner>#<pkg> [--yes]
```

Cwd-only, matching `add` and `update`. Exactly one positional.

A target not declared in `project.json` errors before anything else:
`` `alice#shape` is not declared in project.json ``.

### 4.2 The reverse-dependency closure

1. Read `project.json`; collect every declared registry dependency
   (`ident.contains('#')`, matching `src/cli/resolve.rs:176`).
2. For each, read `packages/<name>.mfp` with `read_package_info`
   (`src/binary_repr/mod.rs:451`) and collect the idents it imports, filtered to
   those containing `#` — the same filter `load_import_edges` applies
   (`src/cli/resolve.rs:445`).
3. Build the reverse map `ident → {idents that import it}`.
4. Compute the transitive closure from the target: the target, everything
   importing it, everything importing those, and so on. Use an explicit worklist
   with a visited set — an import cycle between two packages must terminate, not
   recurse forever.

The closure is the removal set.

### 4.3 The confirmation gate

When the closure is exactly the named target, remove it without prompting — the
user named it, and nothing else is affected.

When the closure is larger, print the full set with the reason for each, then
call `confirm(question, assume_yes)`:

```
Removing alice#shape will also remove packages that import it:

  alice#shape             (named)
  alice#widget            imports alice#shape
  alice#dashboard         imports alice#widget

Remove all 3 packages? [y/N]:
```

Each non-target line names the *direct* importer that pulled it in, so a
three-level cascade is legible rather than a flat list the user has to
reverse-engineer.

Declining exits 0 having changed nothing. `--yes` bypasses the prompt; on a
non-TTY without `--yes`, `confirm` errors per plan-60-B §4.1.

### 4.4 When a declared package is not installed

If a declared dependency's `packages/<name>.mfp` is missing, its imports cannot
be read and the closure may be incomplete — it could omit a package that imports
the target.

**Policy: error, naming the missing package and the fix.**

```
error: cannot determine what depends on alice#shape — alice#widget is declared
       in project.json but not installed (packages/widget.mfp is missing).
       Run `mfb pkg install` first.
```

Proceeding with a knowingly-incomplete cascade is the one outcome to avoid: it
would print a confident list, remove less than it should, and leave exactly the
dangling-import state the cascade exists to prevent. `--yes` does **not** bypass
this — it is a correctness gate, not a confirmation.

### 4.5 File cleanup

After `apply_manifest_change` returns `Ok`, for each removed package name:

1. Delete `packages/<name>.mfp`.
2. Delete `packages/<name>.vendor/` recursively, via
   `imported_vendor_dir(project_dir, name)` (`src/manifest/libraries.rs:443`) —
   reuse the helper rather than re-deriving the path, so the two stay in step.

A missing file is not an error — the goal state is "absent", and it is already
met.

Cleanup runs **after** resolution succeeds, never before. If
`apply_manifest_change` fails, nothing is deleted and the project is untouched.

Deletion failures (permissions, a file held open) are reported as a warning
rather than a hard error: the manifest and lock are already consistent, and
failing the command at that point would misreport a completed removal as failed.
Name the path so the user can clean up manually.

### 4.6 Removing the last dependency

Handled by plan-60-B §4.3: `apply_manifest_change` writes `project.json`, deletes
`mfb.lock`, and skips resolve and install. §4.5's cleanup then runs normally.

Improve `install`'s message for this state while here, per B's Open Decision: a
project with no declared registry dependencies should get `"nothing to install"`
rather than `"no mfb.lock; run \`mfb pkg update\`"` (`src/cli/resolve.rs:92`),
which reads as an error for a project that is correctly configured.

## Compatibility / Format Impact

**Changes:**

- New command `mfb pkg remove <ident> [--yes]`.
- New interactive prompt (cascade case only).
- `mfb pkg install` in a project with no registry dependencies reports
  `"nothing to install"` and exits 0, instead of reporting a missing lock
  (§4.6).

**Explicitly unchanged:** `project.json` schema; `mfb.lock` format; the selection
algorithm, including the dropped-edge behavior at `src/cli/resolve.rs:253`; the
trust chain; every other command's exit codes.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work it describes. An unticked box means NOT DONE.

### Phase 1 — The closure, as a pure function

The cascade's correctness is the whole value of this letter, and it is fully
testable without a registry or a filesystem.

- [ ] Add a pure
      `removal_closure(target: &str, imports: &BTreeMap<String, Vec<String>>) -> Vec<(String, Option<String>)>`
      to `src/cli/pkg.rs`, returning each removed ident paired with the direct
      importer that pulled it in (`None` for the named target). Worklist +
      visited set per §4.2 step 4.
- [ ] Tests in `src/cli/pkg.rs`: target with no importers → closure of one;
      one-level cascade; three-level chain (asserting the reported importer for
      each); a **cycle** between two packages (must terminate); a diamond, where
      two packages import the target and both import a third (each ident appears
      exactly once).

Acceptance: `cargo test --bin mfb` passes, including the cycle case — which must
terminate rather than overflow the stack. Assert on the returned pairs, not just
the ident set, so the "imports X" attribution is covered.
Commit: —

### Phase 2 — Dispatch, import reading, and the not-installed gate

- [ ] Add a `[command, rest @ ..] if command == "remove"` arm to
      `run_pkg_command` (`src/cli/pkg.rs:24`), parsing one positional plus
      `--yes` with plan-60-C's flag-parsing struct.
- [ ] Implement §4.1's not-declared check against the parsed manifest.
- [ ] Build the imports map per §4.2 steps 1–3, using `read_package_info`
      (`src/binary_repr/mod.rs:451`) against `packages/<name>.mfp`, filtering
      imported idents to those containing `#`.
- [ ] Implement §4.4's not-installed error, naming the missing package and its
      expected path. Confirm `--yes` does **not** bypass it.
- [ ] Tests in `src/cli/pkg.rs`: undeclared target errors; a declared-but-missing
      `.mfp` produces the §4.4 error and does not fall through, including with
      `--yes` set.

Acceptance: `cargo test --bin mfb` passes; the `--yes` + missing-`.mfp` test
asserts the command **fails**, which is the case a careless implementation gets
wrong.
Commit: —

### Phase 3 — Confirmation, manifest edit, re-resolution

- [ ] Implement §4.3: print the closure with attributions when it exceeds the
      target, then `confirm`. No prompt for a single-package removal.
- [ ] Add `project_json_without_packages(contents, names) -> Result<String, String>`
      to `src/manifest/package.rs`, removing one or more dependency entries by
      surgical string edit — the same approach as
      `project_json_with_package` (`:547`) and
      `project_json_with_updated_ident_key` (`:644`), so formatting survives.
      It must handle removing the only entry (leaving `"packages": []`) and
      removing a trailing entry without leaving a dangling comma.
- [ ] Call `apply_manifest_change` with the resulting text.
- [ ] Print a result line naming every removed package.
- [ ] Tests in `src/manifest/package.rs`: remove the first, middle, last, and
      only entry; the result must re-parse as valid JSON in every case (mirror
      the existing `out.parse::<JsonValue>().expect("valid json")` assertions at
      `:1312`, `:1330`, `:1340`).

Acceptance: `cargo test --bin mfb` passes; every
`project_json_without_packages` case asserts the output re-parses as valid JSON.
Commit: —

### Phase 4 — File cleanup and the zero-dependency message (largest blast radius)

The only irreversible operation in plan-60. Last, behind every test above.

- [ ] Implement §4.5: after `apply_manifest_change` succeeds, delete
      `packages/<name>.mfp` and `imported_vendor_dir(project_dir, name)` for each
      removed name. Missing is not an error; a deletion failure is a warning
      naming the path.
- [ ] Implement §4.6's `install` message improvement at `src/cli/resolve.rs:92`:
      distinguish "no lock and no declared registry dependencies" (nothing to
      install, exit 0) from "no lock but dependencies are declared" (the existing
      error).
- [ ] Tests in `tests/repo_acceptance.rs`: publish `alice#dep` and `alice#user`
      where `user` imports `dep`; add both to a consumer; `mfb pkg remove
      alice#dep --yes` removes **both** from `project.json`, rewrites
      `mfb.lock`, and deletes `packages/dep.mfp` and `packages/user.mfp`.
- [ ] Tests in `tests/repo_acceptance.rs`: removing the only dependency deletes
      `mfb.lock` entirely and leaves `mfb pkg install` exiting 0 with
      `"nothing to install"`.
- [ ] Tests in `tests/repo_acceptance.rs`: a vendoring package's
      `packages/<name>.vendor/` directory is gone after removal. Reuse the
      vendor-blob fixture at `tests/repo_acceptance.rs:1755`.

Acceptance: `cargo test --test repo_acceptance` passes. The cascade test must
assert `alice#user` is absent from `project.json` — asserting only that the
command succeeded would pass even if the cascade silently removed nothing but the
named target.
Commit: —

### Phase 5 — Docs

- [ ] `src/main.rs:96` `PKG_HELP`: add `remove <target>` and `--yes` to Options.
- [ ] `src/main.rs:45` `USAGE`: add `pkg remove` to the pkg block if it fits the
      block's existing selectivity; otherwise leave the `mfb pkg --help` pointer.
- [ ] `src/docs/spec/tooling/07_cli-reference.md`: add a `pkg remove` row —
      `mfb pkg remove <owner>#<pkg> [--yes]`, `0 ok; 2 usage; 1 failed`.
- [ ] Document the cascade in the tooling spec: why it exists (the dropped-edge
      behavior at `src/cli/resolve.rs:253` means a dangling import resolves
      cleanly), the offline computation, and the not-installed gate. Cite
      `[[src/cli/resolve.rs:resolve]]` and `[[src/manifest/libraries.rs:imported_vendor_dir]]`.
- [ ] Document the zero-dependency lockfile policy (plan-60-B §4.3) — this is the
      first letter where it becomes user-visible, so it becomes documentable here.

Acceptance: `cargo build && cargo test --bin mfb spec` pass; `mfb spec tooling
--all` renders with no leaked `[[` markers.
Commit: —

## Validation Plan

- **Tests:** closure table including cycle and diamond (unit);
  `project_json_without_packages` position cases with JSON re-parse (unit);
  not-declared and not-installed gates (unit); cascade, last-dependency, and
  vendor-directory removal (acceptance).
- **Coverage check:** `removal_closure` and `project_json_without_packages` are
  pure and must sit outside any `// coverage:off` region. The `remove` command
  body reaches the registry through `apply_manifest_change` and will be excluded;
  logic written inline there would look tested while being invisible.
- **Runtime proof:** against the acceptance-harness registry — build a consumer
  depending on two packages where one imports the other, run
  `mfb pkg remove <ident>` interactively, confirm the printed cascade list
  matches the actual dependency direction, accept, and confirm both
  `packages/*.mfp` files are gone and `mfb build` still succeeds on the
  now-smaller project.
- **Doc sync:** `src/main.rs` (`USAGE`, `PKG_HELP`),
  `src/docs/spec/tooling/07_cli-reference.md`, tooling spec cascade and
  zero-dependency sections.
- **Acceptance:** `cargo build && cargo test --bin mfb && cargo test --test repo_acceptance`,
  **and `scripts/test-accept.sh target/debug/mfb target/accept-actual`** — the
  project gate required by `.ai/compiler.md:67` for any change that can affect
  generated diagnostics, which CLI output is. The three cargo commands alone do
  **not** see the goldens that embed CLI output: in plan-60-A they missed a
  `USAGE` golden entirely (plan-60-A Corrections #8). Any letter here that
  changes `USAGE`/`PKG_HELP`/`REPO_HELP` or a command's printed output must
  expect `tests/syntax/packages/audit-usage/golden/audit_usage.audit` to move,
  and must run the four-question check in AGENTS.md before regenerating it.

## Open Decisions

- **Should `remove` also drop a `file://`-sourced dependency's copied `.mfp`?**
  Recommended: **yes, identically.** `add_package_from_file` copies the blob into
  `packages/<name>.mfp` (`src/cli/pkg.rs:593-595`), so the artifact is the
  project's own copy, not the user's original file — deleting it removes nothing
  the user still has elsewhere. Confirm during Phase 4 that the source path
  outside the project is never touched. (§4.5)

## Corrections

<!-- Filled in DURING execution. -->

## Summary

The engineering risk is Phase 4's deletion, and the design puts three gates in
front of it: the closure must be complete (§4.4 errors rather than guessing),
resolution must have already succeeded, and only names this command removed are
eligible. The cascade itself exists because of one line —
`src/cli/resolve.rs:253` — where an import edge naming an undeclared package is
silently dropped, making a dangling dependency look like a clean resolve.

Untouched: the resolver's dropped-edge behavior itself, transitive installation,
the lockfile format, and every other command.
