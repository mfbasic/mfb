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
| plan-60-B Phase 3 outstanding — **C must complete it** | see plan-60-B Corrections #5; the resolve-first atomicity test is deferred into C's Phase 3 below | **OUTSTANDING — an obligation of this letter, not a blocker on starting it** |

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
- **UNVERIFIED and load-bearing: whether a `file://`-added package whose `.mfp`
  ident contains `#` is treated as a registry dependency by `resolve()`.**
  Reading `src/cli/resolve.rs:176` shows the seed filter is
  `.filter(|dep| dep.ident.contains('#'))` — it keys on **ident**, not on
  `source`. A signed package added by `file://` carries an `owner#package` ident
  in its header, so on this reading `resolve()` would fetch its `/index` and
  `install()` would then overwrite the local copy with a registry blob. If true
  this is a pre-existing defect in `mfb pkg update`, not something this letter
  introduces — but this letter routes `add` through the same path, so it would
  become reachable from `add` too. **Phase 1 settles this empirically before any
  code changes.**

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

- [ ] In `tests/repo_acceptance.rs`, publish a package as `alice#spike_pkg`, then
      in a fresh consumer project run `mfb pkg add file://<path to the built
      .mfp>` and inspect the written `project.json` — record the exact `ident`
      and `source` values.
- [ ] Run `mfb pkg update` in that consumer and observe whether `mfb.lock` gains
      a `spike_pkg` entry and whether `packages/spike_pkg.mfp` bytes change.
- [ ] Write the finding — with the observed JSON and the byte comparison — into
      §2's Verified properties, replacing the UNVERIFIED bullet.
- [ ] Pick the §5 branch and record it in Corrections.

Acceptance: §2 has no UNVERIFIED bullet, and §5's branch is chosen with the
observed evidence written down. If the defect branch is taken, this phase also
files the regression test named in §5.
Commit: —

### Phase 2 — Flag parsing and the inference matrix

Pure logic, no registry, fully unit-testable.

- [ ] Add an `AddOptions` (or equivalent) parser in `src/cli/pkg.rs` following
      `run_pkg_doc`'s shape (`:1177-1200`): one positional target, `--pin`,
      `--no-pin`, unknown-flag rejection, second-positional rejection.
- [ ] Add a pure function implementing §4.1's matrix — input: `(has_explicit_version,
      pin_flag, no_pin_flag)`; output: the resolved `pin` boolean or a usage
      error. Keep it separate from any I/O so it is directly testable.
- [ ] Change the `add` dispatch arm (`src/cli/pkg.rs:26`) from
      `[command, url]` to `[command, rest @ ..]`, matching `doc`'s arm at `:32`.
- [ ] Tests in `src/cli/pkg.rs`: one case per row of §4.1's matrix, including
      the `--pin --no-pin` usage error; `--no-pin` on a `file://` target is a
      usage error (§4.2); unknown flag rejected; two positionals rejected.

Acceptance: `cargo test --bin mfb` passes with a test per matrix row. The
`--pin --no-pin` case asserts exit-2 usage, not a silent winner.
Commit: —

### Phase 3 — Rewire both add paths onto `apply_manifest_change` (largest blast radius)

- [ ] Rewrite `add_package_from_registry` (`src/cli/pkg.rs:614`) to build the
      proposed `project.json` text **first** — using `project_json_with_package`
      with the inferred `pin` — and hand it to `apply_manifest_change`. Delete
      the direct `fetch_blob` (`:640`) / `install_verified_package` (`:660`) /
      `install_vendor_blobs` (`:666`) calls; the pipeline's `install()` step
      performs them from the lock.
- [ ] Keep `fetch_index` + `select_index_version` (`:635-639`) — they are still
      needed to determine the `version` value to write into `project.json`, which
      is the anchor `resolve()` will use. Note in a comment that this is the
      floor, not the selection.
- [ ] Replace the hardcoded `pin: true` at `:672` with the inferred value.
- [ ] Rewrite `add_package_from_file` (`:541`) to route its manifest write
      through `apply_manifest_change` so the lock stays current, per §5's chosen
      branch. Keep the vendor-library refusal at `:550-561` and the
      stage-then-rename copy at `:593-595` exactly as they are.
- [ ] Update the success line (`:600`) to append `(floating)` / `(pinned)` /
      `(floating, floor <v>)` per §4.1.
- [ ] Delete any `#[allow(dead_code)]` that plan-60-B added to
      `apply_manifest_change`.
- [ ] Tests: extend `tests/repo_acceptance.rs` — after `mfb pkg add alice#pkg`,
      assert `mfb.lock` exists and contains the package, and that a subsequent
      `mfb pkg install` exits 0 **without** an intervening `mfb pkg update`
      (this is the hole being closed, and no current test covers it). Assert
      `"pin": false` in the written `project.json` for a bare add and
      `"pin": true` for an `@version` add.
- [ ] Tests: the resolve-first atomicity test deferred from plan-60-B Phase 3 —
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

Acceptance: `cargo test --test repo_acceptance` passes, including the
add-then-install-without-update case and the atomicity case. Verify the atomicity
test can fail: temporarily move the `project.json` write before the `resolve`
call in `apply_manifest_change`, confirm the test goes red, restore.
Commit: —

### Phase 4 — Docs

- [ ] `src/main.rs:96` `PKG_HELP`: update the `add` line to
      `add <target> [--pin|--no-pin]` and add an Options entry for both flags
      alongside the existing `--proof` / `--out` entries (`:116-118`).
- [ ] `src/main.rs:45` `USAGE:53` — the short `pkg add <target>` line; add the
      flags or leave the pointer to `mfb pkg --help`, whichever keeps the block
      readable.
- [ ] `src/docs/spec/tooling/07_cli-reference.md:51` — update the `pkg add` row's
      argument column with the flags.
- [ ] Document the pin-inference matrix and the "under `pin: false`, `version` is
      an ABI floor" semantics in the tooling spec. This is a new observable
      contract and `.ai/specifications.md` makes it a same-change obligation.
      Cite `[[src/cli/resolve.rs:select_node]]` for the pin-vs-float branch.

Acceptance: `cargo build && cargo test --bin mfb spec` passes;
`mfb spec tooling --all` renders with no leaked `[[` markers; `mfb pkg --help`
shows both flags.
Commit: —

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

<!-- Filled in DURING execution. Phase 1's finding and the §5 branch chosen go
     here first. -->

## Summary

The engineering risk is Phase 3: it inverts the order of network install and
manifest mutation on the most-used package command, with 16 existing test sites
depending on the current shape. Phase 1 exists because a single unread filter
(`src/cli/resolve.rs:176`, keyed on ident rather than source) determines whether
`file://` adds share that path — and possibly whether `mfb pkg update` has been
silently replacing local packages with registry blobs all along.

Untouched: transitive resolution (still absent), the `.mfp` format, the trust
chain, and `mfb pkg update`'s bare form, which letter E owns.
