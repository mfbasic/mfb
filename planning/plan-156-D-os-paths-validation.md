# plan-156-D: os path family, final validation and golden refresh

Last updated: 2026-09-24
Effort: medium (1h–2h)
Depends on: plan-156-C

This sub-plan proves that plan-156 works as a whole and refreshes the goldens
A through C knowingly left red. It holds the feature's one full-suite run, the
widest runtime sweep the reachable boxes allow, the `.ai` notes, and the
archive of all four sub-plans.

**Correct end state:**

- The full suite is green.
- The only golden changes are those listed below.
- All five calls behave per the plan-156 family contract on every reachable OS.

References:

- plan-156-A (the prerequisites and the family contract), plan-156-B,
  plan-156-C.
- `.ai/testing-gates.md`: regenerating goldens, "Concurrent test-accept
  clobbers actuals", and the known-red baseline.
- AGENTS.md: "Before re-baselining a golden, run the full suite, never one
  module."

## Prerequisites

See plan-156-A. On top of that, plan-156-C must be complete: every C phase has
a filled `Commit:` line.

## 1. Goal

- `scripts/test-accept.sh` and `cargo test` pass.
- The artifact gate passes after regenerating **only** the expected goldens:
  - `tests/byte-identity/os/golden/*` (5 `.ncodesum` files, plus `.ast`/`.ir`/
    `build.log`);
  - the renamed/new `tests/rt-behavior/os/func_os_*Path*` and
    `tests/syntax/os/func_os_*Path*_invalid` fixtures;
  - `tests/rt-behavior/strings/self-update-grow-valid/golden/*`;
  - `tests/acceptance` goldens touched by `os.mfb`;
  - any rendered-plist golden plan-156-C C5 named.

### Non-goals (explicit constraints)

- No goldens outside that list are re-baselined. An unexpected diff sends you
  bug hunting: inspect ONE fixture and localize it (AGENTS.md). It is never
  re-baselined to get to green.

## 2. Current State

A through C have landed, and their per-phase checks passed. The goldens listed
above are expected red.

### Measured populations

| What | Count | Command |
|---|---|---|
| Expected-red golden set | UNMEASURED until the D1 run; its first act is to record the red list here | `scripts/test-accept.sh target/debug/mfb target/accept-actual-156d` output |
| Reachable boxes | re-probe at D start (2026-09-24: 2226, 2230 up; 2222/2223/2224/2225/2227/2228/2229/2232 refused) | `for p in 2222 2223 2224 2225 2226 2227 2228 2229 2230 2232; do ssh -o ConnectTimeout=4 -o BatchMode=yes -p $p test@127.0.0.1 true && echo $p up; done` |

## 3. Design Overview

The order is: one full run, classify every red, regenerate only the expected
reds, then do the runtime sweep. Byte-identity serves only as a change sentinel
here (the `os` sums are expected to move). Correctness comes from the
rt-behavior runs across OSes.

## Phases

> **NOTE: keep the checkboxes current as you go.** Tick in the same commit as
> the work. Use `- [~]` for partial, mark moot as `- [x] ~~text~~ — moot:
> <evidence>`, and fill `Commit:` on landing. **An unticked box means NOT DONE.**

### Phase D1: full suite, classify, regenerate

- [x] `cargo build && cargo test`. Record pass/fail counts here. — scoped per Correction D-1: `cargo test --bin mfb os` → `586 passed; 0 failed`; `self_update` → `11 passed`; `spec` → `43 passed`; `app_info_plist` → `4 passed`; `--test inplace_self_update_census` → `2 passed`; `MFB_SELF_UPDATE_FILTER="os::" --test rt_inplace_self_update` → `3 passed`; `--test codegen_win64_host_paths --test codegen_win64_app_resource_path` → `1 passed`, `3 passed`.
- [x] `scripts/test-accept.sh target/debug/mfb target/accept-actual-156d`.
      Record every red fixture in §2's table. — scoped: `'func_os_*' 'os-*' 'byte-identity/os' 'self-update-grow-valid' 'libsnd-playback-rt' 'acceptance' 'func_fs_createDirectories_*' 'fs-create-directories-*' 'byte-identity/fs'` → 64 ran, 10 mismatches.
- [x] Classify each red as in-list (expected) or not. For every red outside the
      list, inspect that one fixture, find the cause, and fix it at the source.
      Record it in Corrections. — all 10 in-list: `byte-identity/os` `.ast`/`.ir`, the two renamed `func_os_appResourcePath_*` fixtures' `build.log`/`.ast`/`.ir`, `self-update-grow-valid` `.ast`/`.ir`. None outside the list.
- [x] Regenerate only the in-list goldens (the `sync-goldens.sh` /
      `.ai/testing-gates.md` "Regenerating the goldens" procedure). Check each
      regenerated `.ast`/`.ir` diff by eye. It may only rename `resourcePath` →
      `appResourcePath`, add the new calls, or add the fixture lines A through C
      added. — done with `scripts/sync-goldens.sh` (12 files, 4 tests). `self-update-grow-valid` checked: the old golden with `os.resourcePath`→`os.appResourcePath` substituted equals the new one (`only-rename`). `reads_resource`'s run output is unchanged. `_valid`'s adds the 5 A2 lines. The byte-identity `.ast` adds the 4 new calls.
- [x] Regenerate the `os` byte-identity `.ncodesum` for all five targets, then
      run `scripts/artifact-gate.sh`. — done with `bash scripts/regen-native-goldens.sh target/debug/mfb tests/byte-identity/os tests/byte-identity/fs` (`10 golden(s) rewritten, 0 failure(s)`). It changed exactly the 5 `os` sums and the `fs` Windows sum (POSIX `fs` unchanged, per plan-156-C C-4). Then `scripts/artifact-gate.sh target/debug/mfb os` / `fs` → `0 diff(s)` each.

Acceptance: both suites and the artifact gate are green, and every regenerated
golden is on the list.
  Check: `scripts/test-accept.sh target/debug/mfb target/accept-actual-156d-2`
  → 0 failures, and `scripts/artifact-gate.sh` → pass (est. 40 min. This is the
  plan's one full run, per AGENTS.md "never one module" before a re-baseline).
  Result (scoped per Correction D-1): the scoped re-run and the `os`/`fs` artifact
  gates are green (see the task lines); the re-run of `test-accept.sh … 156d2` → `acceptance tests passed (59 test(s) ran)`.
Commit: 759fc1379

### Phase D2: cross-OS runtime sweep

- [x] Re-probe the boxes (§2 command), and record the result here. — 2026-09-24 re-probe: 2226, 2230 up; 2222, 2223, 2224, 2225, 2227, 2228, 2229, 2232 refused.
- [x] For each reachable Linux box, run
      `FILTER=func_os_ scripts/linux-runtime-proof.sh target/debug/mfb <port> <target> <flavor>`.
      2226 = `linux-aarch64 glibc`; if up, 2227 = `linux-x86_64 musl`,
      2228 = `linux-x86_64 glibc`, 2229 = `linux-riscv64 musl`,
      2224 = `linux-aarch64 musl`. — 2226 (`linux-aarch64 glibc`): `FILTER=/os/` → `32 passed, 0 failed, 0 not run`. The other listed boxes were unreachable, so there is no x86_64, musl or riscv64 execution; those targets are covered by the regenerated `.ncodesum` goldens and the lowering tests only.
- [x] On 2230, run the `func_os_*Path*` fixtures with the plan-156-B B4 recipe. — `/tmp/156-win-proof.sh` → PASS for all six `func_os_*Path*` fixtures, plus `fs-create-directories-native-separators-rt` and `func_fs_createDirectories_valid`.
- [x] Run the macOS app mode: build a scratch `--app` project that prints all
      five calls, and run it through `scripts/test-macapp.sh`'s launch path.
      `appResourcePath()` must end in `/Contents/Resources`, `appDataPath()` in
      `/Library/Application Support/<name>`. — `/tmp/p156app` (`mfb build --app`, run with `MFB_MACAPP_HEADLESS=1 …/Contents/MacOS/p156app`) → exit 0. It wrote `…/p156app.app/Contents/Resources`, `~/Library/Application Support/p156app`, `~/Library/Caches/p156app/c`, `/Users/justinzaun`, `~/Documents`, and read back the bundled `data/r.txt`. `plutil` shows `NSDocumentsFolderUsageDescription` = `p156app reads and writes files in your Documents folder.`
- [x] Linux app mode, if 2228 or 2227 is reachable: run a
      `scripts/test-appimage.sh --libc both` build of the same scratch project.
      `appDataPath()` must be the `$HOME`-derived path, not anything under the
      AppImage mount. If neither box is reachable, record that here as not run,
      with the probe output. — not run: neither 2228 nor 2227 is reachable (see the re-probe line above), and the AppImage cannot be emulated (`.ai/compiler.md`). The Linux `--app` base rule (`resource_base_offset`) is unchanged by plan-156, and the new members do not branch on build mode.

Acceptance: every reachable box shows identical fixture output, and the app-mode
paths match the table.
  Check: the commands above (est. 30 min; ship+run per box. Nothing smaller
  executes the per-OS lookups).
Commit: —

### Phase D3: `.ai` notes, examples, archive

- [x] `.ai/arch-abi.md`: add the known-folder arms to the Win64
      acquisition-window note. Record the GUID-at-`0x20` placement and the
      `CoTaskMemFree`-on-every-path rule (a durable ABI lesson). — done: new sections on raw `GetLastError` in `emit_errno` (the `createDirectories` lesson), known-folder query rules, and the in-place validator's separator set.
- [x] `.ai/resources-packages.md`: search it with `rg -n 'resourcePath|resource base'`
      and update any mention to `appResourcePath`. — moot: `rg -n 'resourcePath|resource base|appResourcePath' .ai/resources-packages.md` → no matches.
- [x] Look for a natural use in the examples. The measured candidates are
      `rg -ln 'fs::writeText|save' examples/*/src`. Adopt `appDataPath` only
      where an example already persists user data to a working-directory path
      (a real bug for an installed app). If there is none, record "none". — none: `rg -ln 'fs::writeText|save' examples/*/src` → no matches, so no example persists user data to adopt `appDataPath`.
- [x] Render the man pages with `scripts/man-census.sh --fill os`,
      `scripts/man-run-examples.sh os --run` and
      `scripts/man-census.sh --memory-scope`. That gives 0 unclassified hits,
      and all five pages render. — `man-census.sh --fill os` → `TOTAL 24 24 24 24 13/13`, `pages with neither Description nor Examples: 0`; `--memory-scope os fs` → `unclassified memory-vocabulary hits: 0`; `man-run-examples.sh os --run` → `27 … failed: 0`; `fs --run` → `98 … failed: 0` (after each run, the example-created `~/Library/{Application Support,Caches}/man_examples` was removed, per plan-156-B B-6).
- [ ] Move `plan-156-{A,B,C,D}-*.md` to `planning/completed/`.

Acceptance: the man gates are green and the plans are archived.
  Check: the man commands above (est. 5 min), and `ls planning/plan-156-*` → no
  matches.
Commit: —

## Validation Plan

- **Tests:** everything A through C added, run once together in D1.
- **Coverage check:** D1's artifact gate includes the `os` fixture with all five
  calls, so the new helpers are inside the `.ncodesum` denominator on five
  targets.
- **Runtime proof:** D2.
- **Doc sync:** D3, plus a final `cargo test --bin mfb spec` and
  `scripts/spec-census.sh --citations` inside D1's `cargo test`.
- **Final gate:** D1 (the one full-suite run for plan-156).

## Open Decisions

- None.

## Corrections

- **D-1 (user direction, 2026-09-24): the final gate is the `os::` tests only,
  not the full suite.** The user said: "you dont have to run the full test suite
  for this. Only the os:: tests are needed for the final gate". D1's full
  `cargo test` + `test-accept.sh` + `artifact-gate.sh` run becomes a scoped one:
  - `cargo test --bin mfb os`;
  - the `os` acceptance fixtures (`scripts/test-accept.sh … 'func_os_*' 'os-*'`,
    plus the other fixtures this plan touched: `self-update-grow-valid`,
    `libsnd-playback-rt`, and the `os` byte-identity fixture);
  - the `os` byte-identity sums.

  Golden regeneration stays limited to the expected list.

## Summary

This sub-plan has no new design. The risk is misclassifying a red golden, and
the fixed expected list plus the ONE-fixture inspection rule guards against it.
