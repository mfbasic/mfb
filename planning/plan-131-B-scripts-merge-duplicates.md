# plan-131-B: scripts/ cleanup — merge the duplicates

Last updated: 2026-09-12
Effort: large (3h–1d)
Depends on: plan-131-A

Five groups of scripts do one job between them. This sub-plan folds each group into one script.
The old scripts are deleted only after a differential run shows the new one produces the same
result on the same input. Prerequisites, the full inventory, and the Open Decisions are in
`plan-131-A-scripts-fix-and-delete.md` §3.

References:

- `plan-131-A-scripts-fix-and-delete.md` — prerequisites gate, inventory, `scripts/` vs `tools/` rule.
- `tests/gate/gate_lock_covers_every_writer.rs` — `CLASSIFICATION`, `SCAN_FLOOR`.
- `.ai/testing-gates.md`, `.ai/build-tooling.md` — cite the regen and baseline scripts.
- `planning/tests.md` — the open coverage-work doc that cites the `coverage-src-*` commands.
- Memory `regen-ncodesum-hashes-stale-dump-on-failed-build` — the rm-then-check-exit rule the
  merged regen script must keep.

## Prerequisites

See plan-131-A. In addition:

| Must be true | Command | Status |
|---|---|---|
| plan-131-A complete | `ls planning/plan-131-A-* 2>/dev/null` → no match (archived) | NOT MET |
| Goldens match HEAD before any regen differential | `bash scripts/artifact-gate.sh target/release/mfb all` → exit 0 | UNMEASURED |

## 1. Goal

- `regen-native-goldens.sh` replaces `regen-ncodesum.sh`, `regen-outside-ncode.sh` and
  `regen-rt-goldens.sh`.
- `artifact-baseline.sh` replaces `linux-artifact-baseline.sh` and `exe-oracle.sh`.
- `coverage-report.py <gaps|lines|shapes|dead|delta>` replaces the five `coverage-src-*.py`.
- `coverage.sh --bins` replaces `coverage-bins.sh`.
- `check-tls-loopback.sh --remote <port>` replaces `check-tls-loopback-remote.sh`.
- For each group, a recorded differential run shows identical output from old and new.

### Non-goals

- No golden content changes. A regen differential that rewrites a golden is a bug in the merge,
  not a re-baseline.
- `coverage-check.sh`, `coverage-common.sh` and `coverage.sh`'s default (CI) mode keep their
  current behavior. CI lines 303–308 of `.github/workflows/coverage.yml` do not change.
- `test-accept.sh` and `artifact-gate.sh` are not touched.

## 2. Current State (from the plan-131 audit, 2026-09-12)

**Regen trio.**
- `regen-ncodesum.sh` rewrites every `tests/**/golden/*.ncodesum`.
- `regen-outside-ncode.sh`'s header premise (that regen-ncodesum skips outside fixtures) has been
  false since plan-118-C. Its only unique job is the raw `.ncode` goldens outside byte-identity.
- `regen-rt-goldens.sh` is the only writer for raw `.nir/.nplan/.nobj/.mir` goldens. It
  hardcodes `" nir nplan nobj ncode mir "` instead of using `$ARTIFACT_NATIVE_KINDS` from
  `artifact-kinds.sh`, which contradicts the census comment claiming it takes its kinds from there.
- **Bug:** `regen-ncodesum.sh` and `regen-outside-ncode.sh` default `HOST=${2:-macos-aarch64}`
  (`grep -n HOST scripts/regen-*.sh`). On a Linux host they write the Linux dump's hash into the
  macOS sums. The other scripts derive the host from `uname`.
- Golden filename parsing (`<pkg>.<target>[.app].<ext>[sum]`) is implemented four times: in
  artifact-gate, regen-ncodesum, regen-outside-ncode and regen-rt-goldens.
- Populations to measure in Phase 1: UNMEASURED.
  - `.ncodesum` goldens: `find tests -path '*/golden/*.ncodesum' | wc -l`.
  - Raw native goldens: `find tests -path '*/golden/*' \( -name '*.ncode' -o -name '*.nir' -o -name '*.nplan' -o -name '*.nobj' -o -name '*.mir' \) | wc -l`.

**Baseline pair.**
- `linux-artifact-baseline.sh` does capture/verify of a sha manifest of every dump plus the linked
  `.out` for the three Linux targets. It builds in scratch copies, so it is exempt in the census.
- `exe-oracle.sh` does record/compare of the sha of every linked `.out` for one target. It builds
  in-tree (`tests/<fx>/build`, deletes root `.mfp`) **with no gate lock**, and the census cannot
  see it because it passes no dump flag.
- Its only live caller was `bug387-gate.sh`, deleted in A.

**Coverage.**
- Four of the five `coverage-src-*.py` re-implement the same pieces:
  - the cwd-relative exceptions-file read;
  - `data["data"][0]` loading;
  - the `src/**` filter that skips `repository/src/**`;
  - `FLOOR` read from the environment.
- `coverage-bins.sh` differs from `coverage.sh` only in the cargo line and which reports it emits.
- No CI step, Rust test or `.ai` doc calls either. Their only live citations are in
  `planning/tests.md` and the scripts' own docstrings
  (`git grep -l coverage-src- -- ':!planning/completed'`).

**TLS.**
- `check-tls-loopback-remote.sh` has zero references
  (`git grep -l check-tls-loopback-remote` → itself only).
- Its MFBASIC server and client sources repeat legs 3–4 of `check-tls-loopback.sh` word for word.

## 3. Design

- **`regen-native-goldens.sh <mfb> [fixture-dir…]`.**
  - With no dirs, it sweeps `tests/`.
  - It handles every kind in `$ARTIFACT_NATIVE_KINDS`, raw or `sum`, and reads the target and
    `--app` from the golden filename.
  - The host comes from `uname`, copied from artifact-gate's derivation.
  - It takes the gate lock.
  - For each golden it removes the stale dump, builds, and **checks the build exit before hashing**.
  - It never creates a golden, only rewrites existing ones, the same as `sync-goldens.sh`.
- **`artifact-baseline.sh <mfb> capture|verify <manifest> [--targets t1,t2…]`.**
  - The default targets are the three Linux ones, as today.
  - `--targets macos-aarch64` covers what `exe-oracle.sh` was for.
  - It keeps building in scratch, so it stays exempt from the lock.
- **`coverage-report.py`.**
  - One loader: the exceptions path is resolved from the script's location, not cwd.
  - Five subcommands, each a straight port of one script's body.
- **`coverage.sh --bins`** runs the `coverage-bins.sh` cargo line and its JSON export instead of
  the workspace run.
- **`check-tls-loopback.sh --remote <ssh-port> [target]`** runs legs 3–4 on the box instead of
  locally.

Gate class: **behavior-preserving scripts, verified differentially.** Old and new run on the same
input and their outputs are diffed before the old file is deleted.

Correctness risk concentrates in the regen merge. It writes into `tests/`, so it is done in its
own phase with a mutation test.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick in the same commit as the work; `- [~]`
> for partial; moot tasks stay struck through with evidence; fill `Commit:` on landing.
> **An unticked box means NOT DONE.**

### Phase 1 — Coverage merges (no tree writes, lowest risk)

- [ ] Produce one report JSON with `sh scripts/coverage-bins.sh`. Record the wall time and the
      JSON path here.
- [ ] Write `scripts/coverage-report.py` with subcommands `gaps`, `lines`, `shapes`, `dead` and
      `delta`, sharing one loader. The exceptions path is resolved relative to `__file__`.
- [ ] Differential: for each subcommand, run the old script and the new subcommand from the repo
      root on that JSON (for `delta`, on two JSONs; a copy with one file's counts edited is
      enough), then `diff` the stdout → identical. For `lines`, test both with and without
      `--source`. Record the five diff results.
- [ ] Also run the new script from `/tmp` → same output. This shows the cwd-relative exceptions
      bug is gone.
- [ ] Add `--bins` to `scripts/coverage.sh`. Run `sh scripts/coverage.sh --bins` and compare its
      JSON with `coverage-bins.sh`'s: they must be identical after sorting keys, or differ only in
      timestamps (record which).
- [ ] Run `sh scripts/coverage.sh` (no flag), then `sh scripts/coverage-check.sh` → same result as
      before (the CI path is unchanged).
- [ ] `git rm` `coverage-bins.sh` and the five `coverage-src-*.py`. Update `planning/tests.md` (every
      cited command) and the `scripts/README.md` coverage section.

Acceptance:
- The five subcommand diffs are empty.
- The CI-mode `coverage.sh` + `coverage-check.sh` exit status is unchanged.
- `git grep -n -e coverage-bins.sh -e coverage-src- -- ':!planning/completed' ':!planning/plan-131-*'` → 0.

Commit: —

### Phase 2 — TLS loopback merge

- [ ] Add `--remote <ssh-port> [target]` to `scripts/check-tls-loopback.sh`. It reuses the
      existing MFBASIC server/client sources and ships the `-glibc`/`-musl` `.out` files as
      `check-tls-loopback-remote.sh` does (read it first).
- [ ] On box 2228: run `bash scripts/check-tls-loopback-remote.sh target/release/mfb 2228` and
      `bash scripts/check-tls-loopback.sh target/release/mfb --remote 2228`. The PASS/FAIL lines
      must be the same. Record both.
- [ ] Locally, `bash scripts/check-net-harness-selftest.sh target/release/mfb` → exit 0. The
      sabotage legs must still fail for their reasons.
- [ ] `git rm scripts/check-tls-loopback-remote.sh`. Document `--remote` in the script header.

Acceptance:
- The remote legs match line for line.
- The selftest exits 0.

Commit: —

### Phase 3 — Baseline merge

- [ ] `git mv scripts/linux-artifact-baseline.sh scripts/artifact-baseline.sh`. Add
      `--targets` (default: the three Linux targets).
- [ ] Differential, before deleting `exe-oracle.sh`: run
      `exe-oracle.sh target/release/mfb macos-aarch64 record /tmp/eo.txt` and
      `artifact-baseline.sh target/release/mfb capture /tmp/ab.txt --targets macos-aarch64`.
      Every fixture's `.out` hash in `/tmp/eo.txt` must appear with the same hash in `/tmp/ab.txt`.
      Show this with a join on the fixture name. Record the fixture counts of both.
- [ ] Differential for the default: `artifact-baseline.sh … capture` with no `--targets` gives the
      same manifest as `linux-artifact-baseline.sh` did at HEAD (run the HEAD copy from
      `git show HEAD:scripts/linux-artifact-baseline.sh > /tmp/lab.sh`).
- [ ] `git rm scripts/exe-oracle.sh`.
- [ ] Update the callers:
      - `tests/gate/gate_lock_covers_every_writer.rs`: rename the classification row and keep it
        exempt, with its reason.
      - `scripts/linux-runtime-proof.sh`: the name reference.
      - `.ai/build-tooling.md`, `.ai/testing-gates.md`, `scripts/README.md`.

Acceptance:
- The hash join shows no mismatches and no exe-oracle fixture missing from the baseline.
- The default manifest is identical to HEAD's.
- The `tests/gate` tests pass.

Commit: —

### Phase 4 — Regen merge (writes into tests/, highest risk)

- [ ] Measure the two §2 UNMEASURED golden populations. Record them here.
- [ ] Write `scripts/regen-native-goldens.sh` per §3. It sources `gate-lock.sh` and
      `artifact-kinds.sh`, derives the host with `uname`, and checks the build exit before hashing.
- [ ] Neutral run: on a tree where the prerequisite `artifact-gate.sh all` is green, run
      `bash scripts/regen-native-goldens.sh target/release/mfb`. Then
      `git status --short tests` → empty.
- [ ] Mutation run: overwrite one golden of each shape with junk:
      - a byte-identity `.ncodesum`;
      - an outside-fixture `.ncodesum`;
      - a raw `.ncode`;
      - a raw `.nir`;
      - an `.app.` golden.
      Run the script, then `git diff --exit-code tests` → exit 0 (every file restored).
      Record the five paths.
- [ ] Failure run: make one fixture fail to build (temporarily, in a scratch copy of the tree).
      The script must exit non-zero and leave that golden untouched.
- [ ] Linux host: on box 2228, run the neutral run on a synced tree (use `git archive`, per memory
      `macos-tar-appledouble-breaks-a-linux-build`) → no diff in the macOS goldens. This is the
      `HOST` bug fix.
- [ ] `git rm` the three old regen scripts.
- [ ] `tests/gate/gate_lock_covers_every_writer.rs`:
      - replace the three rows with one locking row;
      - update the module doc and the "three contend without naming a dump flag" comment;
      - re-measure `SCAN_FLOOR` with the plan-131-A §2 loop and set it to the measured count.
- [ ] Update the callers:
      - `.ai/testing-gates.md` (regen section);
      - `.gitignore` comments (lines 11 and 65);
      - `scripts/README.md`;
      - the command lines in the open `bugs/bug-536-*.md` and `bugs/bug-540-*.md`.
- [ ] Record the memory update needed: `regen-ncodesum-hashes-stale-dump-on-failed-build` names the
      old script. Per AGENTS.md, a sub-agent makes that edit.

Acceptance:
- The neutral, mutation, failure and Linux-host runs all pass as specified.
- `cargo test --test gate_lock_covers_every_writer --test gate_mutual_exclusion --test gate_release_on_normal_exit --no-fail-fast` passes.
- `git grep -n -e regen-ncodesum -e regen-outside-ncode -e regen-rt-goldens -- ':!planning/completed' ':!bugs/completed' ':!planning/plan-131-*'` → 0.

Commit: —

## Validation Plan

- Tests: the three `tests/gate` lock/census tests.
- Differential proofs: recorded in each phase.
- Neutrality: `bash scripts/artifact-gate.sh target/release/mfb all` → 0 diffs at the end of B.
- Doc sync: `.ai/testing-gates.md`, `.ai/build-tooling.md`, `scripts/README.md`,
  `planning/tests.md`, open bug docs 536/540.

## Open Decisions

See plan-131-A.

## Corrections

## Summary

The risk is Phase 4. A regen script that writes the wrong hash re-baselines goldens silently. The
mutation and failure runs are what guard against that. Everything else is a script-to-script
differential. `coverage-check.sh`'s inline exceptions parsing is deliberately left alone, because
it is the CI gate.
