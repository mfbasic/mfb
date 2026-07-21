# plan-60-C: `mfb pkg add` — resolve-first, lock-writing, pin inference

Last updated: 2026-07-21
Effort: medium (1h–2h)
Depends on: plan-60-B
Produces:
- `mfb pkg add` writing `mfb.lock` (closes the stale-lock hole below).
- The `--pin` / `--no-pin` flags and the pin-inference rule (§4.1).
- CLI-creatable `pin: false` dependencies — which D's warning path and E's
  pin-preservation rule both require in order to be testable at all.
- Flag parsing on a `pkg` subcommand that takes both flags and a positional,
  reused by E and F.

Today `mfb pkg add` leaves the project in a state where `mfb pkg install`
refuses to run. It writes a new dependency into `project.json` but never touches
`mfb.lock`; `projectHash` (`src/audit/collect/mod.rs:87`) covers every dependency
tuple, so the lock is stale the instant `add` returns and `install` hard-errors
with `"mfb.lock is stale (project.json changed since it was written)"`
(`src/cli/resolve.rs:100`). This letter closes that hole and, in the same change,
makes `add` default to a floating dependency rather than a pinned one.

**Behavioral outcome:** after `mfb pkg add alice#shape` in a clean project,
`mfb pkg install` succeeds without an intervening `mfb pkg update`, the
dependency is recorded with `"pin": false`, and the printed line ends in
`(floating)`. After `mfb pkg add alice#shape@1.2.0`, the same holds with
`"pin": true` and `(pinned)`.

References:

- plan-60-B §4.2 — `apply_manifest_change`, the resolve-first pipeline
- `src/cli/pkg.rs:1177` — `run_pkg_doc`, the existing flag-parsing precedent
- `src/docs/spec/package-manager/01_repository-protocol.md:832-835` — resolution
  eligibility per version state
- `src/docs/spec/tooling/07_cli-reference.md:51` — the `pkg add` row

## Prerequisites

See plan-60-A for the plan-wide prerequisite gate. In addition:

| Must be true | Command | Status |
|---|---|---|
| plan-60-A complete | `sed -n '/pub(crate) fn run_pkg_command/,/^}/p' src/cli/pkg.rs \| grep -c 'publish_package_project\|transfer_offer\|transfer_accept\|set_release_state\|check_abi'` → 0 | **MET** (2026-07-21). Archived to `planning/old-plans/`. |
| plan-60-B Phases 1–2 complete — `apply_manifest_change` and `confirm` exist | `grep -cE '^pub\(crate\) fn apply_manifest_change' src/cli/resolve.rs` → 1 **and** `grep -cE '^pub\(crate\) fn confirm' src/cli/mod.rs` → 1 | **MET** (2026-07-21) |
| plan-60-B Phase 3 outstanding — **C must complete it** | see plan-60-B Corrections #5; the resolve-first atomicity test is deferred into C's Phase 3 below | **DISCHARGED** (2026-07-21) — landed in C Phase 3 (`ddb4c8898`); plan-60-B archived to `planning/old-plans/` |

If either of the first two is incomplete, this plan cannot start, full stop.

> **Corrected 2026-07-21.** The plan-60-A row originally checked
> `grep -c '"publish"' src/cli/pkg.rs` → 0, which does not measure the stated
> condition: after plan-60-A that returns 3, all legitimate (the moved-command
> guard A deliberately added, plus two test assertion lists). Read literally it
> would block this letter on plan-60-A having *succeeded*. Same defect and same
> fix as plan-60-B Corrections #1 — check the construct, not the spelling.
>
> The plan-60-B row had **the same flaw a second time**: `grep -c 'fn
> apply_manifest_change'` returns **4**, not 1, and `grep -c 'fn confirm'`
> returns **3**, not 1 — because plan-60-B's own tests are named after the
> functions they test, and an unanchored `fn <name>` matches every one of them.
> Both now anchor on `^pub(crate) fn`, which matches the definition and nothing
> else. This is the **third** miscalibrated gate check in plan-60 (B #1, and both
> rows here); D/E/F should be assumed to carry the same pattern until checked.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue.

## 1. Goal

- `mfb pkg add <ident>` resolves before mutating, writes `project.json` **and**
  `mfb.lock`, and installs — leaving `mfb pkg install` immediately runnable.
- Pin state follows §4.1's inference matrix, with `--pin` / `--no-pin` overrides.
- A resolution failure leaves `project.json` and `mfb.lock` byte-identical.

### Non-goals (explicit constraints)

- **No change to `file://` add semantics** beyond lock maintenance. A `file://`
  package stays `pin: true` unconditionally (§4.2) — there is no registry to
  float against.
- **No transitive dependency installation.** `resolve()` seeds nodes only from
  dependencies declared in `project.json` and silently drops import edges naming
  an undeclared ident (`src/cli/resolve.rs:253`). This letter does not change
  that; it is pre-existing behavior and out of scope.
- **No change to the `.mfp` format, `mfb.lock` format, or `lockfileVersion`.**
- **No change to the trust chain.** The plan-23 §3.5 verification in
  `install_verified_package` is untouched.

## 2. Current State

`add_package` (`src/cli/pkg.rs:529`) branches on the target:

- `file://…` → `add_package_from_file` (`:541`)
- contains `#` → `add_package_from_registry` (`:614`)
- otherwise → usage error (`:535`)

`add_package_from_registry` runs install-first, in this order:

1. split `@version` off the target (`:615`)
2. `fetch_index` (`:635`)
3. `select_index_version` (`:639`) — exact version if given, else newest
   floating-eligible
4. `fetch_blob` (`:640`), verify, `install_verified_package` (`:660`)
5. `install_vendor_blobs` (`:666`)
6. build a `ProjectPackageDependency` with **`pin: true` hardcoded** (`:672`)
7. `project_json_with_package` + `fs::write` (`:676-677`)

`mfb.lock` is never written. `project_json_with_package`
(`src/manifest/package.rs:547`) rejects an already-declared name (`:564-569`).

Dispatch is `[command, url] if command == "add"` (`src/cli/pkg.rs:26`) — exactly
two arguments, cwd-only via `Path::new(".")`. There is no flag handling.

### Measured populations

| What | Count | Command |
|---|---|---|
| `pkg add` arg-vector sites in `tests/` | 16 | `grep -rn '"pkg", "add"' tests/ \| wc -l` → 16 |
| Hardcoded `pin: true` sites in the add paths | 2 | `grep -n 'pin: true' src/cli/pkg.rs` → `:577` (file), `:672` (registry) |
| CLI-reference spec rows for `pkg add` | 1 | `src/docs/spec/tooling/07_cli-reference.md:51` |
| Flag-parsing precedents in `run_pkg_command` | 1 | `run_pkg_doc`, `src/cli/pkg.rs:1177-1200` |

### Verified properties

- **`add` leaves the lock stale, and `install` then refuses.** Read
  `add_package_from_registry` (`:614-682`) — no `write_lock` call, no `resolve`
  call. Read `project_hash` (`src/audit/collect/mod.rs:87-114`) — it hashes
  `name∥ident∥version∥pin∥source` for every dependency, with **no filter** on
  ident or source. Read `install` (`src/cli/resolve.rs:96-102`) — any hash
  mismatch is a hard error. The three together make the hole certain, not
  suspected.
- **`file://` adds also write `pin: true`** (`src/cli/pkg.rs:577`) and are
  correct to do so: a local file has no version stream to float along.
- **`select_index_version`'s floating branch already excludes yanked**, via
  `state_is_floating_eligible` (`src/cli/pkg.rs:786-788`, `matches!(state,
  "available" | "deprecated")`), and its exact branch admits any non-`blocked`,
  non-`legal-tombstoned` state (`:809-814`). This matches the spec at
  `src/docs/spec/package-manager/01_repository-protocol.md:832-835` — read both.
- **CONFIRMED (Phase 1 spike, 2026-07-21): a `file://`-added package whose `.mfp`
  ident contains `#` WAS treated as a registry dependency, and `mfb pkg update`
  silently replaced the user's local file with a registry blob.** The §5 defect
  branch is the real one. The reading of the seed filter was correct: it keyed on
  **ident**, not `source`, and `add_package_from_file` copies the ident out of the
  `.mfp` header (`src/cli/pkg.rs:566`), not out of the URL — so a
  published-then-file-added package carries registry coordinates.

  **Observed, not inferred.** The spike published `alice#spike_pkg@0.2.0` to the
  registry and added `0.1.0` locally by `file://`. `mfb pkg update` then printed:

  ```
  Resolution:
    + spike_pkg 0.2.0 (available)
  Wrote 1 resolved package(s) to mfb.lock
  Installed spike_pkg 0.2.0 (available)
  ```

  `mfb.lock` recorded `"requested": "0.2.0", "selected": "0.2.0"`, and the bytes
  of `packages/spike_pkg.mfp` changed — the version string in the header went
  from `0.1.0` to `0.2.0`. The user's local package was replaced by a different
  version with no diagnostic and no way to opt out.

  This is a **pre-existing data-loss defect in `mfb pkg update`**, not one this
  letter introduces — but this letter routes `add` through the same path, which
  would have made it reachable from `add` too. Fixed here per §5 and AGENTS.md
  ("a bug you find is a bug you fix"). Regression test:
  `spike_file_added_package_with_registry_ident_survives_update` in
  `tests/repo_acceptance.rs`, A/B-verified — restoring the ident-only filter
  makes it fail.

## 3. Design Overview

Three pieces, ordered so the one unproven premise is falsified first.

1. **Phase 1 — settle the `file://` + `#`-ident question** with a real
   reproduction against the acceptance harness. This is a spike, not an audit:
   it can invalidate §4.2's design, so it runs before anything is written.
2. **Phase 2 — flag parsing and the inference matrix**, a pure function that is
   fully unit-testable with no registry.
3. **Phase 3 — rewire both add paths onto `apply_manifest_change`.**

**Design uncertainty: concentrated entirely in the Phase 1 question.** Everything
else in this letter is determined by code already read.

**Correctness risk: concentrated in Phase 3**, which changes the order of
irreversible operations (network install vs. manifest write) on the most-used
package command — 16 test sites depend on its current behavior.

**Rejected alternative: keep `pin: true` as the `add` default and add `--float`.**
Rejected by explicit decision. Defaulting to pinned means nothing in the CLI ever
produces a floating dependency, which leaves `resolve()`'s entire ABI-superset
search (`src/cli/resolve.rs:388-400`) unreachable from the CLI for top-level
dependencies — load-bearing code with no way to exercise it.

**Rejected alternative: infer pin state in `update` too.** Rejected — see
plan-60-E §4.1. `add` infers because there is no prior intent to respect;
`update` operates on a dependency that already carries a declared `pin`, and
silently flipping it as a side effect of a version bump is a change nobody asked
for.

## 4. Detailed Design

### 4.1 The pin-inference matrix

| Invocation | `version` written | `pin` written | Printed suffix |
|---|---|---|---|
| `add alice#shape` | newest floating-eligible | `false` | `(floating)` |
| `add alice#shape@1.4.0` | `1.4.0` | `true` | `(pinned)` |
| `add alice#shape --pin` | newest floating-eligible | `true` | `(pinned)` |
| `add alice#shape@1.4.0 --no-pin` | `1.4.0` | `false` | `(floating, floor 1.4.0)` |
| `add alice#shape --pin --no-pin` | — | — | usage error, exit 2 |

The rule: **an explicit `@version` implies `--pin`; an explicit flag always
wins.** `--pin` and `--no-pin` together are a usage error rather than
last-flag-wins, because the two orderings would otherwise mean different things
with no way to tell from the command line which was intended.

**Why `--no-pin` exists.** Under `pin: false`, the `version` field is not the
version you get — it is the **ABI floor**. `resolve()` looks it up as the anchor
(`src/cli/resolve.rs:186-194`), takes that release's ABI map as the project's
requirement set, and floating selection then picks the highest eligible version
whose ABI is a superset (`:388-400`). Without `--no-pin`, the floor is only ever
settable to "whatever was newest on the day I ran `add`", and the one deliberate
use of the anchor mechanism would require hand-editing JSON.

The printed suffix for the `--no-pin` + `@version` case names the floor
explicitly, because that is the combination whose meaning is least guessable.

### 4.2 `file://` adds

Pin inference does **not** apply. A `file://` add always writes `pin: true`, and
`--no-pin` on a `file://` target is a usage error naming the reason: there is no
registry version stream to float along.

`--pin` on a `file://` target is accepted as a no-op (it is already the
behavior), rather than an error — it is a redundant statement of the truth, not a
contradiction.

Whether a `file://` add invokes resolution at all depends on Phase 1's finding;
§5 states both branches.

### 4.3 Command shape

```
mfb pkg add <file://….mfp | <owner>#<pkg>[@version]> [--pin | --no-pin]
```

Cwd-only, matching the existing `add` (`Path::new(".")`, `src/cli/pkg.rs:27`) and
per the plan-wide decision that manifest-mutating commands do not take a path.

Parsing follows `run_pkg_doc` (`src/cli/pkg.rs:1177-1200`): a `while` loop over
`args`, a `match` on each element, an explicit `flag if flag.starts_with("--")`
arm returning `unknown flag \`{flag}\``, and a positional arm that errors if a
second positional appears. Extract this into a small struct so E and F reuse it
rather than each growing a copy.

## 5. Contingent design — resolved by Phase 1

**If a `file://`-added package with an `owner#pkg` ident IS resolved against the
registry today** (the reading of `src/cli/resolve.rs:176` in §2):

This is a pre-existing defect — `mfb pkg update` on such a project already
replaces the user's local file with a registry blob. Per AGENTS.md ("a bug you
find is a bug you fix"), fix it in this letter: change the seed filter at
`src/cli/resolve.rs:176` to key on `source` rather than `ident`, so a dependency
whose `source` begins with `file://` is excluded from registry resolution.
Add a regression test asserting a `file://` dependency survives `mfb pkg update`
with its local bytes intact. Then route both add paths through
`apply_manifest_change` uniformly.

**If it is NOT resolved** (e.g. `file://` adds record a non-`#` ident in
practice): no filter change is needed, and both add paths route through
`apply_manifest_change` unchanged — the `file://` dependency is simply not a
resolver node, while still contributing to `projectHash` and therefore still
requiring the lock rewrite that is this letter's main point.

Record which branch was taken, with the evidence, in Corrections.

## Compatibility / Format Impact

**Changes:**

- `mfb pkg add <ident>` now writes `mfb.lock` and installs the resolved
  selection. Previously it wrote only `project.json` and installed the version it
  picked itself.
- The default `pin` for a registry add flips from `true` to `false`. **This
  changes what a bare `add` records in `project.json`** — existing manifests are
  unaffected, but the same command now produces a different file.
- New flags `--pin` and `--no-pin`.
- New output suffix `(floating)` / `(pinned)` on the success line.

**Explicitly unchanged:** `project.json` schema (the `pin` key already exists and
is already optional — `src/manifest/package.rs:478-487`); `mfb.lock` format;
the `.mfp` format; the trust chain; exit codes.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work it describes. An unticked box means NOT DONE.

### Phase 1 — Spike: does a `file://` dependency get resolved against the registry?

The one unproven premise. It runs first and cheaply because it can invalidate
§4.2 and §5.

- [x] In `tests/repo_acceptance.rs`, publish a package as `alice#spike_pkg`, then
      in a fresh consumer project run `mfb pkg add file://<path to the built
      .mfp>` and inspect the written `project.json` — record the exact `ident`
      and `source` values. → ident `alice#spike_pkg`, source the `file://` URL.
- [x] Run `mfb pkg update` in that consumer and observe whether `mfb.lock` gains
      a `spike_pkg` entry and whether `packages/spike_pkg.mfp` bytes change.
      → **Both.** Lock gained `"selected": "0.2.0"`; the installed bytes changed
      from the local 0.1.0 to the registry's 0.2.0.
- [x] Write the finding — with the observed JSON and the byte comparison — into
      §2's Verified properties, replacing the UNVERIFIED bullet.
- [x] Pick the §5 branch and record it in Corrections. → **defect branch.**
- [x] **Added task:** fix it. `is_registry_dependency` is now the single
      predicate used by BOTH `resolve()`'s seeding and plan-60-B's
      `registry_dependency_count`, requiring an `owner#pkg` ident **and** a
      non-`file://` source. One predicate, because the two callers silently
      diverging is exactly the class of bug this is.
- [x] **Added task:** keep the spike as the permanent regression test, and prove
      it can fail (restoring the ident-only filter makes it red).

Acceptance: §2 has no UNVERIFIED bullet, and §5's branch is chosen with the
observed evidence written down. If the defect branch is taken, this phase also
files the regression test named in §5. **VERIFIED** — §2's bullet is now
CONFIRMED with the observed resolver output, lock contents and byte comparison;
§5's defect branch is chosen and recorded in Corrections #1; the regression test
is `spike_file_added_package_with_registry_ident_survives_update` and is
A/B-verified in both directions. 3150 unit / 21 acceptance (was 20), 0 failed.
Commit: e627ce953

### Phase 2 — Flag parsing and the inference matrix

Pure logic, no registry, fully unit-testable.

- [x] Add an `AddOptions` (or equivalent) parser in `src/cli/pkg.rs` following
      `run_pkg_doc`'s shape (`:1177-1200`): one positional target, `--pin`,
      `--no-pin`, unknown-flag rejection, second-positional rejection.
- [x] Add a pure function implementing §4.1's matrix. **Signature simplified**
      (Corrections #4): `infer_pin(has_explicit_version: bool, pin_flag:
      Option<bool>) -> bool`, not the plan's `(has_explicit_version, pin_flag,
      no_pin_flag)`. Collapsing the two flags to `Option<bool>` at the parse
      boundary makes the mutually-exclusive `--pin --no-pin` state
      *unrepresentable* downstream, so the matrix function cannot be handed a
      contradiction and has no error case to return.
- [x] Change the `add` dispatch arm (`src/cli/pkg.rs:26`) from
      `[command, url]` to `[command, rest @ ..]`, matching `doc`'s arm at `:32`.
- [x] Tests in `src/cli/pkg.rs`: one case per row of §4.1's matrix, including
      the `--pin --no-pin` usage error **in both orders** (neither wins);
      `--no-pin` on a `file://` target is a usage error (§4.2) while `--pin` on
      one is accepted as a no-op; unknown flag rejected; two positionals
      rejected; flags accepted before the positional.
- [x] **Added task:** thread the inferred pin through
      `add_package_from_registry`, replacing the hardcoded `pin: true` at what
      was `:672`. (The plan lists this under Phase 3, but the signature change
      belongs with the parser that produces the value.)

Acceptance: `cargo test --bin mfb` passes with a test per matrix row. The
`--pin --no-pin` case asserts exit-2 usage, not a silent winner. **VERIFIED** —
3154 passed / 0 failed (from 3150); `parse_add_options_rejects_bad_argument_shapes`
asserts the `Usage` variant (exit 2) for both flag orderings.
Commit: 68afd94e2

### Phase 3 — Rewire both add paths onto `apply_manifest_change` (largest blast radius)

- [x] Rewrite `add_package_from_registry` (`src/cli/pkg.rs:614`) to build the
      proposed `project.json` text **first** — using `project_json_with_package`
      with the inferred `pin` — and hand it to `apply_manifest_change`. Delete
      the direct `fetch_blob` (`:640`) / `install_verified_package` (`:660`) /
      `install_vendor_blobs` (`:666`) calls; the pipeline's `install()` step
      performs them from the lock.
- [x] Keep `fetch_index` + `select_index_version` (`:635-639`) — they are still
      needed to determine the `version` value to write into `project.json`, which
      is the anchor `resolve()` will use. Note in a comment that this is the
      floor, not the selection.
- [x] Replace the hardcoded `pin: true` at `:672` with the inferred value.
- [x] Rewrite `add_package_from_file` (`:541`) to route its manifest write
      through `apply_manifest_change` so the lock stays current, per §5's chosen
      branch. Keep the vendor-library refusal at `:550-561` and the
      stage-then-rename copy at `:593-595` exactly as they are.
- [x] Update the success line (`:600`) to append `(floating)` / `(pinned)` /
      `(floating, floor <v>)` per §4.1.
- [x] Delete any `#[allow(dead_code)]` that plan-60-B added to
      `apply_manifest_change`.
- [x] Tests: extend `tests/repo_acceptance.rs` — after `mfb pkg add alice#pkg`,
      assert `mfb.lock` exists and contains the package, and that a subsequent
      `mfb pkg install` exits 0 **without** an intervening `mfb pkg update`
      (this is the hole being closed, and no current test covers it). Assert
      `"pin": false` in the written `project.json` for a bare add and
      `"pin": true` for an `@version` add.
- [x] Tests: the resolve-first atomicity test deferred from plan-60-B Phase 3 —
      `mfb pkg add alice#pkg@9.9.9` (unpublished) leaves `project.json` and
      `mfb.lock` byte-identical. Note that `tests/repo_acceptance.rs:1055`
      already exercises `pkg add alice#addable_pkg@9.9.9`; extend that case
      rather than adding a second one.
      **⚠ THIS TASK COMPLETES plan-60-B.** B's Phase 3 is `- [~]` and B is
      unarchived pending exactly this test plus the reorder-goes-red check in
      the acceptance line below (plan-60-B Corrections #5). It is the only proof
      that the resolve-first guarantee — B's entire reason for existing — holds
      end to end. When it lands: tick B's Phase 3, fill B's `Commit:` line, and
      archive B to `planning/old-plans/`.

- [x] **Added task** (Corrections #6): restructure
      `repo_resolver_reports_diamond_conflict_naming_both_requirers`, whose
      *setup* assumed `add` does not resolve. Its protected assertion (the
      conflict diagnostic names the symbol and the requirer) is unchanged.

Acceptance: `cargo test --test repo_acceptance` passes, including the
add-then-install-without-update case and the atomicity case. Verify the atomicity
test can fail: temporarily move the `project.json` write before the `resolve`
call in `apply_manifest_change`, confirm the test goes red, restore.
**VERIFIED — and this check earned its keep (Corrections #5): the test the plan
pointed at is VACUOUS for this mutation.** 21 acceptance / 3154 unit, 0 failed.
Commit: ddb4c8898

### Phase 4 — Docs

- [x] `src/main.rs:96` `PKG_HELP`: update the `add` line to
      `add <target> [--pin|--no-pin]` and add an Options entry for both flags
      alongside the existing `--proof` / `--out` entries (`:116-118`).
- [x] ~~`src/main.rs:45` `USAGE:53` — the short `pkg add <target>` line~~ —
      **left as-is, deliberately.** The task offers the choice; the top-level
      screen is a representative subset that already ends in
      `Run 'mfb pkg --help' for all package commands`, and two flags on a
      one-line summary would cost more readability than they buy. The flags are
      documented in `PKG_HELP`, which that pointer leads to.
- [x] `src/docs/spec/tooling/07_cli-reference.md:51` — update the `pkg add` row's
      argument column with the flags.
- [x] Document the pin-inference matrix and the "under `pin: false`, `version` is
      an ABI floor" semantics in the tooling spec. Added as a new
      `## \`pkg add\` Pin Inference` section carrying the full five-row matrix,
      the both-flags usage error, the ABI-floor explanation, and the `file://`
      rule — citing `[[src/cli/pkg.rs:infer_pin]]` and
      `[[src/cli/resolve.rs:select_node]]`.
- [x] **Added task** (Corrections #7): document resolve-first in the `pkg`
      subcommands prose, and correct two stale claims found there — one of them a
      **plan-60-A leftover** that no rename census could have caught.

Acceptance: `cargo build && cargo test --bin mfb spec` passes;
`mfb spec tooling --all` renders with no leaked `[[` markers; `mfb pkg --help`
shows both flags. **VERIFIED** — build exit 0; `cargo test --bin mfb spec` 48
passed; 0 leaked `[[` markers; `mfb pkg --help` shows `--pin` and `--no-pin`.
Full project acceptance (`scripts/test-accept.sh`) also green: 1069 tests, 0
mismatches — the `PKG_HELP` change caused no golden churn.
Commit: d13b5daee

## Validation Plan

- **Tests:** §4.1 matrix table (unit, `src/cli/pkg.rs`); flag rejection cases;
  add-then-install-without-update (acceptance); resolve-first atomicity
  (acceptance, extending `tests/repo_acceptance.rs:1055`); `file://` local-bytes
  regression if §5's defect branch is taken.
- **Coverage check:** `add_package_from_registry` sits under a `// coverage:off`
  block (`src/cli/pkg.rs:625-627`) because it reaches a live registry. The new
  *pure* pieces — flag parsing and the inference matrix — must be **outside** that
  block, or the matrix tests will not appear in the denominator and a green gate
  will prove nothing about them.
- **Runtime proof:** in a scratch project against the acceptance-harness
  registry: `mfb pkg add alice#shape` → inspect `project.json` for `"pin": false`
  → `mfb pkg install` → exit 0. Then `mfb pkg add alice#other@1.0.0` → inspect
  for `"pin": true`.
- **Doc sync:** `src/main.rs` (`USAGE`, `PKG_HELP`),
  `src/docs/spec/tooling/07_cli-reference.md`, plus the new pin-semantics section
  in the tooling spec.
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

- **Should a bare `add` warn when the newest eligible version is not ABI-compatible
  with anything?** Not applicable at add time — a fresh dependency has no
  requirement set to be incompatible with. Noted here only to record that it was
  considered and does not apply until plan-60-E, where an existing floor exists.
  (§4.1)

## Corrections

**#1 — §5's branch: the DEFECT branch, confirmed empirically.** (Phase 1 spike,
2026-07-21.) `mfb pkg update` silently replaced a `file://`-added local package
with a registry blob of a different version. Full evidence in §2's Verified
properties; the short form is that the spike added local `0.1.0` and `update`
installed registry `0.2.0` over it, recording `"selected": "0.2.0"` in the lock.

**The fix, and why it is one predicate rather than two.** §5 says to change the
seed filter to key on `source`. Done — but implemented as a single
`is_registry_dependency(dep)` used by **both** `resolve()`'s seeding *and*
plan-60-B's `registry_dependency_count`. Those two must agree exactly or
`apply_manifest_change` either calls `resolve()` on a set it will reject, or
takes the zero-dependency path while real dependencies still need locking. Two
copies of a filter that must never diverge is the same class of bug as the one
being fixed, so there is now one definition and a test
(`registry_dependency_count_matches_the_resolver_seeding_filter`) that fails if
a caller drifts from it.

**Both directions are A/B-verified**, not assumed: restoring the ident-only
filter makes the acceptance regression test red, and dropping the `#` check makes
the unit agreement test red.

**#2 — the spike's first draft measured the wrong baseline, and would have
"confirmed" the bug after it was fixed.** (Phase 1, 2026-07-21.) It captured the
local `.mfp` bytes from the *package build directory* before publishing, then
compared against the consumer's installed copy. But publishing **rebuilds
`spike_pkg.mfp` in place**, so the consumer's `pkg add file://…` copied the
freshly-built `0.2.0`, and the byte comparison failed for a reason that had
nothing to do with resolution. The fixed test takes its baseline from the
consumer's `packages/` directory immediately after `add` — i.e. it asserts the
real invariant, "`update` does not change what `add` installed". Worth recording
because the first version failed *both* before and after the fix, which is the
signature of a test measuring the wrong thing rather than a bug.

**#4 — the matrix function needs two inputs, not three.** (Phase 2, 2026-07-21.)
Phase 2 specifies `(has_explicit_version, pin_flag, no_pin_flag)` returning "the
resolved `pin` boolean **or a usage error**". Implemented instead as
`infer_pin(has_explicit_version: bool, pin_flag: Option<bool>) -> bool`. Two
separate booleans can encode `(true, true)` — the `--pin --no-pin` contradiction
— so the three-input version must carry an error case, and every caller must
handle an error that is really a parsing concern. Collapsing to `Option<bool>` at
the parse boundary (where the contradiction is actually detected and rejected)
makes the invalid state unrepresentable downstream: the matrix function becomes
total, and there is exactly one place that can produce the usage error. The
matrix's behavior is unchanged; all five rows are still tested.

**#5 — the atomicity test the plan specifies is VACUOUS, and the
reorder-goes-red check is what caught it.** (Phase 3, 2026-07-21.) Phase 3 says
to extend the existing `pkg add alice#addable_pkg@9.9.9` case rather than add a
second one, and plan-60-B Phase 3 requires proving it can fail by moving the
`project.json` write above the `resolve()` call. Doing exactly that: **the test
stayed green.**

The reason is that `@9.9.9` fails in `select_index_version`, inside
`add_package_from_registry`, *before* `apply_manifest_change` is ever called. So
it proves the pre-resolve validation path writes nothing — genuinely worth
having — but it cannot observe the ordering *inside* the pipeline, which is the
guarantee plan-60-B exists to provide. Had the mutation check been skipped, B
would have been archived with its central property unproven and a test that
looked like proof.

**What actually proves it:** a failure that occurs *inside* `resolve()`. The
diamond-conflict case is one, so
`repo_resolver_reports_diamond_conflict_naming_both_requirers` now also asserts
that a refused `add` leaves `project.json` untouched — and that assertion **does**
go red under the same mutation, with the message "a refused add must leave
project.json untouched". Both tests are kept, each labelled in-code with what it
does and does not prove.

**#6 — `add` now refuses a change that cannot resolve, which broke a test's
setup.** (Phase 3, 2026-07-21.) `repo_resolver_reports_diamond_conflict_…`
(from `9e5aae4ae`, plan-10-B2) built its conflicting state with two `pkg add`s
and then relaxed the pins, relying on `add` writing without resolving. With
resolve-first, the second add is correctly refused, so the setup could no longer
construct the state.

The assertion that broke was **setup**, not the protected behavior: the test
exists to prove the resolver's diagnostic names the disagreeing symbol and the
requirer package, and that is untouched. Restructured to declare the conflicting
dependency directly in `project.json`, keeping the `update` assertions verbatim,
and *added* coverage for the new, better behavior — the conflict now surfaces at
`add` time rather than being written to disk and discovered later. Net: strictly
more coverage, same protected property.

**#7 — a plan-60-A leftover that no rename census could have found.** (Phase 4,
2026-07-21.) The `## pkg and repo Subcommands` prose in
`src/docs/spec/tooling/07_cli-reference.md` described the publisher command as
**`publish <owner> <package>`** — inside the paragraph documenting
`run_pkg_command`, and therefore attributing it to `mfb pkg`. plan-60-A's census
searched for `pkg publish`, and this text never contains that string: the
subcommand names in that paragraph are written bare, with the parent implied by
the surrounding sentence. A grep for the *renamed* string cannot find a reference
that never spelled the parent out.

Two stale claims fixed together:

- `publish <owner> <package>` → moved into the `repo` paragraph as
  `repo publish <owner> [path]` (also picking up plan-60-A's optional-path
  change, which the old text predated).
- "records a **pinned** dependency" → now correct only for `file://` adds; a
  registry add defaults to floating as of this letter.

**Generalizable lesson, and the second time this bit plan-60** (see plan-60-A
Corrections #5, the line-broken `mfb pkg\\npublish`): a rename census that greps
for the old *full* command string will miss every reference that abbreviates or
wraps it. For the remaining letters, prefer reading the affected spec sections
end to end over trusting a grep count.

**#3 — a consequence of the fix, deliberately left to plan-60-E.** With the
filter corrected, `mfb pkg update` on a project whose *only* dependency is a
`file://` package now exits 1 with `"project.json declares no registry
dependencies to resolve"`, where it previously "succeeded" by corrupting the
package. That error is the pre-existing behavior for any project with no registry
dependencies, so the fix makes such projects **consistent** rather than newly
broken — but a clean no-op would be better, and plan-60-B §4.3 already defines
that policy. Not done here: plan-60-B Phase 2 explicitly forbids rewiring
`update()` ("letter E decides its final shape"), and widening C to redesign
`update` would undercut that. **Recorded as an input to plan-60-E.**

## Summary

The engineering risk is Phase 3: it inverts the order of network install and
manifest mutation on the most-used package command, with 16 existing test sites
depending on the current shape. Phase 1 exists because a single unread filter
(`src/cli/resolve.rs:176`, keyed on ident rather than source) determines whether
`file://` adds share that path — and possibly whether `mfb pkg update` has been
silently replacing local packages with registry blobs all along.

Untouched: transitive resolution (still absent), the `.mfp` format, the trust
chain, and `mfb pkg update`'s bare form, which letter E owns.
