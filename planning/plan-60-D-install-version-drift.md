# plan-60-D: `mfb pkg install` — specific version-drift diagnosis

Last updated: 2026-07-21
Effort: small (< 1h)
Depends on: plan-60-C
Produces: the manifest↔lock field-diff used by `install` to explain drift. Not
consumed by any later letter — D, E and F are independent siblings.

`mfb pkg install` currently rejects **any** difference between `project.json` and
`mfb.lock` with one opaque message, computed by comparing a single SHA-256 over
every dependency tuple. It cannot say which package drifted, which field changed,
or whether the difference matters. This letter replaces the verdict with a
field-level diff and splits the outcome by pin state.

**Behavioral outcome:** with a `pin: false` dependency whose `project.json`
version was bumped past what `mfb.lock` records, `mfb pkg install` prints a
warning naming the package, both versions, and the command that reconciles them —
then installs the locked selection and exits 0. With a `pin: true` dependency in
the same situation it exits 1 with an equally specific message and installs
nothing.

References:

- `src/cli/resolve.rs:89-155` — `install`
- `src/audit/collect/mod.rs:87` — `project_hash`, the current all-or-nothing check
- `src/manifest/package.rs:312` — the build-time pin check this letter front-runs
- `src/docs/spec/tooling/07_cli-reference.md:53` — the `pkg install` row

## Prerequisites

See plan-60-A for the plan-wide prerequisite gate. In addition:

| Must be true | Command | Status |
|---|---|---|
| plan-60-C complete — a `pin: false` dependency is creatable from the CLI | `grep -cE 'no_pin' src/cli/pkg.rs` → ≥ 1 **and** `mfb pkg add --no-pin <ident>` parses (a bare grep also matches a test name, so confirm the flag is dispatched, not merely mentioned) | NOT MET at authoring |

If plan-60-C is not complete, this plan cannot start, full stop.

> **Corrected 2026-07-21 (plan-60-C Corrections).** These rows originally used
> unanchored greps (`grep -c 'fn confirm'`, `grep -c 'fn apply_manifest_change'`)
> that count **every** mention, not the definition — and plan-60-B names its tests
> after the functions they test, so they return 3 and 4 rather than 1. Anchored on
> `^pub(crate) fn`. This was the recurring defect of plan-60's gate checks
> (plan-60-B Corrections #1, plan-60-C Prerequisites): a grep for a spelling
> cannot tell a definition from a test asserting something about it. Check the
> construct.
 Before C, the
only way to produce the `pin: false` dependency this letter's warning path exists
for is to hand-edit `project.json`, so the warning could not be exercised by any
CLI-driven test.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue.

## 1. Goal

- Drift on a `pin: false` dependency's version produces a warning naming the
  package and both versions; `install` proceeds and exits 0.
- Drift on a `pin: true` dependency's version produces an error naming the
  package and both versions; `install` exits 1 having installed nothing.
- Every other class of drift (a dependency added, removed, or changed in a field
  the lock does not record) remains a hard error, with a message that says which
  package and which class.

### Non-goals (explicit constraints)

- **No change to `mfb.lock`'s format or `lockfileVersion`.** See the Open
  Decision — recording `pin` in the lock would make one case diagnosable that
  currently is not, and is deliberately deferred.
- **No change to what `install` installs.** It installs the locked `hash`, always.
  The warning path does not silently switch to the manifest's version — saying so
  precisely is the whole point of the message.
- **No change to the trust chain**: the pinned-metadata check
  (`src/cli/resolve.rs:110`), the `repoFingerprint` check (`:113-119`) and
  `install_verified_package` are untouched and still run before anything is
  written.
- **No new flags.** No `--frozen` / `--locked` escape hatch in this letter.

## 2. Current State

`install` (`src/cli/resolve.rs:89`) performs, in order:

1. `read_manifest` (`:90`)
2. `read_lock` → absent is an error naming `mfb pkg update` (`:91-94`)
3. **`project_hash(&manifest) != lock.project_hash` → hard error** (`:96-102`),
   message: `"mfb.lock is stale (project.json changed since it was written); run
   \`mfb pkg update\`"`
4. pinned-metadata verification (`:110`)
5. `repoFingerprint` match against the pinned server key (`:113-119`)
6. per-package `fetch_blob` + `install_verified_package` + `install_vendor_blobs`
   (`:127-150`)

`project_hash` (`src/audit/collect/mod.rs:87-114`) hashes, per dependency,
`name∥ident∥version∥pin∥source` joined by NULs, sorted, with no filter on ident
or source. Any change to any of those five fields on any dependency changes the
digest.

`LockedPackage` (`src/cli/resolve.rs:32-47`) records `name`, `ident`,
`requested`, `selected`, `hash`, `ident_key`, `ident_fingerprint`, `state`.

A later, independent check exists at build time: `src/manifest/package.rs:312`
errors with `"package \`{}\` is pinned to version {}, but installed package is
version {}"` when a `pin: true` dependency's declared version differs from the
installed `.mfp`'s header version.

### Measured populations

| What | Count | Command |
|---|---|---|
| `pkg install` arg-vector sites in `tests/` | 1 | `grep -rn '"pkg", "install"' tests/ \| wc -l` → 1 |
| Fields hashed by `project_hash` per dependency | 5 | `src/audit/collect/mod.rs:96-105` — name, ident, version, pin, source |
| Fields recorded per package in `mfb.lock` | 8 | `src/cli/resolve.rs:32-47` — name, ident, requested, selected, hash, identKey, identFingerprint, state |
| Hashed fields that are **also** in the lock | 3 | name, ident, version(↔`requested`) — set intersection of the two above |
| CLI-reference spec rows for `pkg install` | 1 | `src/docs/spec/tooling/07_cli-reference.md:53` |

### Verified properties

- **Only three of the five hashed fields can be diffed against the lock.** Read
  `project_hash` (`src/audit/collect/mod.rs:96-105`) and the `LockedPackage`
  struct (`src/cli/resolve.rs:32-47`). `pin` and `source` are hashed but **not**
  recorded in the lock, so a change to either produces a hash mismatch that a
  field diff cannot attribute. §4.2 handles this explicitly rather than letting
  it fall through as a false "no drift found".
- **The `pin: true` error this letter adds duplicates a check that already exists
  later**, at `src/manifest/package.rs:312` (build time, against the installed
  `.mfp` header). Read both. This letter does not remove the build-time check —
  it front-runs it so the failure names `mfb.lock` at the moment the lock is
  applied, rather than surfacing as a confusing build error afterwards.
- **The warning path is genuinely safe.** `install` installs `package.hash`
  (`src/cli/resolve.rs:128`), which is the resolved selection, and every blob
  still passes the full §3.5 chain via `install_verified_package` (`:132-138`).
  Warning and continuing therefore cannot install anything unverified — it can
  only install something *older or different than the manifest now asks for*,
  which is exactly what the message must say.

## 3. Design Overview

One piece: replace the boolean hash verdict at `src/cli/resolve.rs:96-102` with a
diff that classifies each difference, then act on the classification.

The hash check is **kept** as the trigger. It is cheap, it is the authoritative
"something changed" signal, and it covers fields the diff cannot see. The diff
runs only when the hash mismatches, and its job is to explain the mismatch, not
to detect it.

**Design uncertainty: low.** The only genuinely open question — what to do about
the two undiffable fields — is answered conservatively in §4.2 and recorded as an
Open Decision.

**Correctness risk: the false-negative.** A diff that finds nothing to report on
a real mismatch must not fall through to a successful install. §4.2's default is
therefore "error", not "proceed".

**Rejected alternative: drop the hash check and rely on the field diff alone.**
Rejected — `pin` and `source` drift would then be invisible, and a `source`
change is precisely the case where installing the old locked blob is wrong.

## 4. Detailed Design

### 4.1 The diff

Runs only when `project_hash(&manifest) != lock.project_hash`. For each registry
dependency in the manifest and each entry in `lock.packages`, matched on `ident`:

| Condition | Class | Outcome |
|---|---|---|
| manifest dep has no lock entry | `Added` | error |
| lock entry has no manifest dep | `Removed` | error |
| `dep.version != entry.requested`, `dep.pin == false` | `FloorMoved` | **warn, continue** |
| `dep.version != entry.requested`, `dep.pin == true` | `PinMoved` | error |
| `dep.name != entry.name` | `Renamed` | error |
| no difference found on any diffable field | `Unattributable` | error (§4.2) |

Classification collects **all** differences before deciding, so a project with
three drifted dependencies reports three lines rather than the first one. The
outcome is: error if any class other than `FloorMoved` is present; otherwise warn
for each `FloorMoved` and proceed.

### 4.2 The unattributable case

`pin` and `source` are hashed but not recorded in the lock (§2, verified). A flip
of either produces a mismatched hash with every diffable field equal.

**Policy: treat `Unattributable` as an error**, with a message that says so
honestly rather than inventing a cause:

```
error: mfb.lock does not match project.json, but the difference is not in a
field the lock records (most likely `pin` or `source` changed); run `mfb pkg update`
```

This is the conservative direction: the alternative — proceeding because nothing
recognizable changed — would install the old locked blob after a `source` change
pointed the dependency somewhere else entirely.

### 4.3 Message shapes

`FloorMoved` (warning, stderr, exit stays 0):

```
warning: libsnd is floating and project.json now declares 1.4.0 as its ABI floor,
         but mfb.lock was resolved against 1.3.0 and selects 1.3.2.
         Installing the locked selection. Run `mfb pkg update` to re-resolve.
```

`PinMoved` (error, exit 1):

```
error: libsnd is pinned to 1.4.0 in project.json but mfb.lock records 1.3.0.
       Run `mfb pkg update` to re-resolve, or restore the pinned version.
```

Both name the package, both versions, and the reconciling command. The warning
additionally names the **selected** version, because under `pin: false` the
locked `requested` and `selected` differ and the user needs to know which one is
about to land on disk.

Warnings go to stderr, consistent with the build-output convention recorded in
`planning/old-plans/plan-36-build-progress-output.md` (progress and diagnostics on
stderr, the artifact line on stdout).

### 4.4 Ordering

The diff runs where the current hash check runs — **step 3**, before the
pinned-metadata and `repoFingerprint` checks. A stale lock is a
project-consistency problem and should be reported before network trust state, so
that a user with a drifted manifest and no registry session gets the message that
actually applies to them.

## Compatibility / Format Impact

**Changes:**

- `mfb pkg install` exits **0 instead of 1** for one previously-fatal case:
  version drift on a `pin: false` dependency. This is a deliberate relaxation.
- Error text for every other drift class becomes specific rather than the single
  `"mfb.lock is stale"` string. **Any test asserting that exact string will
  need updating** — see Phase 2.
- New stderr warning output.

**Explicitly unchanged:** `mfb.lock` format and `lockfileVersion`; `project.json`
schema; what gets installed; the trust chain; the exit code for every drift class
other than `FloorMoved`.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work it describes. An unticked box means NOT DONE.

### Phase 1 — The diff, as a pure function

- [x] Add a `DriftClass` enum and a pure
      `classify_drift(manifest, lock_packages) -> Vec<(String, DriftClass)>`
      to `src/cli/resolve.rs`, implementing §4.1's table. No I/O, no registry.
      Takes the parsed **manifest** rather than a pre-extracted dependency list,
      so it reuses `is_registry_dependency` and cannot drift from the resolver's
      notion of which entries participate (see plan-60-C Corrections #1).
- [x] ~~Include the `Unattributable` class per §4.2~~ — **moot as an enum
      variant** (Corrections #1): `Unattributable` is not a property of any
      dependency, it is the *absence* of classification combined with a fact only
      `install` holds (that the hash mismatched). Modelling it as a variant would
      require a fake ident to attach it to. It is Phase 2's outcome rule instead:
      empty result + observed mismatch → the §4.2 error. The empty-result case is
      tested here.
- [x] Tests in `src/cli/resolve.rs`, one per row of §4.1: added, removed,
      floor-moved on `pin: false`, pin-moved on `pin: true`, renamed, and the
      empty-result case. Plus one the plan did not list —
      `classify_drift_reports_every_drifted_dependency`, pinning §4.1's
      collect-all-before-deciding rule across four simultaneously-drifted
      dependencies.

Acceptance: `cargo test --bin mfb` passes with one test per class, including an
explicit assertion that a `pin: false` version difference classifies as
`FloorMoved` and a `pin: true` one as `PinMoved`. **VERIFIED** — 3156 passed / 0
failed (from 3154). The Floor/Pin pair is asserted on *identical* version drift
with only `pin` differing, so the two cannot be satisfied by the same code path.
Commit: —

### Phase 2 — Wire it into `install`

- [ ] Replace `src/cli/resolve.rs:96-102` with: hash comparison as the trigger,
      then `classify_drift`, then §4.1's outcome rule.
- [ ] Emit §4.3's messages — warnings to stderr via `eprintln!`, errors as the
      returned `Err` string.
- [ ] Keep the ordering at §4.4: the drift check stays ahead of
      `verify_pinned_metadata` (`:110`) and the `repoFingerprint` check (`:113`).
- [ ] Grep for tests asserting the old string:
      `grep -rn 'mfb.lock is stale' src/ tests/` — update each to the new
      class-specific message. If a test's intent was "any drift is refused",
      re-point it at a class that still errors (e.g. `PinMoved`) rather than
      weakening it.
- [ ] Tests: extend `tests/repo_acceptance.rs` with two cases — a `pin: false`
      dependency whose manifest version is bumped past the lock (assert exit 0,
      assert the warning appears on stderr, assert `packages/<name>.mfp` still
      holds the **locked** version's bytes), and a `pin: true` dependency in the
      same situation (assert exit 1, assert `packages/` is unchanged).

Acceptance: `cargo test --bin mfb && cargo test --test repo_acceptance` pass. The
`pin: false` acceptance case must assert the installed bytes match the *locked*
selection — asserting only the exit code would pass even if the warning path
wrongly installed the manifest's version.
Commit: —

### Phase 3 — Docs

- [ ] `src/docs/spec/tooling/07_cli-reference.md:53` — the `pkg install` row's
      exit-code column currently reads `0 ok; 2 usage; 1 stale lock or failed`.
      Update to distinguish the warn case from the error case.
- [ ] Document the drift classes and their outcomes in the tooling spec's
      lockfile section, citing `[[src/cli/resolve.rs:classify_drift]]`. Per
      `.ai/specifications.md`, lockfile and CLI output are both named contracts
      requiring a same-change spec update.
- [ ] Note in the spec that `pin` and `source` are covered by `projectHash` but
      not recorded in the lock, and that drift in either is therefore reported as
      unattributable (§4.2). This is the kind of asymmetry that reads as a bug
      later if it is not written down as intentional.

Acceptance: `cargo build && cargo test --bin mfb spec` pass; `mfb spec tooling
--all` renders with no leaked `[[` markers.
Commit: —

## Validation Plan

- **Tests:** one unit test per §4.1 class (`src/cli/resolve.rs`); two acceptance
  cases covering the warn and error paths end to end, the warn case asserting
  installed **bytes**, not just the exit code.
- **Coverage check:** `classify_drift` is pure and must sit outside `install`'s
  `// coverage:off` region — `install` reaches the network and is excluded, so a
  diff function written inside it would be invisible to the suite while looking
  tested.
- **Runtime proof:** in a scratch project against the acceptance-harness
  registry — `mfb pkg add alice#shape` (floating, per plan-60-C), hand-edit
  `project.json`'s version upward, run `mfb pkg install`, confirm exit 0 + the
  warning + unchanged `packages/` bytes. Repeat with `"pin": true` and confirm
  exit 1.
- **Doc sync:** `src/docs/spec/tooling/07_cli-reference.md` and the tooling
  spec's lockfile section.
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

- **Record `pin` in `mfb.lock` to make §4.2's unattributable case diagnosable?**
  Recommended: **not in this letter.** It is a lockfile format change, it would
  oblige a `lockfileVersion` decision and a `read_lock` compatibility path
  (`src/cli/resolve.rs:604`), and it buys a better message for one uncommon case.
  Revisit only if the unattributable error proves confusing in practice. (§4.2)

## Corrections

**#1 — `Unattributable` cannot be a `DriftClass` variant.** (Phase 1,
2026-07-21.) Phase 1 asks for it in the enum, "emitted when the caller reports a
hash mismatch but classification found nothing". But `classify_drift` returns
`Vec<(ident, DriftClass)>` — every element is *about a specific dependency*, and
`Unattributable` is by definition about none of them: it is the absence of any
finding, combined with a fact the classifier does not have (that `projectHash`
mismatched). Emitting it would mean inventing an ident to hang it on.

Modelled instead as Phase 2's outcome rule: empty classification + observed hash
mismatch → the §4.2 error. This keeps `classify_drift` pure and total — it
answers "what differs?", while `install` answers "and what should happen?", which
is the only place that knows a mismatch occurred. The §4.2 behavior and message
are unchanged; only where the decision lives.

**#2 — `classify_drift` takes the manifest, not a dependency list.** (Phase 1,
2026-07-21.) The signature in Phase 1 is `classify_drift(manifest_deps,
lock_packages)`. Implemented as `classify_drift(manifest, lock_packages)` so the
function applies `is_registry_dependency` itself. Passing a pre-extracted list
would let a caller filter differently from the resolver — exactly the drift that
caused plan-60-C's data-loss bug, where two copies of "which entries are registry
dependencies" disagreed. One extraction path, one definition.

## Summary

The engineering risk is the false-negative: a drift the classifier cannot
attribute must not fall through into a successful install. §4.2 defaults to
error for exactly that reason, and the acceptance test for the warning path
asserts installed bytes rather than exit code so that a wrong-version install
cannot pass as green.

Untouched: the lockfile format, the trust chain, and what `install` actually
installs. This letter changes only what `install` says and, in one specific and
deliberate case, whether it stops.
