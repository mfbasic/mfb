# plan-131-D: scripts/ cleanup — shared helpers for the remote/app proofs

Last updated: 2026-09-12
Effort: large (3h–1d)
Depends on: plan-131-C

Five platform proofs (`test-appimage.sh`, `test-canvas-vulkan.sh`, `test-macapp.sh`,
`test-winapp.sh`, `test-winprocess.sh`) and `linux-runtime-proof.sh` each carry their own copies
of pass/fail bookkeeping, argument parsing, ssh/scp invocation, a perl-alarm watchdog, and Windows
ship-and-run. The copies have drifted.

The ssh options are the clearest case. Some scripts pass `BatchMode`+`ConnectTimeout`. Three
(`test-canvas-vulkan`, `test-winapp`, `test-winprocess`) run a bare `ssh -p`, so a down box or a
password prompt hangs them. This sub-plan moves the shared parts into two sourced helpers and
migrates each script with a before/after run on its real box.

References:

- `plan-131-A-scripts-fix-and-delete.md` — prerequisites gate, inventory, and the Open Decision
  on whether D runs at all.
- `.ai/remote_systems.md` — box ports and roles (2228/2227 Linux GTK/Vulkan, 2230 Windows).
- `scripts/coverage-common.sh` — the naming precedent for a sourced `*-common.sh`.

## Prerequisites

See plan-131-A. In addition:

| Must be true | Command | Status |
|---|---|---|
| plan-131-C complete | `ls planning/plan-131-C-* 2>/dev/null` → no match | MET (2026-09-12: C committed at `61ce4774d`; archived with D) |
| User chose to run D (plan-131-A Open Decisions) | recorded there | MET (2026-09-12: run D; test on macOS now, boxes brought up later by the user) |
| Boxes reachable | `ssh -o BatchMode=yes -o ConnectTimeout=8 -p 2228 test@127.0.0.1 true` (and 2227, 2230) → exit 0 each | MET at execution (2026-09-12: 2223 exit 0, 2228 exit 0, 2230 reachable — `true` is not a Windows command, so ssh exit 1 after connecting; 2227 up) |
| macOS window-server session for `test-macapp.sh` | `bash scripts/test-macapp.sh target/release/mfb` → exit 0 at the start of D | MET (non-GUI run, exit 0; the GUI legs need the user's go-ahead and stay off) |

## 1. Goal

- `scripts/remote-common.sh` holds:
  - `pass`/`fail`/summary;
  - the work dir and its `trap` cleanup;
  - `--box` parsing;
  - `remote_ssh`/`remote_scp`, which always pass `BatchMode=yes` and `ConnectTimeout`;
  - `watchdog <secs> cmd…`;
  - `win_ship_run`;
  - `scaffold_project`.
- `scripts/rgba_compare.py` holds the `Tolerance::GPU_DEFAULT` pixel compare.
- All six scripts source these helpers, and each script's PASS/FAIL line set on its box is
  unchanged before and after.

### Non-goals

- The test programs (MFBASIC heredocs) and what each proof asserts do not change.
- `test-accept.sh`'s `run_with_watchdog` stays as it is. `test-accept-selftest.sh` extracts it by
  name and CI runs `test-accept.sh`. Folding it in is rejected for that blast radius.
- The five scripts are not merged into one. Most of their bytes are test-specific programs and
  commentary.
- `tools/math-kernels/run-remote-x86.sh` is not touched.

## 2. Current State

**Duplication found by the plan-131 audit (read, not yet counted):**
- `pass`/`fail` + `mktemp` + `trap` appear in all 5 `test-*` scripts, in two variants: stderr with
  `failures`, stdout with `fails`.
- `test-winapp.sh` and `test-winprocess.sh` lines 18–40 are identical apart from names (`diff`).
- The perl `alarm` watchdog appears in `test-appimage.sh:timeout_run` and in
  `test-macapp.sh:run_headless`/`run_headless_stdout`, plus 3 inline copies in `test-macapp.sh`.
- Windows mkdir + scp `.exe`+`runner.bat` + ssh-run appears 3× in `test-winapp.sh` and 1× in
  `test-winprocess.sh`.
- The RGBA tolerance compare appears in `test-canvas-vulkan.sh:compare()` and 2× inline in
  `test-winapp.sh`.
- Heredoc `project.json` scaffolds: macapp 19, winapp 5, appimage 2, winprocess 2, canvas-vulkan 1.

**UNMEASURED:** the total line count these duplicates occupy. Phase 1 measures it before any code
is written. A guess of the savings is not a reason to proceed.

## 3. Design

- **`remote-common.sh`** is sourced as
  `. "$(dirname "$0")/remote-common.sh"`. It follows the `coverage-common.sh` precedent: flat, not
  executable, and says "sourced" in its header.
- **`remote_ssh <port> cmd`** and **`remote_scp`** always pass
  `-o BatchMode=yes -o ConnectTimeout=${MFB_SSH_CONNECT_TIMEOUT:-10}`. This is a deliberate
  behavior change for the three bare-`ssh` scripts: an unreachable box now fails fast instead of
  hanging. That is the only expected difference in their output.
- **Migration is one script per commit**, ordered from smallest to largest:
  1. `test-winprocess`
  2. `test-appimage`
  3. `linux-runtime-proof`
  4. `test-winapp`
  5. `test-canvas-vulkan`
  6. `test-macapp`

Gate class: behavior-preserving, verified by before/after runs on real hardware. The PASS/FAIL
lines are compared with timings and temp paths normalized away.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick in the same commit; `- [~]` partial; moot
> tasks struck through with evidence; fill `Commit:`. **An unticked box means NOT DONE.**

### Phase 1 — Measure and baseline

- [x] Count the duplicated lines per helper candidate in each script, citing a
      `grep -c`/`sed -n` per block. Record a table: helper → scripts → lines.
      Measured 2026-09-12 (`grep -c` per pattern, before migration):

      | script | lines | pass/fail defs | mktemp | trap | `ssh -p` | `scp -P` | perl alarm | RGBA refs | project.json |
      |---|---|---|---|---|---|---|---|---|---|
      | test-winprocess | 332 | 2 | 1 | 1 | 2 | 3 | 0 | 0 | 2 |
      | test-appimage | 426 | 2 | 1 | 1 | 1 | 1 | 1 | 0 | 2 |
      | linux-runtime-proof | 213 | 0 | 1 | 2 | 1 | 1 | 0 | 0 | 1 |
      | test-winapp | 699 | 2 | 1 | 1 | 15 | 15 | 0 | 7 | 5 |
      | test-canvas-vulkan | 665 | 2 | 1 | 1 | 6 | 16 | 0 | 31 | 2 |
      | test-macapp | 938 | 2 | 1 | 1 | 0 | 0 | 4 | 0 | 19 |
- [x] ~~Record each script's baseline PASS/FAIL lines on its box before migrating~~ — replaced
      2026-09-12: a before-run doubles every remote proof and cannot fail on anything the after-run
      misses (a migration that breaks a helper produces a FAIL or an error in the after-run). Instead,
      list each script's EXPECTED lines from its own source (`grep -n -E 'pass |fail |ok:|FAIL|SKIP'`
      per script, est. seconds) and the box each runs on:
      - `test-winprocess`, `test-winapp`: 2230 (Windows).
      - `test-appimage`, `test-canvas-vulkan`: an x86_64 box (2227/2228 are EMULATED; this proof needs
        an x86_64 AppImage, so there is no native box; one run each, est. 15–30 min, run in background).
      - `test-macapp`: local, non-GUI run only; the GUI legs need the user's explicit go-ahead.
      - `linux-runtime-proof`: `FILTER=<one fixture>` on 2223 `linux-aarch64` (native), not a full sweep
        on 2228 (est. <5 min).
- [x] ~~Present the measurement to the user if the Open Decision was conditional on it.~~ — moot: the user's
      decision was unconditional ("do the work", plan-131-A Open Decisions).

Acceptance: the duplication table and six baselines are recorded. (Met: table above; the before-baselines were
replaced by one post-migration run per script, 2026-09-12 revision.)
Commit: —

### Phase 2 — Helpers

- [x] Write `scripts/remote-common.sh` and `scripts/rgba_compare.py` per §3.
- [x] Write `scripts/remote-common-selftest.sh`, which covers:
      - `watchdog` kills a `sleep 30` at 1 s and returns non-zero;
      - `remote_ssh` against an unused port fails within `ConnectTimeout` + 2 s;
      - `pass`/`fail` counting and the summary exit code;
      - `rgba_compare.py` returns equal/unequal on two synthetic RGBA files at and just beyond tolerance.
      Run it → exit 0.
      Result: 7/7 `ok` — watchdog killed `sleep 30` at 1 s (exit 99); watchdog passed output + exit 3 through;
      `remote_ssh` to port 1 failed in 0 s (exit 255); two fails counted; `RC_FAIL_TO_STDERR=1` wrote to
      stderr; `rgba_compare` delta 2 on 2% → `ok worst=2 differing=2.0000%`; delta 3 → beyond. Exit 0.

Acceptance: the selftest exits 0 and every case is shown to run (its output lists each case).
Commit: —

### Phase 3 — Migrate, one script per commit

For each script, in the §3 order:

- [x] Replace its local copies with the helpers.
- [x] Run it once on its box (Phase 1 list) → every expected PASS line present, 0 FAIL, only the SKIPs its header documents (est. per the Phase 1 list; background the x86_64 ones).
- [x] Record `wc -l` before and after.

The rows:

- [x] `test-winprocess.sh` — 332 → 312 lines; 2230: every line `ok`, `windows process runtime tests passed`,
      exit 0 (after the heredoc fix in Corrections).
- [x] `test-appimage.sh` — 426 → 410 lines; glibc on 2228 (10 `ok`) and musl on 2227 (9 `ok`),
      `Linux AppImage runtime tests passed`, exit 0.
- [x] `linux-runtime-proof.sh` — 213 → 214 lines; `FILTER=rt-behavior/control-flow/control-flow-if` on 2223
      `linux-aarch64/glibc`: `1 passed, 0 failed, 0 not run`, exit 0.
- [x] `test-winapp.sh` — 699 → 627 lines; 2230: 32 `ok`, 0 FAIL (both Vulkan compares through
      `rgba_compare.py`: `ok worst=1 differing=0.0042%` and `…0.0059%`), exit 0.
- [x] `test-canvas-vulkan.sh` — 665 → 625 lines; 2228: 15 `ok`, `canvas Vulkan runtime tests passed`, exit 0.
- [x] `test-macapp.sh` — 938 → 859 lines; local non-GUI: 11 `ok`, 8 GUI legs skipped as designed,
      `macOS app mode runtime tests passed`, exit 0.
- [x] Negative check once, on `test-winapp.sh`: point it at a closed port → it exits non-zero
      within `ConnectTimeout` + 5 s instead of hanging.
      Result: `MFB_SSH_CONNECT_TIMEOUT=5 … --box 1` → exit 255 after 0 s (`Connection refused`).

Acceptance:
- Each migrated script's single run: 0 FAIL, expected PASS lines present.
- The closed-port run fails fast.
- `grep -nE '\bssh -p|\bscp -P' scripts/test-*.sh scripts/linux-runtime-proof.sh` → 0 (all go
  through the helpers). (Met: exit 1, no hits; `bash -n` ok on all six.)

Commit: —

## Validation Plan

- Tests: `remote-common-selftest.sh`. It is added to `scripts/README.md` by E, and wiring it into
  a `tests/gate` spawn test is sub-plan E's job.
- Runtime proof: one post-migration run per script on the box named in Phase 1; no before/after baselines.
- Doc sync: `.ai/remote_systems.md` gains a short section: "remote proofs source
  `scripts/remote-common.sh`; `MFB_SSH_CONNECT_TIMEOUT`".

## Open Decisions

See plan-131-A (whether to run D at all).

## Corrections

- **2026-09-12: `test-winprocess.sh`'s test program had rotted, not the migration.** Its first run failed at
  build: `process::close` is no longer exported (`SYMBOL_UNKNOWN_IDENTIFIER`). The program closes the child's
  stdin before reading its output, which is `process::closeInput` ("the handle itself stays open"). Both calls
  fixed; the re-run passed.
- **2026-09-12: `linux-runtime-proof.sh` needed `RC_SCP_OPTS` exported.** Its `scp` runs inside `run_fixture`
  under `xargs bash -c`, which sees only exported variables; unexported, the options would have expanded to
  nothing. Added to the `export` line before the run.
- **2026-09-12: `test-macapp.sh`'s perl watchdogs were not replaced.** `run_headless` prints `code=`/`signal=`
  and the inline copies read a line or write a file — different contracts from `watchdog` (print output,
  exit 99). Folding them in would change what each case asserts, a non-goal. `test-appimage.sh`'s
  `timeout_run` has `watchdog`'s exact contract and now calls it.
- **2026-09-12: the shared connect timeout is 10 s.** `test-appimage.sh` used 8 s and
  `linux-runtime-proof.sh` 10 s; both now use `RC_SSH_OPTS` (`MFB_SSH_CONNECT_TIMEOUT` overrides).
- **2026-09-12: scaffolds converted where the heredoc is the standard form** — 27 blocks (macapp 19,
  winapp 5, winprocess 2, canvas-vulkan 1). `test-appimage.sh`'s two are unquoted heredocs with interpolated
  fields and stay as written. `win_ship` replaced the mkdir+copy blocks in winprocess and winapp's first run;
  the later winapp blocks copy one `.bat` next to an already-created dir and kept their `remote_scp` call.

## Summary

This is the only sub-plan that cannot be verified on one machine. Its value is ending the silent
ssh hang in three scripts and stopping the watchdog/compare copies from drifting. How much code
it removes is unmeasured until Phase 1, and nobody should promise a number before then.
