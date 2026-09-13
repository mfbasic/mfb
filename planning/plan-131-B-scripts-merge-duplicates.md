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

- [x] Produce one report JSON with `sh scripts/coverage-bins.sh`. Record the wall time and the
      JSON path here.
      `/usr/bin/time -p sh scripts/coverage-bins.sh`: the unit-test run completed, `real 3654.05`
      (61 min; it shared the CPU with the Phase 3/4 sweeps). The script then exited 1 at its report
      step: `failed to create file target/coverage/coverage.json: No such file or directory`, because
      nothing had created `target/coverage/` in this fresh worktree (see Corrections). The JSON was
      regenerated from the profile that run left, using the script's own report command after
      `mkdir -p target/coverage`: `target/coverage/coverage.json`, 50008226 bytes. Kept as
      `/tmp/p131-cov/bins-a.json`.
- [x] Write `scripts/coverage-report.py` with subcommands `gaps`, `lines`, `shapes`, `dead` and
      `delta`, sharing one loader. The exceptions path is resolved relative to `__file__`.
      The loader, the exceptions read and the `src/**` key function (repository prefix first) are
      each written once. The subcommand bodies are straight ports.
      `shapes` keeps its original fixed 98 floor, and `delta` applies exceptions to the counts only,
      as the originals did.
- [x] Differential: for each subcommand, run the old script and the new subcommand from the repo
      root on that JSON (for `delta`, on two JSONs; a copy with one file's counts edited is
      enough), then `diff` the stdout → identical. For `lines`, test both with and without
      `--source`. Record the five diff results.
      Run 2026-09-12 on the main checkout's `target/coverage/coverage.json` (a full llvm-cov
      export; re-checked on the fresh bins JSON below). Every `diff` is empty:
      - `gaps` (29 lines: `359 src/** files below 98%, 16935 uncovered lines`);
      - `lines src/arch/aarch64/encode/emitter.rs` (18 lines), and with `--source` (36 lines);
      - `shapes` (49 lines: `13386 uncovered region-entry lines`);
      - `dead --top 40` (83 lines: `1120 src/** functions never executed`);
      - `delta` with one report (361 lines), and with two (3 lines). The second report is a copy
        with `emitter.rs` covered lowered by 50 (`-5.13   97.64% ->  92.51%`).
      Re-run on the fresh bins report (`/tmp/p131-covdiff.sh /tmp/p131-cov/bins-a.json`), every diff
      empty:
      - `gaps` (29 lines: `155 src/** files below 98%, 4623 uncovered lines`);
      - `lines src/ir/lower.rs` (82 lines), and with `--source` (234);
      - `shapes` (49), `dead` (63);
      - `delta` on one report (157 lines), and on two, Aug 27 full → fresh bins (829 lines:
        `below 98%: 359 -> 155`).
- [x] Also run the new script from `/tmp` → same output. This shows the cwd-relative exceptions
      bug is gone.
      New `gaps` from `/tmp` is identical to new from the repo root (359 files). Old
      `coverage-src-gaps.py` from `/tmp` reports `372 src/** files below 98%, 18530 uncovered
      lines`, 13 more files, because it cannot find the exceptions file from there.
- [ ] Add `--bins` to `scripts/coverage.sh`. Run `sh scripts/coverage.sh --bins` and compare its
      JSON with `coverage-bins.sh`'s: they must be identical after sorting keys, or differ only in
      timestamps (record which).
- [ ] Run `sh scripts/coverage.sh` (no flag), then `sh scripts/coverage-check.sh` → same result as
      before (the CI path is unchanged).
      Where it runs was decided by the user, 2026-09-12: on this Mac, now. The full suite includes
      `tests/runtime/rt_audio_mml_bounds.rs`, which plays tunes through the default audio output, and
      each pass is hours of CPU. "Before" is HEAD's committed `coverage.sh`, which has no `--bins`. It
      runs from `scripts/.coverage-head.sh` in the same tree, after the "after" pass, so both passes
      use the same instrumented build.
- [ ] `git rm` `coverage-bins.sh` and the five `coverage-src-*.py`. Update `planning/tests.md` (every
      cited command) and the `scripts/README.md` coverage section.

Acceptance:
- The five subcommand diffs are empty.
- The CI-mode `coverage.sh` + `coverage-check.sh` exit status is unchanged.
- `git grep -n -e coverage-bins.sh -e coverage-src- -- ':!planning/completed' ':!planning/plan-131-*'` → 0.

Commit: —

### Phase 2 — TLS loopback merge

- [x] Add `--remote <ssh-port> [target]` to `scripts/check-tls-loopback.sh`. It reuses the
      existing MFBASIC server/client sources and ships the `-glibc`/`-musl` `.out` files as
      `check-tls-loopback-remote.sh` does (read it first).
      Spelled `check-tls-loopback.sh <mfb-exe> --remote <ssh-port> [linux-target]`. The server and
      client programs are now one `server_source <chain> <key>` and one `client_source` function,
      used by both modes. The remote mode carries the old script's logic and its exact messages.
      The selftest's sabotage `sed` still matches, since leg 1's `*"echo:hello-tls"*)` and its
      `did not round-trip` message are unchanged. Local mode output diffs empty against the old
      script (two `PASS` lines plus the macOS `SKIP`, exit 0).
- [x] On box 2228: run `bash scripts/check-tls-loopback-remote.sh target/release/mfb 2228` and
      `bash scripts/check-tls-loopback.sh target/release/mfb --remote 2228`. The PASS/FAIL lines
      must be the same. Record both.
      2026-09-12, box 2228 (glibc): old and new both print
      `PASS: tls::connect <-> tls::listen completed on linux-x86_64/glibc (port 2228)` and
      `PASS: tls::connect refuses a chain it cannot verify on linux-x86_64/glibc`, exit 0; `diff` is
      empty. Also box 2227 (musl): the same two lines with `linux-x86_64/musl (port 2227)`, and
      `diff` is empty.
- [x] Locally, `bash scripts/check-net-harness-selftest.sh target/release/mfb` → exit 0. The
      sabotage legs must still fail for their reasons.
      8/8 `ok` (the tls leg: as shipped PASS, sabotaged FAIL), exit 0.
- [x] `git rm scripts/check-tls-loopback-remote.sh`. Document `--remote` in the script header.
      The header has a `## --remote` section and both usage lines. Live references after removal:
      none (`git grep -n check-tls-loopback-remote -- ':!planning/completed' ':!bugs/completed'
      ':!planning/plan-131-*'` → exit 1).

Acceptance:
- The remote legs match line for line. (Met: 2228 and 2227 diffs empty.)
- The selftest exits 0. (Met: exit 0.)

Commit: 5625a41ea

### Phase 3 — Baseline merge

- [x] `git mv scripts/linux-artifact-baseline.sh scripts/artifact-baseline.sh`. Add
      `--targets` (default: the three Linux targets).
      `--targets t1,t2` follows the three positional arguments. A bad flag or a missing value prints
      the usage and exits 2 (both checked). The summary lines count the targets instead of
      hardcoding 3.
- [x] Differential, before deleting `exe-oracle.sh`: run
      `exe-oracle.sh target/release/mfb macos-aarch64 record /tmp/eo.txt` and
      `artifact-baseline.sh target/release/mfb capture /tmp/ab.txt --targets macos-aarch64`.
      Every fixture's `.out` hash in `/tmp/eo.txt` must appear with the same hash in `/tmp/ab.txt`.
      Show this with a join on the fixture name. Record the fixture counts of both.
      `exe-oracle.sh … macos-aarch64 record`: `fixtures=1459 executables=878`. The first
      `--targets macos-aarch64` capture had 0 executables, and the second had 877 of 878; both
      failures were root-caused and fixed in the script (see Corrections: dump builds do not link,
      and escaping vendor symlinks). Final capture: `baseline captured: 1459 fixture(s) x 1 target(s),
      6732 artifact hash(es)`. The join on fixture-relative executable path: 878 present, 0 hash
      mismatches, 0 missing, 0 executables only in the baseline.
- [x] Differential for the default: `artifact-baseline.sh … capture` with no `--targets` gives the
      same manifest as `linux-artifact-baseline.sh` did at HEAD (run the HEAD copy from
      `git show HEAD:scripts/linux-artifact-baseline.sh > /tmp/lab.sh`).
      The HEAD copy went to `scripts/.lab-head.sh`, not `/tmp`: it resolves `ROOT` from its own
      directory, so from `/tmp` it would sweep the wrong tree. It was removed after use. Both runs
      used `JOBS=10` and each printed `baseline captured: 1459 fixture(s) x 3 target(s), 17542
      artifact hash(es)`. `cmp` of the two manifests found them identical (17542 lines).
      That first identity only held because both scripts shared two bugs: no linked executable was
      ever hashed, and one fixture never built in its scratch copy (see Corrections). The final
      script fixes both, so the criterion was strengthened in Corrections. Final default capture
      from the finished script: `1459 fixture(s) x 3 target(s), 22825 artifact hash(es)`, checked
      against HEAD's manifest by `/tmp/p131-default-diff.sh`:
      - outside `rt-behavior/native/libsnd-open-file-info-rt`, the new manifest minus its `.out`
        lines is IDENTICAL to HEAD's (17539 lines);
      - 5262 lines added, 0 non-`.out`, 0 removed; `.out` lines: 1756 per Linux target (glibc + musl);
      - the permitted fixture goes `STATUS|build-failed` → `STATUS|ok` on all three targets, with
        glibc and musl `.out` hashes.
- [x] `git rm scripts/exe-oracle.sh`.
      Removed after the join above showed all 878 of its executable hashes reproduced by
      `artifact-baseline.sh --targets macos-aarch64`.
- [x] Update the callers:
      - `tests/gate/gate_lock_covers_every_writer.rs`: rename the classification row and keep it
        exempt, with its reason.
      - `scripts/linux-runtime-proof.sh`: the name reference.
      - `.ai/build-tooling.md`, `.ai/testing-gates.md`, `scripts/README.md`.
      All done. The census row is now `artifact-baseline.sh`, still exempt with the same reason (it
      builds in `$WORKDIR/w$slot`). `linux-runtime-proof.sh` names the new script. In
      `.ai/build-tooling.md` the recipe is renamed and gains `--targets`. In
      `.ai/testing-gates.md` the old "exe-oracle concurrent clobber" note is rewritten as the general
      in-tree-sweep rule, pointing at the scratch-copy script. `scripts/README.md` has one
      `artifact-baseline.sh` entry and no `exe-oracle.sh` entry. No live file names
      `exe-oracle` or `linux-artifact-baseline`: the grep with the plan-131/completed exclusions
      exits 1.

Acceptance:
- The hash join shows no mismatches and no exe-oracle fixture missing from the baseline.
  (Met: 878 present, 0 mismatched, 0 missing, from the finished script's capture.)
- ~~The default manifest is identical to HEAD's.~~ Strengthened in Corrections, because the old
  manifest omitted every linked executable and one fixture: outside `libsnd-open-file-info-rt` the
  new manifest minus `.out` lines is byte-identical to HEAD's, and every added line is `.out`.
  (Met: 17539 identical lines, 5262 added `.out` lines, 0 removed.)
- The `tests/gate` tests pass. (Met: 1 + 5 + 3, exit 0, after the census row rename.)

Commit: e0dc2b8c7

### Phase 4 — Regen merge (writes into tests/, highest risk)

- [x] Measure the two §2 UNMEASURED golden populations. Record them here.
      `.ncodesum`: 144 (6 `.app.`, 10 outside `tests/byte-identity/`). Raw native: 69 (7 `.ncode`,
      21 `.nir`, 21 `.nplan`, 18 `.nobj`, 2 `.mir`). Total 213; every one's filename prefix equals
      its fixture's `project.json` name (see Corrections).
- [x] Write `scripts/regen-native-goldens.sh` per §3. It sources `gate-lock.sh` and
      `artifact-kinds.sh`, derives the host with `uname`, and checks the build exit before hashing.
      It enumerates goldens exactly as `artifact-gate.sh`'s Pass 2 does: one build per target and
      mode, carrying every needed kind flag. It removes stale dumps before each build, never writes
      a golden unless that build exited 0 and produced the dump, and exits non-zero on any failure.
      It re-execs under bash from zsh (bug-513).
- [x] Neutral run: on a tree where the prerequisite `artifact-gate.sh all` is green, run
      `bash scripts/regen-native-goldens.sh target/release/mfb`. Then
      `git status --short tests` → empty.
      Run 2026-09-12 in the detached HEAD worktree `/tmp/p131-regen` (see Corrections):
      `regen-native-goldens: 166 build(s), 213 golden(s) rewritten, 0 failure(s)`, exit 0, 167 s.
      A sha256 snapshot of all 7596 files under `tests/`, taken before and after, is IDENTICAL, so
      every golden was rewritten with its committed bytes and no dump was left behind.
- [x] Mutation run: overwrite one golden of each shape with junk:
      - a byte-identity `.ncodesum`;
      - an outside-fixture `.ncodesum`;
      - a raw `.ncode`;
      - a raw `.nir`;
      - an `.app.` golden.
      Run the script, then `git diff --exit-code tests` → exit 0 (every file restored).
      Record the five paths.
      Run 2026-09-12 in `/tmp/p131-regen`. Each of these was overwritten with a junk line, then
      restored by the full sweep (`166 build(s), 213 golden(s) rewritten, 0 failure(s)`, exit 0):
      - byte-identity `.ncodesum`:
        `tests/byte-identity/crypto/golden/crypto_codegen_cover_rt.windows-x86_64.ncodesum`;
      - outside-fixture `.ncodesum`:
        `tests/rt-behavior/crypto/crypto-ec-valid/golden/crypto-ec-valid.linux-x86_64.ncodesum`;
      - raw `.ncode`: `tests/syntax/match/control-flow-match/golden/control_flow_match.macos-aarch64.ncode`;
      - raw `.nir`: `tests/rt-behavior/control-flow/control-flow-if/golden/control_flow_if.macos-aarch64.nir`;
      - `.app.` golden:
        `tests/syntax/app/macos-app-mode-term/golden/macos_app_mode_term.linux-x86_64.app.ncodesum`.
      The sha256 of each matches its pre-mutation value, and the full 7596-file `tests/` snapshot is
      IDENTICAL to the pre-mutation one.
- [x] Failure run: make one fixture fail to build (temporarily, in a scratch copy of the tree).
      The script must exit non-zero and leave that golden untouched.
      Run 2026-09-12 in `/tmp/p131-regen`: a non-MFBASIC line appended to
      `tests/rt-behavior/control-flow/control-flow-if`'s source, then
      `regen-native-goldens.sh <mfb> tests/rt-behavior/control-flow/control-flow-if` → exit 1,
      `BUILD FAILED tests/rt-behavior/control-flow/control-flow-if (macos-aarch64) — its goldens left
      unchanged`, `1 build(s), 0 golden(s) rewritten, 1 failure(s)`. The sha256 of all 10 files in
      its `golden/` is unchanged, no stale dump was left beside the fixture, and the source was
      restored byte-for-byte from its saved copy.
- [ ] Linux host: on box 2228, run the neutral run on a synced tree (use `git archive`, per memory
      `macos-tar-appledouble-breaks-a-linux-build`) → no diff in the macOS goldens. This is the
      `HOST` bug fix.
- [x] `git rm` the three old regen scripts.
      `regen-ncodesum.sh`, `regen-outside-ncode.sh` and `regen-rt-goldens.sh` are removed. Nothing
      still needed them: every Phase 4 run uses `regen-native-goldens.sh`.
- [x] `tests/gate/gate_lock_covers_every_writer.rs`:
      - replace the three rows with one locking row;
      - update the module doc and the "three contend without naming a dump flag" comment;
      - re-measure `SCAN_FLOOR` with the plan-131-A §2 loop and set it to the measured count.
      One `regen-native-goldens.sh` row, locking. The module doc, the history comment and the
      "contend without naming a dump flag" comment now describe it. The §2 loop measured 5
      (`artifact-baseline`, `artifact-gate`, `bench-lowering`, `ncode-determinism-alltargets`,
      `test-accept`), so `SCAN_FLOOR` = 5. It fell from 7 because the two deleted flag-naming regen
      scripts left the scan, and their replacement builds `-$ext` and names no literal flag (it is
      checked by classification, as the census does for every classified script).
      `rustfmt --check` on the file → exit 0.
- [x] Update the callers:
      - `.ai/testing-gates.md` (regen section);
      - `.gitignore` comments (lines 11 and 65);
      - `scripts/README.md`;
      - the command lines in the open `bugs/bug-536-*.md` and `bugs/bug-540-*.md`.
      All updated, plus the two open bug docs the plan did not list, `bug-564` and `bug-599` (see
      Corrections). `scripts/README.md` now has one `regen-native-goldens.sh` entry in place of the
      old `regen-ncodesum.sh` and `regen-rt-goldens.sh` entries; `regen-outside-ncode.sh` was never
      indexed.
- [x] Record the memory update needed: `regen-ncodesum-hashes-stale-dump-on-failed-build` names the
      old script. Per AGENTS.md, a sub-agent makes that edit.
      Done by a sub-agent, 2026-09-12: the memory now names `scripts/regen-native-goldens.sh`, keeps
      the rm-the-dump-then-check-the-exit rule and the zsh re-exec (bug-513), and its MEMORY.md
      index line was updated to match.

Acceptance:
- The neutral, mutation, failure and Linux-host runs all pass as specified.
- `cargo test --test gate_lock_covers_every_writer --test gate_mutual_exclusion --test gate_release_on_normal_exit --no-fail-fast` passes.
- `git grep -n -e regen-ncodesum -e regen-outside-ncode -e regen-rt-goldens -- ':!planning/completed' ':!bugs/completed' ':!planning/plan-131-*'` → 0.
  (Met 2026-09-12: exit 1, no hits. The three `tests/gate` tests pass, 1 + 5 + 3, exit 0.)

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

- **2026-09-12, Phase 4 populations (were UNMEASURED).** `find tests -path '*/golden/*.ncodesum' | wc -l`
  → 144 (6 of them `.app.`, 10 outside `tests/byte-identity/`). Raw native goldens → 69: 7 `.ncode`,
  21 `.nir`, 21 `.nplan`, 18 `.nobj`, 2 `.mir`.
- **2026-09-12, Phase 4 local runs use a scratch worktree, not the integration tree.** The neutral,
  mutation and failure runs need "a tree where `artifact-gate all` is green". A detached worktree of
  HEAD at `/tmp/p131-regen` has the same goldens (the gate was green at `6347f0be5`, with no
  `tests/` or compiler change since) and its own gate lock. Using it let the runs proceed while
  `exe-oracle.sh` was building in-tree in the integration worktree; two in-tree sweeps in one tree
  clobber each other (`.ai/testing-gates.md`). Tree state is checked with a sha256 snapshot of every
  file under `tests/` before and after, which is stricter than `git status --short tests` (it also
  catches a rewritten golden whose new bytes happen to equal the old).
- **2026-09-12, Phase 4 Linux-host run on box 2227, not 2228.** The task needs a Linux host building
  `mfb` from a shipped tree. 2228 has 1 core and a `rust-lld` that segfaults on the large link
  (`.ai/remote_systems.md`); 2227 (Alpine x86_64, 4 cores, `/usr/bin/cargo` 1.96.1) is equally a
  Linux host for the `uname`-derived HOST fix. The tree is shipped as `git archive HEAD` plus the
  uncommitted `regen-native-goldens.sh`, built with `CARGO_TARGET_DIR=/tmp/p131-target`.
- **2026-09-12, Phase 1: back-to-back coverage runs merged each other's profiles.** Neither coverage
  script cleans `target/llvm-cov-target/*.profraw`, and `cargo llvm-cov report` merges every profraw
  it finds. After four instrumented runs there were 30 profraw files, time-stamped 16:3x (25),
  17:3x (2), 17:4x (1) and 18:2x (2). So the first `coverage.sh --bins` JSON was not comparable
  with `coverage-bins.sh`'s. Measured by `/tmp/p131-covratio.py`:
  - segments: 539422 counts exactly 2×, 418948 zero in both, 29056 with some other ratio
    (e.g. `880` vs `1761`);
  - one per-file line summary differs;
  - functions: 17992 exactly 2×, 1747 other, 2068 equal.

  The first CI-mode "after" pass would have merged the same stale data, but CI always starts from a
  fresh checkout. That pass was killed, along with the release build its tests had started, and the
  whole of Phase 1's comparison was restarted. Each run now begins with `cargo llvm-cov clean
  --workspace`: `coverage-bins.sh`, then `coverage.sh --bins`, then compare; then new `coverage.sh` +
  `coverage-check.sh`; then HEAD's `coverage.sh` + `coverage-check.sh` (`/tmp/p131-cimode2.sh`).
  The five-subcommand differential is unaffected: it compares two scripts on one and the same
  report file.
- **2026-09-12, Phase 1: `coverage-bins.sh` could not write its report in a fresh worktree.** Its
  `cargo llvm-cov report … --json --output-path target/coverage/coverage.json` does not create the
  directory. In the main checkout an earlier full `coverage.sh` run (its `--html --output-dir
  target/coverage` pass) had created it, so the script only ever worked after one. Here the 61-minute
  test run finished and the report step failed (exit 1). `coverage.sh --bins` copied that report
  step, so it now runs `mkdir -p target/coverage` first. `coverage-bins.sh` is being deleted, so it
  is not patched: its JSON for the comparison was rebuilt by its own report command from the kept
  profile.
- **2026-09-12, Phase 3: the Linux baseline never hashed a linked executable.** `mfb build` given
  dump flags writes the dumps and does not link: a scratch copy of `byte-identity/audio` built with
  the script's flags for `macos-aarch64` has no `build/` directory, while a plain build of the same
  copy writes `build/audio_codegen_cover_rt.out`. So the `build/*.out` loop never matched anything.
  HEAD's default manifest has 0 `.out` lines out of 17542 (`grep -c '\.out|'`), and the first
  `--targets macos-aarch64` capture had 878 `STATUS|ok` fixtures but 0 executables. The join against
  `exe-oracle.sh`'s 878 hashes therefore reported 878 missing. The header's claim that the manifest
  covers linked executables was false in the original script too. Fixed in `artifact-baseline.sh`:
  after the dump build, a second, plain build in the same scratch copy links, and its `build/*.out`
  is hashed.
  **Acceptance strengthened accordingly.** The default-mode differential can no longer be
  byte-identical to HEAD, because the fix adds `.out` lines. It becomes: the new default manifest
  with its `.out` lines removed is byte-identical to HEAD's, and every added line is an `.out` line.
  The macOS join must still show 0 missing and 0 mismatched against `exe-oracle.sh`.
- **2026-09-12, Phase 3: one fixture never built in the scratch copy.** After the link fix, the macOS
  join showed 877 of `exe-oracle.sh`'s 878 executables present with 0 mismatches, and 1 missing:
  `rt-behavior/native/libsnd-open-file-info-rt`. Its `vendor/` holds 7 relative symlinks into
  `packages/libsnd/vendor/`. `cp -R` copies them as links, which dangle under `/tmp`, so the scratch
  build fails with `NATIVE_LIBRARY_SOURCE_UNREADABLE`. HEAD's manifest already records this fixture as
  `STATUS|build-failed` on all three Linux targets, so the bug predates this plan. `exe-oracle.sh`
  built it in-tree, where the links resolve. Measured over the whole tree: 11 symlinks under
  `tests/`, all resolving in-tree. The 7 vendor links are the only ones that escape their fixture; the
  4 in the fs fixtures point inside the fixture or at an absolute path, so they resolve in a copy.
  Fixed in `artifact-baseline.sh`: after the copy, any symlink that dangles in the scratch copy but
  resolves in the fixture is replaced with a copy of its target. Links that resolve are kept, because
  the fs fixtures test symlinks.
  **Default differential, strengthened again.** With the vendor fix,
  `rt-behavior/native/libsnd-open-file-info-rt` builds on the Linux targets for the first time. So
  besides the added `.out` lines, its lines are the only other permitted difference from HEAD: its
  STATUS goes `build-failed` → `ok`, and its dump lines appear. Every other fixture's non-`.out` lines
  must stay byte-identical to HEAD.
- **2026-09-12, Phase 3: `artifact-baseline.sh` had the same `shasum`-only hashing.** On a host
  without `shasum` every manifest hash would be empty. A `capture` and a later `verify` on that host
  would then agree on emptiness and report "no differences" whatever changed, a silent pass. Fixed with
  the same `sha256_of` helper (`shasum -a 256` or `sha256sum`, exit 2 when neither exists), exported to
  the `xargs` workers. On macOS the manifest bytes are unchanged: a `byte-identity/audio` capture
  after the change is byte-identical to the one before (7 lines).
- **2026-09-12, Phase 3: one default re-capture was discarded.** Its script file was rewritten in place
  while it ran (a comment edit that shifted every later byte). Bash reads a script incrementally, so
  that run's final stage could have executed shifted text. It was killed, and the final default and
  macOS captures were re-run from the finished script, so the recorded results come from the exact
  file being committed.
- **2026-09-12, Phase 4: a host without `shasum` got empty sum goldens.** Box 2227 (Alpine) has no
  `shasum` and no perl; it has busybox `sha256sum` (`command -v shasum` → not found). The sum write
  `shasum -a 256 "$af" | cut -d' ' -f1 > "$gf"`, which `regen-native-goldens.sh` inherited from the
  old regen scripts, would there write an EMPTY file into every `…sum` golden and count it as
  rewritten. That is a silent corruption on exactly the Linux-host leg this phase exists to prove.
  Fixed: a `sha256_of` helper uses `shasum -a 256` or `sha256sum`, the script exits 2 up front when
  neither exists, and a sum golden is written only when the digest is 64 lowercase hex characters.
  `artifact-gate.sh` uses `shasum` too, but only to compare, so on such a host it reports DIFF on
  every sum (loud, not silent). It stays untouched per this sub-plan's non-goals, and the Linux-host
  run is judged by a sha256sum snapshot of `tests/` rather than by the gate.
  Verified in `/tmp/p131-regen`. With a `PATH` of 1250 system tools minus `shasum` and `sha256sum`,
  the run exits 2 with `neither shasum nor sha256sum is on PATH`, and the 18 goldens of
  `crypto-ec-valid` and `control-flow-if` keep their sha256. A normal scoped run on those two
  fixtures gives `5 build(s), 9 golden(s) rewritten, 0 failure(s)`, exit 0, and the same 18
  goldens are byte-identical.
- **2026-09-12, the merged regen script finds goldens by the project.json name; every golden uses it.**
  The old `.ncodesum` script took the package name from each golden's filename, while
  `regen-native-goldens.sh` (like `artifact-gate.sh`) matches `<pkg>.*` with `<pkg>` from
  `project.json`. A golden whose prefix differed would silently stop being regenerated. Measured
  with a walk of every `tests/**/golden/` native golden: 213 goldens, 213 prefixes equal to their
  fixture's `project.json` name, 0 mismatches.
- **2026-09-12, the lock census cannot see `regen-native-goldens.sh` by its flag scan.** It builds
  `-$ext` from `artifact-kinds.sh` and spells no literal dump flag. It is classified by hand, and the
  census test checks every classified script whether or not the scan sees it. The scan population
  therefore falls by the two deleted flag-naming regen scripts; `SCAN_FLOOR` is re-measured at
  deletion time.
- **2026-09-12, more live references than §1/Phase 4 listed.** Besides bug-536 and bug-540, the open
  `bugs/bug-564-*` and `bugs/bug-599-*` cite `regen-ncodesum.sh`. They are re-worded to name
  `regen-native-goldens.sh` without restating the old name, so the Phase 4 zero-hit grep holds and
  the historical record still reads true.

## Summary

The risk is Phase 4. A regen script that writes the wrong hash re-baselines goldens silently. The
mutation and failure runs are what guard against that. Everything else is a script-to-script
differential. `coverage-check.sh`'s inline exceptions parsing is deliberately left alone, because
it is the CI gate.
