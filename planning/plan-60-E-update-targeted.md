# plan-60-E: `mfb pkg update [<target>]` — targeted version changes

Last updated: 2026-07-21
Effort: medium (1h–2h)
Depends on: plan-60-C
Produces: the targeted-update command. Not consumed by any later letter — D, E
and F are independent siblings.

There is currently no command that changes a dependency's version. `mfb pkg add`
refuses an already-declared package (`src/manifest/package.rs:564-569`), and bare
`mfb pkg update` re-resolves whatever `project.json` already says. Bumping a
version means hand-editing JSON and then running `update` — a workflow with no
CLI affordance at all. This letter gives `update` a targeted form.

**Behavioral outcome:** `mfb pkg update alice#shape@1.4.0` sets that dependency's
version to 1.4.0, re-resolves, rewrites `mfb.lock` and installs — leaving `pin`
exactly as it was. `mfb pkg update alice#shape` with no version reports
`alice#shape is already on the latest version (1.4.0)` and changes nothing, or
raises it to the newest ABI-compatible eligible release. Bare `mfb pkg update` is
unchanged in meaning: re-resolve everything declared.

References:

- plan-60-B §4.2 — `apply_manifest_change`; §4.1 — `confirm`
- plan-60-C §4.1 — the pin-inference matrix that this letter deliberately does
  **not** reuse
- `src/cli/resolve.rs:332` — `select_node`, where the pin branch bypasses the ABI
  check
- `src/docs/spec/package-manager/01_repository-protocol.md:832-835` — eligibility

## Prerequisites

See plan-60-A for the plan-wide prerequisite gate. In addition:

| Must be true | Command | Status |
|---|---|---|
| plan-60-B complete — `confirm` and `apply_manifest_change` exist | `grep -cE '^pub\(crate\) fn confirm' src/cli/mod.rs` → 1 and `grep -cE '^pub\(crate\) fn apply_manifest_change' src/cli/resolve.rs` → 1 | **MET** (2026-07-21) — → 1 and 1. plan-60-B archived. |
| plan-60-C complete — flag parsing and CLI-creatable `pin: false` | `grep -cE 'no_pin' src/cli/pkg.rs` → ≥ 1 **and** `mfb pkg add --no-pin <ident>` parses (a bare grep also matches a test name, so confirm the flag is dispatched, not merely mentioned) | **MET** (2026-07-21) — → 5, and `mfb pkg add ada#x --pin --no-pin` exits 2 with the both-flags error, proving dispatch. plan-60-C archived. |

If either is incomplete, this plan cannot start, full stop.

### Input from plan-60-C (added 2026-07-21) — `update` on a registry-free project

plan-60-C fixed a data-loss defect: `resolve()` seeded registry nodes by **ident**
alone, so a package that was published and then added by `file://` (which copies
its ident out of the `.mfp` header) was resolved against the registry, and
`mfb pkg update` silently overwrote the user's local copy with a registry blob of
a different version. The seed filter now also requires a non-`file://` `source`.

**DISCHARGED 2026-07-21** — implemented in Phase 1's commit (`d05d02dff`):
`update()` now applies plan-60-B §4.3's policy directly (drop a stale lock,
report, exit 0) instead of calling `resolve()` on an empty set. Asserted by
plan-60-C's `spike_file_added_package_with_registry_ident_survives_update`,
extended here exactly as this note suggested.

**The consequence this letter owned:** `mfb pkg update` on a project whose only
dependencies are `file://` packages exited **1** with `"project.json declares
no registry dependencies to resolve"`. That is the pre-existing behavior for any
project with no registry dependencies — the fix made such projects consistent
rather than newly broken — but it is still the wrong answer. plan-60-B §4.3
already defines the right one: a project with no registry dependencies has
nothing to lock, which is a clean no-op, not a failure.

This was deliberately **not** fixed in C, because plan-60-B Phase 2 forbids
rewiring `update()` and assigns its final shape to this letter. When deciding
that shape, make `update` on a registry-free project succeed as a no-op
(and drop a stale `mfb.lock`, per §4.3) rather than error. Add a test covering a
`file://`-only project; plan-60-C's
`spike_file_added_package_with_registry_ident_survives_update` is a ready-made
fixture to extend.

> **Corrected 2026-07-21 (plan-60-C Corrections).** These rows originally used
> unanchored greps (`grep -c 'fn confirm'`, `grep -c 'fn apply_manifest_change'`)
> that count **every** mention, not the definition — and plan-60-B names its tests
> after the functions they test, so they return 3 and 4 rather than 1. Anchored on
> `^pub(crate) fn`. This was the recurring defect of plan-60's gate checks
> (plan-60-B Corrections #1, plan-60-C Prerequisites): a grep for a spelling
> cannot tell a definition from a test asserting something about it. Check the
> construct.


> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue.

## 1. Goal

- `mfb pkg update <ident>@<version>` sets that dependency's declared version,
  re-resolves, and installs.
- `mfb pkg update <ident>` raises it to the newest ABI-compatible eligible
  version, or reports that it is already current.
- A target not declared in `project.json` is an error.
- A `pin: true` target requires confirmation; a `pin: false` one does not.
- `pin` state is **preserved**, never inferred from the presence of `@version`.
- Bare `mfb pkg update` keeps its current meaning and its byte-identical
  re-resolve property.

### Non-goals (explicit constraints)

- **No pin inference.** This is the explicit difference from `add` — see §4.1.
- **No `[location]` argument.** Dropped; see §4.4 and Compatibility.
- **No change to `resolve()`'s selection algorithm.** The ABI check in §4.3 is a
  *pre-flight advisory* computed from the index; it does not alter what
  `select_node` does.
- **No transitive updates.** Updating A does not update A's dependencies beyond
  whatever re-resolution of the declared set already does.
- **No change to `mfb.lock`'s format.**

## 2. Current State

`update` dispatches at `src/cli/pkg.rs:51` (bare) and `:54` (with a `[location]`
path), both calling `resolve::update(project_dir)` (`src/cli/resolve.rs:72`):

```
read_manifest → read_lock → resolve(&manifest) → print_lock_diff
              → write_lock → install(project_dir)
```

`select_node` (`src/cli/resolve.rs:332`) has two branches. The pin branch
(`:355-381`) finds the exact declared version in the index, rejects `blocked` and
`legal-tombstoned`, and returns — **without calling `is_superset`**. The floating
branch (`:383-400`) filters by `state_is_floating_eligible` **and** by
`is_superset(candidate_abi, required)`, then takes the highest by
`compare_versions` (`:481`).

`compare_versions` and `is_superset` are private to `src/cli/resolve.rs`
(`:481`, `:415`).

### Measured populations

| What | Count | Command |
|---|---|---|
| `pkg update` arg-vector sites in `tests/` | 4 | `grep -rn '"pkg", "update"' tests/ \| wc -l` → 4 |
| …of those, sites passing a `[location]` path | 0 | `grep -rn '"pkg", "update"' -A 2 tests/` — all four are the bare form (`repo_acceptance.rs:1240`, `:1254`, `:1277`, `:1685`) |
| `update` dispatch arms to replace | 3 | `src/cli/pkg.rs:51`, `:54`, `:57` |
| CLI-reference spec rows for `pkg update` | 1 | `src/docs/spec/tooling/07_cli-reference.md:52` |

### Verified properties

- **Dropping `[location]` breaks no existing test.** All four `pkg update` call
  sites in `tests/` use the bare form — verified by reading the two lines
  following each (`repo_acceptance.rs:1240`, `:1254`, `:1277`, `:1685`), not by
  counting matches.
- **A pinned selection bypasses the ABI check entirely.** Read `select_node`
  (`src/cli/resolve.rs:355-381`): the pin branch returns before reaching the
  `is_superset` filter at `:394`. So a targeted bump of a `pin: true` dependency
  to a version that dropped a symbol the project imports resolves cleanly and
  fails later at build time. This is the hazard §4.3 exists to address.
- **Bare `update` must stay byte-identical on re-resolve.**
  `tests/repo_acceptance.rs:1254-1256` asserts that running `pkg update` twice
  produces identical `mfb.lock` bytes. Read the assertion. Any change to
  `update`'s bare path must preserve this.
- **`compare_versions` handles pre-release suffixes.** Read
  `src/cli/resolve.rs:481` and its test at `:716-720`: `1.0.1 > 1.0.0`,
  `1.2.0 < 1.10.0`, and a release outranks the same release with a pre-release
  suffix. §4.2's "is there a newer version" comparison can rely on it.

## 3. Design Overview

Four pieces:

1. **Argument parsing** — bare vs. targeted, and the `--yes` / `--pin` /
   `--no-pin` flags (§4.4).
2. **The pin-preservation rule** (§4.1) — the one place this letter deliberately
   diverges from `add`.
3. **Version selection for the no-`@version` form** (§4.2, §4.3), including the
   ABI advisory that the pin branch would otherwise skip.
4. **The confirmation gate** for pinned targets (§4.5).

**Design uncertainty: concentrated in §4.3**, the ABI advisory. It re-implements
a compatibility judgement that `select_node` makes internally, using the index's
`abi_map()` rather than the resolver's requirement union — so it is an
approximation of the real check. §4.3 states precisely what it can and cannot
prove, and Phase 2 verifies the approximation on a real breaking change.

**Correctness risk: concentrated in the bare-`update` path staying untouched.**
Three of the four existing tests cover it, including a byte-identical assertion.
Refactoring the dispatch must not perturb it.

**Rejected alternative: infer `pin` from `@version`, as `add` does.** Rejected.
`add` infers because there is no prior intent to respect; `update` operates on a
dependency that already carries a declared `pin`, and flipping it as a side
effect of a version bump changes a property the user set deliberately. The rule
is: **`add` infers, `update` preserves.**

**Rejected alternative: make the targeted form a separate `mfb pkg bump`
command.** Rejected — two verbs for "change what version I depend on" is the
overloading this plan set out to remove.

## 4. Detailed Design

### 4.1 Pin preservation

The targeted form never changes `pin` unless `--pin` or `--no-pin` is passed
explicitly. `mfb pkg update alice#shape@1.4.0` on a `pin: false` dependency
leaves it floating with a new floor of 1.4.0.

`--pin` and `--no-pin` together are a usage error, matching plan-60-C §4.1.

### 4.2 Selecting a version for the no-`@version` form

1. `fetch_index` for the target's ident.
2. Filter to `state_is_floating_eligible` (`src/cli/pkg.rs:786`) — this excludes
   `yanked`, which the spec makes selectable only by exact pin
   (`01_repository-protocol.md:833-834`). A bare `update` must never move a
   dependency *onto* a yanked release.
3. Compare each candidate against the currently declared version with
   `compare_versions`. If none is greater, print
   `<ident> is already on the latest eligible version (<v>)` and exit 0 having
   changed nothing — no manifest write, no resolve, no install.
4. Otherwise apply §4.3's ABI filter and take the highest survivor.

### 4.3 The ABI advisory

**The problem:** for a `pin: true` target, `select_node` will take whatever exact
version we write into `project.json` without any ABI check (§2, verified). So if
step 4 above simply took the newest eligible release, a targeted update could
move a pinned dependency onto a version that dropped a symbol the project uses —
resolving cleanly and failing at build time.

**The mechanism:** the registry index carries `abiIndex` per version
(`01_repository-protocol.md:593`), which is why `resolve()` can filter candidates
without downloading blobs. Compute the currently declared version's `abi_map()`
as the baseline and keep only candidates whose `abi_map()` is a superset of it.

**What this proves and does not prove.** It proves the candidate still exports
everything the *currently declared version* exported. It does **not** prove the
candidate satisfies the union of every requirer's needs — that union is built by
`resolve()` from sibling packages' import tables (`src/cli/resolve.rs:247-262`)
and is not available before resolution. So this is a *pre-flight advisory*: it
prevents the obvious breakage, and `resolve()` remains the authority. When
resolution afterwards fails or selects differently, resolution wins.

**When the newest eligible version fails the filter**, do not silently select an
older one. Print which version was skipped and why:

```
alice#shape 2.0.0 is available but drops symbols the currently declared 1.4.0
exports (foo, bar); selecting 1.6.0 instead. Use `@2.0.0` to take it anyway.
```

This keeps the escape hatch explicit — an exact `@version` is always honored,
because the user has then named the version themselves.

To reach `is_superset` (`src/cli/resolve.rs:415`) and `compare_versions` (`:481`)
from `pkg.rs`, make both `pub(crate)`. Both are already unit-tested in place
(`:716`, `:772`); no behavior change.

### 4.4 Command shape

```
mfb pkg update                                    re-resolve everything declared
mfb pkg update <owner>#<pkg>                      raise to newest compatible
mfb pkg update <owner>#<pkg>@<version>            set exactly
        [--pin | --no-pin] [--yes]
```

**Cwd-only.** The `[location]` form is dropped: a positional argument now means
an ident, and `mfb pkg update foo` cannot mean both a path and a package. Zero
existing tests pass a path (§2, verified).

A target that is not declared in `project.json` errors:
`` `alice#shape` is not declared in project.json; use `mfb pkg add alice#shape` ``.
This is the user-specified step 1 and is checked **before** any network access.

### 4.5 The confirmation gate

For a `pin: true` target, print what will change and call
`confirm(question, assume_yes)`:

```
alice#shape is pinned to 1.3.0. Updating it to 1.4.0 will change the pin.
Continue? [y/N]:
```

Declining exits 0 having changed nothing — the user answered the question that
was asked, which is not a failure.

`--yes` bypasses the prompt. On a non-TTY without `--yes`, `confirm` errors per
plan-60-B §4.1 rather than hanging or guessing.

A `pin: false` target is not prompted.

### 4.6 Applying the change

Build the proposed `project.json` text by rewriting the target dependency's
`version` (and `pin`, if a flag was given) in place, then hand it to
`apply_manifest_change` (plan-60-B §4.2), which resolves before writing anything.

`src/manifest/package.rs` has no in-place version rewriter — `project_json_with_updated_ident_key`
(`:644`) rewrites `identKey` and is the closest precedent. Add a sibling
`project_json_with_updated_version(contents, name, version, pin) -> Result<String, String>`
following the same surgical-string-edit approach, so formatting and comments in
`project.json` survive the edit.

## Compatibility / Format Impact

**Changes:**

- `mfb pkg update <path>` — **removed.** Previously accepted (`src/cli/pkg.rs:54`).
  A positional is now an ident. `mfb pkg update ./some/dir` becomes an error
  reporting that the target is not declared.
- `mfb pkg update <ident>[@version]` — new.
- New flags `--pin`, `--no-pin`, `--yes`.
- New interactive prompt for pinned targets.

**Explicitly unchanged:** bare `mfb pkg update`, including its byte-identical
re-resolve property; `mfb.lock` format; `project.json` schema; the selection
algorithm; exit codes for the bare form.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work it describes. An unticked box means NOT DONE.

### Phase 1 — Dispatch, argument parsing, and the not-declared check

Everything that can be tested without a registry.

- [x] Replace the three `update` arms (`src/cli/pkg.rs:51`, `:54`, `:57`) with a
      single `[command, rest @ ..]` arm feeding a parser built on plan-60-C's
      flag-parsing struct: zero or one positional, plus `--pin` / `--no-pin` /
      `--yes`.
- [x] Bare form (no positional) → `resolve::update(Path::new("."))`, unchanged.
- [x] Add `project_json_with_updated_version` to `src/manifest/package.rs`,
      modelled on `project_json_with_updated_ident_key` (`:644`).
- [x] Implement §4.4's not-declared check against the parsed manifest, before any
      network call.
- [x] Tests in `src/cli/pkg.rs`: bare form still dispatches; `--pin --no-pin` is
      a usage error; an undeclared target errors with the `mfb pkg add` hint; a
      path-looking positional (`./foo`) produces the not-declared error rather
      than being treated as a location.
- [x] Tests in `src/manifest/package.rs`: `project_json_with_updated_version`
      rewrites the version, optionally rewrites `pin`, leaves other dependencies
      and formatting untouched, and errors when the named package is absent —
      mirroring the existing `project_json_with_updated_ident_key` tests at
      `:1354-1379`.

Acceptance: `cargo test --bin mfb` passes; the four existing bare-`update`
acceptance tests still pass unchanged (`cargo test --test repo_acceptance`).
**VERIFIED** — 3170 unit (from 3158) / 24 acceptance, 0 failed. The bare-form
tests were not edited.
Commit: d05d02dff

### Phase 2 — Version selection and the ABI advisory

- [x] Make `is_superset` (`src/cli/resolve.rs:415`) and `compare_versions`
      (`:481`) `pub(crate)`.
- [x] Implement §4.2's selection: fetch index, filter eligible, compare against
      the declared version, report already-latest and exit 0 when nothing is
      newer.
- [x] Implement §4.3's ABI filter and its skipped-version message.
- [x] Tests in `src/cli/pkg.rs`: a pure selection function taking
      `(declared_version, Vec<IndexVersion>)` and returning the choice or the
      already-latest verdict — covering: nothing newer; a newer eligible
      compatible version; a newer version that is `yanked` (must be skipped); a
      newest version that fails the ABI filter with an older compatible one
      behind it (must select the older **and** report the skip); and no
      compatible candidate at all.
- [x] Tests in `tests/repo_acceptance.rs`: publish two versions of a package
      where the newer one **drops an exported symbol**, then run
      `mfb pkg update <ident>` and assert the advisory fires and the older
      version is selected. This is what verifies §4.3's approximation against a
      real breaking change rather than a synthetic ABI map.

Acceptance: `cargo test --bin mfb && cargo test --test repo_acceptance` pass,
including the dropped-symbol case. That case must fail if the ABI filter is
removed — verify by temporarily disabling the filter and confirming red.
**VERIFIED, and A/B-checked**: replacing the `is_superset` predicate with `true`
makes `update_targeted_applies_the_abi_advisory_and_preserves_pin` fail on "the
advisory must name the skipped version". The acceptance case uses a **real**
dropped `EXPORT FUNC`, not a synthetic ABI map.
Commit: —

### Phase 3 — The confirmation gate and applying the change

- [x] Implement §4.5: for a `pin: true` target, print the consequence and call
      `confirm`; declining exits 0 with no change.
- [x] Implement §4.6: build the proposed manifest text and call
      `apply_manifest_change`.
- [x] Print a result line naming old version → new version and the resulting pin
      state, consistent with plan-60-C's `(floating)` / `(pinned)` suffixes.
- [x] Tests in `tests/repo_acceptance.rs`: `update <ident>@<ver> --yes` on a
      pinned dependency changes the version, rewrites `mfb.lock`, installs the
      new bytes, and leaves `"pin": true`; the same on a floating dependency
      needs no `--yes` and leaves `"pin": false`; `update <ident>@9.9.9`
      (unpublished) leaves `project.json` and `mfb.lock` byte-identical.

Acceptance: `cargo test --test repo_acceptance` passes. The pin-preservation
assertion must check the literal `"pin"` value in the written `project.json`, not
merely that the command succeeded. **VERIFIED** — every pin assertion reads the
literal `"pin": true` / `"pin": false` text back out of the written manifest.
Commit: —

### Phase 4 — Docs

- [x] `src/main.rs:96` `PKG_HELP`: replace the `update [path]` line with the
      three forms from §4.4 and add `--yes` to the Options block.
- [x] `src/main.rs:45` `USAGE:54` — the short `pkg update` line.
- [x] `src/docs/spec/tooling/07_cli-reference.md:52` — rewrite the `pkg update`
      row for the new argument shape. Note the removal of `[location]`.
- [x] Document in the tooling spec: pin preservation (§4.1), the eligibility
      filter (§4.2), and the ABI advisory including precisely what it does and
      does not prove (§4.3). Cite `[[src/cli/resolve.rs:select_node]]` and
      `[[src/cli/pkg.rs:state_is_floating_eligible]]`.

Acceptance: `cargo build && cargo test --bin mfb spec` pass; `mfb spec tooling
--all` renders with no leaked `[[` markers. **VERIFIED** — build exit 0, 48 spec
tests, 0 leaked markers.
Commit: —

## Validation Plan

- **Tests:** pure selection-function table (§4.2/§4.3 cases);
  `project_json_with_updated_version` rewriting; not-declared and flag-conflict
  usage errors; acceptance cases for pinned-with-`--yes`, floating-without,
  unpublished-version atomicity, and the dropped-symbol ABI advisory.
- **Coverage check:** the selection and ABI-advisory logic must be pure functions
  **outside** any `// coverage:off` region — the surrounding command reaches the
  registry and is excluded, so logic written inline would look tested while being
  invisible to the suite. The `confirm` call site is TTY-gated and unreachable
  from tests; that is why §4.5's decision logic lives in the caller and only the
  prompt itself is inside `confirm`.
- **Runtime proof:** against the acceptance-harness registry — publish 1.0.0 and
  1.1.0 of a package, add it floating, run `mfb pkg update <ident>`, confirm the
  manifest floor moves to 1.1.0 and `packages/` holds 1.1.0's bytes. Then run it
  again and confirm the already-latest message with no file changes.
- **Doc sync:** `src/main.rs` (`USAGE`, `PKG_HELP`),
  `src/docs/spec/tooling/07_cli-reference.md`, tooling spec pin/eligibility
  sections.
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

- **Should the no-`@version` form on a `pin: false` dependency raise the declared
  floor, or leave `project.json` untouched and merely re-resolve?**
  Recommended: **raise the floor.** A floating dependency already re-floats on
  every bare `update`, so leaving the manifest untouched makes
  `mfb pkg update <ident>` report success while changing nothing and permits the
  next resolve to float back down. Raising the floor makes the command mean what
  its output says. The cost is that a floating dependency's declared version
  creeps upward over time — which is the correct record of "the oldest ABI I have
  actually verified against". (§4.2)

## Corrections

<!-- Filled in DURING execution. -->

## Summary

The engineering risk is §4.3's ABI advisory: it approximates, from the registry
index alone, a compatibility judgement that `resolve()` makes from the full
import graph — and it exists only because the pin branch of `select_node` skips
the real check (§2, verified). Phase 2's dropped-symbol acceptance test is what
keeps the approximation honest.

Untouched: bare `mfb pkg update` and its byte-identical re-resolve guarantee, the
selection algorithm, the lockfile format, and transitive resolution.
