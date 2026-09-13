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
| plan-131-C complete | `ls planning/plan-131-C-* 2>/dev/null` → no match | NOT MET |
| User chose to run D (plan-131-A Open Decisions) | recorded there | MET (2026-09-12: run D; test on macOS now, boxes brought up later by the user) |
| Boxes reachable | `ssh -o BatchMode=yes -o ConnectTimeout=8 -p 2228 test@127.0.0.1 true` (and 2227, 2230) → exit 0 each | NOT MET at plan start (2026-09-12: 2227 exit 0; 2228, 2230 exit 255 connection refused). The user will bring the boxes up; the box legs wait for them |
| macOS window-server session for `test-macapp.sh` | `bash scripts/test-macapp.sh target/release/mfb` → exit 0 at the start of D | UNMEASURED |

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

- [ ] Count the duplicated lines per helper candidate in each script, citing a
      `grep -c`/`sed -n` per block. Record a table: helper → scripts → lines.
- [ ] Record each script's baseline PASS/FAIL lines on its box, normalized with
      `grep -E '^(PASS|FAIL|SKIP)' | sed -E 's/[0-9]+(\.[0-9]+)?(ms|s)\b/<t>/g'`:
      - `test-winprocess`: 2230
      - `test-winapp`: 2230
      - `test-appimage`: default boxes
      - `test-canvas-vulkan`: 2228
      - `test-macapp`: local
      - `linux-runtime-proof`: 2228, `linux-x86_64`
      Store them under `/tmp/131d-baseline/`, and paste the file line counts here.
- [ ] Present the measurement to the user if the Open Decision was conditional on it.

Acceptance: the duplication table and six baselines are recorded.
Commit: —

### Phase 2 — Helpers

- [ ] Write `scripts/remote-common.sh` and `scripts/rgba_compare.py` per §3.
- [ ] Write `scripts/remote-common-selftest.sh`, which covers:
      - `watchdog` kills a `sleep 30` at 1 s and returns non-zero;
      - `remote_ssh` against an unused port fails within `ConnectTimeout` + 2 s;
      - `pass`/`fail` counting and the summary exit code;
      - `rgba_compare.py` returns equal/unequal on two synthetic RGBA files at and just beyond tolerance.
      Run it → exit 0.

Acceptance: the selftest exits 0 and every case is shown to run (its output lists each case).
Commit: —

### Phase 3 — Migrate, one script per commit

For each script, in the §3 order:

- [ ] Replace its local copies with the helpers.
- [ ] Re-run it on the same box, normalize the output, and `diff` it against its Phase 1 baseline → empty.
- [ ] Record `wc -l` before and after.

The rows:

- [ ] `test-winprocess.sh`
- [ ] `test-appimage.sh`
- [ ] `linux-runtime-proof.sh`
- [ ] `test-winapp.sh`
- [ ] `test-canvas-vulkan.sh`
- [ ] `test-macapp.sh`
- [ ] Negative check once, on `test-winapp.sh`: point it at a closed port → it exits non-zero
      within `ConnectTimeout` + 5 s instead of hanging.

Acceptance:
- Six empty baseline diffs.
- The closed-port run fails fast.
- `grep -nE '\bssh -p|\bscp -P' scripts/test-*.sh scripts/linux-runtime-proof.sh` → 0 (all go
  through the helpers).

Commit: —

## Validation Plan

- Tests: `remote-common-selftest.sh`. It is added to `scripts/README.md` by E, and wiring it into
  a `tests/gate` spawn test is sub-plan E's job.
- Runtime proof: the six box baselines, diffed.
- Doc sync: `.ai/remote_systems.md` gains a short section: "remote proofs source
  `scripts/remote-common.sh`; `MFB_SSH_CONNECT_TIMEOUT`".

## Open Decisions

See plan-131-A (whether to run D at all).

## Corrections

## Summary

This is the only sub-plan that cannot be verified on one machine. Its value is ending the silent
ssh hang in three scripts and stopping the watchdog/compare copies from drifting. How much code
it removes is unmeasured until Phase 1, and nobody should promise a number before then.
