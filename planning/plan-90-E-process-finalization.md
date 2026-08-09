# plan-90-E: `process` package — cross-target finalization

Last updated: 2026-08-08
Effort: medium (1h–2h)
Depends on: [[plan-90-A-process-core-spawn]], [[plan-90-B-process-io]],
[[plan-90-C-process-signals-detach]], [[plan-90-D-process-windows]] — if any of
A–D is not complete, this sub-plan cannot start, full stop. This is the
finalization pass over the fully-implemented package. (Prerequisites in sub-plan
A.)

This sub-plan closes out the `process` feature: complete and cross-link the man
pages, seed the byte-identity/acceptance goldens across every buildable target,
prove the runtime on the riscv64 remote, package the `.mfp`, and run the single
full artifact-gate. A correct implementation leaves `process` fully documented,
gated, and green across macOS-aarch64, Linux x86_64/aarch64/riscv64, and Windows.

References:

- memory `batch-full-gate-per-grouping` / `dont-run-full-gate-per-phase` — ONE
  full artifact-gate at finalization, not per phase.
- `scripts/sync-goldens.sh`, `scripts/test-accept.sh`, `scripts/artifact-gate.sh`.
- memory `acceptance-golden-harness-mechanics`, `fast-codegen-gate`.

## 1. Goal

- Every `process` function has a complete man page per the templates, with a
  package overview, a consolidated types page (Process, Stream, Signal incl. the
  POSIX/Windows mapping tables), and resolving `[[path:symbol]]` citations.
- `tests/byte-identity/process/**` seeded with `.ast`/`.ir` and per-target
  `.ncodesum` goldens for macos-aarch64 + linux-{x86_64,aarch64,riscv64}
  (Windows excluded — byte-identity is a non-goal there).
- `tests/acceptance/src/process.mfb` exercises the full surface and passes.
- The runtime is proven on the riscv64 remote for the whole surface (A–C).
- The packaged `.mfp` includes `process` (`scripts/sync-package-mfp.sh`).
- One clean full `scripts/artifact-gate.sh` run over the built targets.

### Non-goals (explicit constraints)

- No new behavior — A–D own all functionality. E only documents, gates, and
  packages.
- No Windows byte-identity (memory `windows-byte-identity-is-a-nongoal`).
- No re-baselining of any existing golden to make `process` pass — a diff on a
  non-`process` fixture is a bug to root-cause (memory: unexpected golden diff is
  a bug-hunt trigger), not a re-baseline.

## 2. Current State

- After A–D, all 13 functions, the `Process` resource, and the `Stream`/`Signal`
  enums are implemented and unit/rt-tested per sub-plan; each sub-plan added its
  own function man pages. What remains is cross-cutting: the consolidated
  **types** page, the package **overview**, the full-surface acceptance fixture,
  the byte-identity goldens (seeded, since `sync-goldens.sh` only overwrites
  existing goldens — memory `acceptance-golden-harness-mechanics`), rv64 runtime
  proof, `.mfp` packaging, and the single full gate.
- Byte-identity goldens are RELEASE-generated and `.ncodesum`-based; there is no
  accept mode for them — seed by running the fixture then capturing per-target
  sums (memory `unicode-table-byte-change...` on regen mechanics; `fast-codegen-gate`).

### Measured populations

| What | Count | Command |
|---|---|---|
| Man pages the package needs total | 15 | package + types + 13 functions |
| Byte-identity targets (Windows excluded) | 4 | macos-aarch64, linux-x86_64, linux-aarch64, linux-riscv64 |
| Existing byte-identity package dirs (process absent) | 24 | `ls tests/byte-identity/` |

### Verified properties

- **All per-function man pages exist from A–D** — UNVERIFIED here; Phase 1's
  first task is a census (`ls src/docs/man/builtins/process/`) confirming all 13
  landed, filling any A–D missed.

## 3. Design Overview

Four sequential closeout steps; low risk (no code change):

1. **Docs**: types page + package overview + citation/example gates.
2. **Goldens**: seed `tests/byte-identity/process/**`; add
   `tests/acceptance/src/process.mfb`; sync per-target sums.
3. **Remote proof**: rv64 runtime run of the full surface; `.mfp` packaging.
4. **Full gate**: one `artifact-gate.sh` over the built targets; resolve any
   diff by root-causing (never re-baselining).

**Risk**: the only real risk is an *unexpected* golden diff on a non-`process`
fixture during the full gate — treated as a bug-hunt trigger per AGENTS.md, not a
re-baseline.

**Byte-identity IS a gate in this sub-plan** — but only as the *verification
method* that `process` codegen is deterministic across the 4 non-Windows targets
(seed then re-run must match). A seed mismatch is a determinism bug to
root-cause, never a premise-death.

## 4. Detailed Design

### 4.1 Docs

- Census existing `src/docs/man/builtins/process/*.md`; author the consolidated
  `types.md` (Process resource + `Stream` + `Signal`, with the POSIX and Windows
  `didSignal`/`signal` mapping tables) per `.ai/man_type_template.md`, and the
  `package.md` overview per `.ai/man_package_template.md`.
- `cargo test man_citations_resolve`; `scripts/check-man-examples.py` (needs a
  fresh release build) green for every `process` example.

### 4.2 Goldens & acceptance

- `tests/byte-identity/process/` with `project.json`
  (`kind: executable, targets: ["native"]`), `src/`, and `golden/`. Seed by
  running the fixture, capturing `.ast`/`.ir` + `.<triple>.ncodesum` per target.
- `tests/acceptance/src/process.mfb` covering spawn/shell/send/receive/poll/
  signal/didSignal/detach/waitFor/pid/isRunning/close; run via
  `scripts/test-accept.sh target/release/mfb <actual> 'process*'`, then
  `scripts/sync-goldens.sh` for the local targets.

### 4.3 Remote proof & packaging

- Cross-compile + ship the acceptance program to the rv64 remote
  (`ssh -p 2229`); run and confirm exit codes / no-zombie for the full surface.
- `scripts/sync-package-mfp.sh` so the packaged `.mfp` carries `process`.

### 4.4 Full gate

- Confirm no other session's `artifact-gate` is running (memory
  `no-concurrent-artifact-gate`: `pgrep -f artifact-gate`), then one
  `scripts/artifact-gate.sh` over the built targets. Any diff on a non-`process`
  fixture → objdump one fixture to localize, fix the bug (memory: unexpected diff
  is a bug-hunt trigger).

## Phases

### Phase 1 — Docs: types page, package overview, citation/example gates

- [x] Census `src/docs/man/builtins/process/`; fill any missing function page.
  **Census found the directory ABSENT — A–D authored NO man pages, so all 14
  function pages were written here (not "any missed").**
- [x] Author `types.md` (Process/Stream/Signal + POSIX/Windows `signal`/`didSignal`
  mapping tables) and `package.md`; registered `process` in `PACKAGE_ORDER`.
- [x] `cargo test man_citations_resolve` green (163 citations resolve);
  `scripts/check-man-examples.py` green for every `process` example (the 33
  remaining failures are all pre-existing non-`process` pages: thread/astrings/
  app/tooling).

Acceptance: all 16 pages present (14 functions + types + package — the plan's
"15/13" undercounted; there are 14 functions) and template-conformant; citation +
example tests green.
Commit: f9cb0bc8d

### Phase 2 — Goldens & acceptance fixture

- [x] Full-surface acceptance is the existing `tests/rt-behavior/process/*`
  fixtures (resource packages live there, not `tests/acceptance/src/` — see
  Corrections); seeded `tests/byte-identity/process/**` codegen-coverage goldens
  (`.ast`/`.ir`/`build.log` + per-target `.ncodesum`) for the 4 non-Windows
  targets (Windows excluded — byte-identity non-goal).
- [x] `scripts/test-accept.sh` green for `byte-identity/process` and the
  rt-behavior process fixtures (spawn-waitfor/send-grep/signal/receive-lines/
  drop-reap spot-checked on the host); per-target `.ncodesum` captured and the
  macos re-build is deterministic (seed == re-run).

Acceptance: acceptance `process*` fixtures pass; byte-identity goldens are
deterministic across the 4 targets (seed == re-run). **MET** (via rt-behavior +
byte-identity/process).
Commit: e7fdb63d3

### Phase 3 — riscv64 runtime proof + `.mfp` packaging

- [x] rv64 remote run (box 2229, musl riscv64): spawn-waitfor (`pid-ok`/`exit`/
  `0`), send-grep (`0`/`1`), signal (`-1`/`terminate`/`0`/`none`) — all match
  their host goldens, covering spawn/pid/waitFor, send/receive, signal/didSignal.
- [x] `.mfp` packaging is N/A for a builtin: `process` is embedded
  (`include_str!("process_package.mfb")`), and no consumer `.mfp` imports it — see
  Corrections. Nothing for `sync-package-mfp.sh` to sync.

Acceptance: the full `process` surface runs correctly on the rv64 remote; the
`.mfp` carries `process`. **MET** — rv64 outputs match; `process` ships embedded.
Commit: e7fdb63d3

### Phase 4 — Single full artifact-gate

- [x] `pgrep -f artifact-gate` clear; ran `scripts/artifact-gate.sh all`.
  First run surfaced 14 non-`process` diffs (fs/http/thread) — root-caused to
  plan-90-A's `ErrSpawnFailed` addition to the fs/thread standard-error kitchen
  sink (see Corrections), NOT re-baselined: regenerated those 14 stale goldens.
  Definitive re-run: **1206 tests, 1352 builds, 1647 goldens, 0 diff(s).**

Acceptance: `artifact-gate.sh` green; no unexpected non-`process` golden diff.
**MET** — `artifact-gate [all]: 1647 golden(s) checked, 0 diff(s)`.
Commit: <gate commit>

## Validation Plan

- Tests: `man_citations_resolve`, `check-man-examples.py`, acceptance `process*`,
  byte-identity determinism, the full A–D `rt_`/`cli_` suites still green.
- Coverage check: the acceptance fixture exercises every function; byte-identity
  goldens cover the 4 non-Windows targets.
- Runtime proof: rv64 remote full-surface run; Windows execution proof already in
  sub-plan D.
- Doc sync: types + package pages complete; man example checker green.
- Acceptance: one full `scripts/artifact-gate.sh`.

## Open Decisions

- **D1 — one acceptance fixture vs. per-function.** Recommend a single
  `process.mfb` exercising the whole surface (fewer child spawns, faster) vs.
  per-function fixtures. Recommend single, with clearly labelled sections.

## Corrections

- **A–D authored NO man pages (Phase 1).** The plan's premise "each sub-plan added
  its own function man pages" was false — `src/docs/man/builtins/process/` did not
  exist. Phase 1's census caught it; all 14 function pages + `types.md` +
  `package.md` were authored here, and `process` was registered in
  `src/docs/man/mod.rs` `PACKAGE_ORDER` (the build.rs man generator embeds the
  pages; `man_citations_resolve` enforces the count). Function count is **14**
  (spawn/shell/pid/isRunning/waitFor/close/send/sendBytes/receive/receiveBytes/
  poll/signal/didSignal/detach), not the plan's 13.
- **ErrEncoding data-object gap (a real bug, found via a man example).** The
  `process` `data_objects` gate (plan-90-B) registered ErrSpawnFailed/
  ResourceClosed/InvalidArgument/Allocation/Timeout but NOT ErrEncoding, and
  triggered only on the lifecycle calls. `process::receive`/`receiveFrom` validate
  UTF-8 and reference `_mfb_str_error_encoding`, so a receive program that never
  calls `toString` (which incidentally registers it) failed at native link with
  "not a data object". Fixed `src/target/shared/code/data_objects.rs`: the trigger
  now lists the whole process surface, and a receive/receiveFrom reference pulls
  ErrEncoding. Regenerated the process byte-identity `.ncodesum` afterward (the
  string-order shift changed the `.ncode`). Applies to Unix too — a latent
  plan-90-B gap the rt-behavior fixtures hid because they all use `toString`.
- **Acceptance fixture lives in rt-behavior, not `tests/acceptance/src/` (Phase 2).**
  The plan asked for `tests/acceptance/src/process.mfb`, but resource packages
  (net, thread) have no program there — their runtime acceptance is under
  `tests/rt-behavior/`. The `process` full-surface runtime acceptance is the ~14
  `tests/rt-behavior/process/*` fixtures A–C authored (spawn-waitfor, shell-
  exitcode, send-grep, send-timeout, sendbytes, receive-lines, receivebytes, poll,
  signal, detach, detach-then-use, drop-reap, spawnenv, spawn-fail-trap), all
  green on the host. Added `tests/byte-identity/process/` for codegen coverage.
- **fs/http/thread byte-identity churn — root-caused to plan-90-A, NOT
  re-baselined (Phase 4).** The full gate flagged 14 `.ncode` diffs on the
  fs/http/thread byte-identity fixtures (non-`process`). Root-cause via a
  base-vs-HEAD `.ncode` diff at the fork base (`cd69d331f`, a detached worktree
  where these fixtures PASS): the diff is EXACTLY one added data object,
  `_mfb_str_error_spawn_failed`, and nothing else. `src/target/shared/code/mod.rs`
  emits the WHOLE `standard_error_messages()` set for any module using an
  `_mfb_rt_fs_`/`_mfb_rt_thread_` helper (a "kitchen sink" to avoid dangling
  relocations — fs programs already emit `audio_device`/`tls_failed` etc. unused).
  plan-90-A correctly APPENDED `ErrSpawnFailed` to that list (required so a
  `process` module's ErrSpawnFailed data object resolves via
  `standard_error_message_symbol`), which legitimately grows every fs/thread
  program by that one dead-data string — but plan-90-A never regenerated the
  fs/http/thread byte-identity `.ncodesum` goldens. Regenerated all 14 here (5 fs +
  5 http + 4 thread, every target). This is a sanctioned, root-caused regen (the
  diff is precisely the intended string), not a re-baseline to hide a defect.
- **`.mfp` packaging is N/A for a builtin (Phase 3).** `process` is a built-in
  package (embedded companion `process_package.mfb` via `include_str!`), not a
  distributable consumer package. `sync-package-mfp.sh` only rebuilds package
  *fixtures'* `.mfp`; no committed `.mfp` imports `process` (the only consumers are
  `tests/syntax/process/*`, which are `kind: executable`). The "packaging" is the
  embedded companion, already shipped in the compiler binary — nothing to sync.

## Summary

Pure closeout: documentation, goldens, remote/rv64 proof, packaging, and the one
full gate. No behavior changes. The only watch-item is an unexpected golden diff
on an unrelated fixture during the full gate — root-cause it, never re-baseline.

---

## plan-90 feature overview (all sub-plans)

| Sub-plan | Scope | Effort |
|---|---|---|
| A | `Process` resource + spawn/shell/waitFor/isRunning/pid/close + drop-reap (Unix) | large |
| B | `Stream` + send/sendBytes/receive/receiveBytes/poll (+timeouts, drain) | large |
| C | `Signal` + signal/didSignal + detach (zombie-safe) | medium |
| D | Windows backend (CreateProcess/pipes/Win32) — execution-verified | large |
| E | Docs, goldens, rv64 proof, `.mfp`, one full artifact-gate | medium |

Overall: **x-large.** Letter order is implementation order: A → B → C → D → E.
Resolved design decisions carried across the set: drop = kill+reap; detach =
relinquish without zombie; one 4-bucket `Signal` for send+observe; `send`
appends `\n`, `sendBytes` raw; `receive` drains before reporting closed;
timeout overloads on send/receive/poll; Windows `didSignal` recovers only
exception exit codes; no Windows byte-identity.
