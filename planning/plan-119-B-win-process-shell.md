# plan-119-B: `process::shell` on Windows via cmd.exe

Last updated: 2026-09-01
Effort: medium (1h–2h)
Depends on: plan-119-A (uses its `emit_win_spawn_tail`; its quoting work is NOT involved — shell passes the line through un-quoted by design)

Implement `process::shell` for `windows-x86_64`: run the command line through
`cmd.exe /S /C`, wired to the same three pipes and `Process` record as spawn.
Today it is compile-time rejected (capability absent from
`src/target/win_x86_64/mod.rs:289-307`; entry fn is
`unimplemented_on_windows` at `func_shell.rs:233-240`), and the man page says
"Unix-only" (`func_shell.rs:34-38`).

**Feasibility is box-proven** (spike, box 2230, 2026-09-01, via 1-arg spawn
building the identical command lines): `cmd.exe` resolves with
`lpApplicationName = NULL`; `/C` runs sequencing (`&`), pipelines (`| sort`),
redirection (`> f` … `type f`); the child's exit code propagates
(`cmd /C exit 7` → `waitFor` = 7); and stdin streams through cmd into a
filter (`send`×2 + `close` + `sort` → sorted lines back). The Windows shell
body is therefore exactly: build `cmd.exe /S /C "<line>"` into the `CMD`
slot, then call `emit_win_spawn_tail` with no env/cwd.

References:

- plan-119-A (tail helper, frame constants, box scripting).
- `src/codegen/builtins/process/func_shell.rs` — descriptor, posix body
  (`lower_process_shell_helper_posix` builds `sh -c` argv then
  `emit_spawn_tail`; the win body mirrors that shape), DESC to rewrite.
- `src/codegen/builtins/process/mod.rs:101-104` — MODULE_DESC's
  "(`/bin/sh -c` on Unix)" phrasing to extend.
- `tests/rt_trapped_call_capability_gate.rs:22-24` — uses the
  shell/spawnEnv/`os.resourcePath` capability gaps as its test vehicle.
- `.ai/man-content.md` — man-page standard for the DESC/EX rewrite.

## Prerequisites

Family gate in plan-119-A, plus:

| Must be true | Command | Status |
|---|---|---|
| plan-119-A landed (`emit_win_spawn_tail` exists) | `grep -n emit_win_spawn_tail src/codegen/builtins/process/gen_windows.rs` | MET — `gen_windows.rs:196` (commits 407ea3bdf, 0be618f08) |

## 1. Goal

- A program calling `process::shell("echo hello | sort")` builds for
  `windows-x86_64` and, on box 2230, behaves as on Unix modulo shell dialect:
  pipeline output readable via `receive`, exit code via `waitFor`, stdin via
  `send`/`close`. `mfb man process shell` no longer says Unix-only.

### Non-goals (explicit constraints)

- The Unix body and semantics are untouched.
- No shell-dialect translation: the string is handed to `cmd.exe` verbatim —
  callers write platform-appropriate command lines (the doc says which shell
  interprets it per platform; that's the whole contract).
- No quoting/escaping of the user's line beyond the `/S /C "…"` wrap — a
  shell command line is *supposed* to be interpreted.
- CRLF behavior of `receive` (keeps `\r`, plan-119-A §2) is unchanged; the
  page examples must not pretend otherwise.

## 2. Current State

- `lower_shell` (`func_shell.rs:80-97`) already branches
  `PlatformFamily::Windows` → `lower_process_shell_helper_win` — only the
  helper body is missing.
- The posix helper builds argv `["/bin/sh", "-c", cmd]` with `emit_cstring_literal`
  + a byte-copy of the MFB `String`, then `emit_spawn_tail` (read
  `func_shell.rs:150-232`).
- Capability lists carry `process.shell` for linux (`linux_common/mod.rs:257`)
  and macos (`macos_aarch64/mod.rs:254`) only.

### Measured populations

| What | Count | Command |
|---|---|---|
| Spike proof lines | `rc-b=7`, `p2=[alpha]` (piped `sort`), `s1=[apple]`/`s2=[banana]` (stdin filter), `d1=[filed]` (redirect+type) | box 2230 run, 2026-09-01 |
| DESC "Unix-only" paragraph | `func_shell.rs:34-38` | read |
| Gate-test dependence | vehicle names shell+spawnEnv+os.resourcePath | `tests/rt_trapped_call_capability_gate.rs:22-24` |

### Verified properties

- `cmd.exe` resolves without a path via `CreateProcessA(NULL, "cmd.exe /C …")`
  — box-proven (no `COMSPEC` reading needed).
- VERIFIED (box 2230, via the implemented helper, `scripts/test-winprocess.sh`):
  a line *starting* with a double quote under the `/S /C "…"` wrap — the reason
  `/S` is chosen. `shell("\"cmd\" /C echo quoted")` prints `quoted`
  (`sh7:quoted`, `sh7:rc=0`), so `/S` strips exactly the outer wrap and runs the
  quoted program name. A line with quotes in the MIDDLE was added alongside it:
  `shell("echo \"a b\"")` prints `"a b"` — cmd's `echo` is literal, so the
  inner quotes came back untouched, which is the assertion that the wrap did not
  swallow or double them.
- The full Phase 1 shell matrix on box 2230, through the shell surface itself:
  `sh1:one`/`sh1:two` (sequencing), `sh2:rc=7` (exit code), `sh4:shelled`
  (redirect + `type`), `sh5:apple`/`sh5:banana` (`| sort` pipeline),
  `sh6:apple`/`sh6:banana` (stdin streamed into `sort`). 37/37 assertions ok
  across the whole script.

## 3. Design Overview

`lower_process_shell_helper_win`: allocate `len(cmd) + len("cmd.exe /S /C \"")
+ 2` bytes; emit the literal prefix `cmd.exe /S /C "` (via the
`emit_cstring_literal`-style byte stores), byte-copy the MFB `String` payload,
append closing `"` + NUL into the `CMD` slot; then `emit_win_spawn_tail(CMD,
env=None, cwd=None)`. Error paths: `ErrOutOfMemory` (alloc) and
`ErrSpawnFailed` (CreateProcess FALSE) — the same two the posix body raises.

`/S /C "…"` rather than bare `/C …`: with `/S`, cmd strips exactly the outer
quotes and treats everything inside as the command, which makes
quote-containing and quote-leading lines behave predictably (cmd's legacy
heuristics otherwise re-guess). The spike ran bare `/C` successfully for
plain lines; `/S` + wrap is the strictly-more-defined superset.

Risk is low: one new prologue over a proven tail. Byte-identity is NOT the
gate (new emission only for programs that previously failed to build; no
existing golden churns unless a fixture starts using shell). Gates:
box-run behavior + compile tests.

Rejected: `%COMSPEC%` lookup (an env read + fallback for zero observed need;
box-proven unnecessary — record as a future hardening note in the DESC if
ever reported); translating to PowerShell (different, bigger contract).

## Phases

### Phase 1 — helper + capability + proof

- [x] `func_shell.rs`: implement `lower_process_shell_helper_win` per §3.
- [x] `src/target/win_x86_64/mod.rs`: add `"process.shell"` to the capability
      list.
- [x] Extend `scripts/test-winprocess.sh` with the shell matrix: sequencing,
      `exit 7`, pipeline, redirect+type, stdin→`sort`, AND a quote-leading
      line (`"quoted prog" style`) pinning the `/S` choice. sh1–sh8; the box
      transcript is in §2 Verified properties.
- [x] `tests/cli_process_windows_build.rs`: a shell program's nplan imports
      the same Win32 set (CreateProcessA etc.).
- [x] Check `tests/rt_trapped_call_capability_gate.rs`: if its vehicle uses
      `process.shell`, re-point that case to a still-absent capability
      (`process.spawnEnv` until C lands, `os.resourcePath` after) — the test
      guards the GATE mechanics, not shell specifically (its own header says
      so); do not weaken its assertions. Re-pointed **straight to
      `os.resourcePath`**: `process.spawnEnv` turned out not to be a valid
      vehicle at all (see Corrections). Assertions unweakened; the module
      header now records why, so the next re-point does not repeat the trip.

Acceptance: `scripts/test-winprocess.sh` passes on box 2230 including the
shell matrix; full `cargo test --no-fail-fast` green.
Commit: —

### Phase 2 — docs

- [x] `func_shell.rs` DESC/EX: delete the Unix-only paragraph; state the
      per-platform shell (`/bin/sh -c` on Unix, `cmd.exe /S /C` on Windows),
      the dialect caveat, and the CRLF note for `receive` on Windows
      children; keep the injection warning. Verify with `mfb man process
      shell` + `scripts/man-census.sh --memory-scope` +
      `scripts/man-run-examples.sh process` (examples must stay
      Unix-runnable on the host or be dialect-neutral like `sort`). Both
      examples were made dialect-neutral rather than merely host-runnable
      (`tr a-z A-Z` → `sort`, `true` → `exit 0`), so the page's own claim that
      the two shells agree on simple lines is demonstrated by its examples.
      `man-census.sh --memory-scope` unchanged at 8 unclassified hits (all
      pre-existing, all `canvas`); `man-run-examples.sh process --run` 18/18.
- [x] `mod.rs` MODULE_DESC: extend "(`/bin/sh -c` on Unix)" with the Windows
      shell.
- [x] ~~`planning/todo.md`: drop/adjust the "Two undocumented Windows limits"
      note's shell half (coordinate with the uncommitted main-checkout
      edit).~~ — moot: the note is not on this branch and never was.
      `git show main:planning/todo.md | grep -c -i "undocumented Windows
      limits"` → 0. It exists **only** as an uncommitted edit in the shared
      main checkout (`planning/todo.md` is `M` there), i.e. a peer session's
      in-progress work, which this plan must not touch. Recorded in
      Corrections and surfaced in the final report instead.

Acceptance: rendered pages show the new wording; man gates green; full suite
re-run green.
Commit: —

## Validation Plan

- Tests: compile-level (`cli_process_windows_build.rs`), runtime
  (`scripts/test-winprocess.sh` on 2230 — the only place a PE actually runs),
  gate-test re-point verified by running it against a windows target.
- Doc sync: Phase 2 list; `.ai/man-content.md` rules apply.
- Acceptance: family-standard (full cargo test, test-accept, artifact-gate,
  fmt, check --all-targets).

## Open Decisions

- None — the one design fork (`/S /C` wrap vs bare `/C`) is decided in §3
  with the quote-leading box case as its regression pin.

## Corrections

- **`process.spawnEnv` is NOT a valid capability-gate vehicle, so the gate test
  was re-pointed straight to `os.resourcePath`.** The plan said to park it on
  `process.spawnEnv` until C landed. Doing that turned the test red, and the
  failure is the interesting part: it did not fail on `!bare_ok` (the build does
  fail) but on the assertion that the failure is a *capability* rejection. The
  actual message is

      error: native code internal relocation target
             '_mfb_rt_process_process_spawnEnv' is not defined

  `validate_capabilities` sees the base call `process.spawn`, which Windows
  advertises, so the four-argument overload's alias never faces the gate at all
  and dies at link time instead. The plan's own §1 for letter C inherits this:
  its "Today it is compile-time rejected (capability absent)" is wrong for the
  same reason — corrected there. This letter therefore skips the intermediate
  hop and points the vehicle at `os.resourcePath`, which Windows genuinely does
  not advertise (`grep -n 'os.resourcePath' src/target/win_x86_64/mod.rs` → no
  match; macOS has it at `macos_aarch64/mod.rs:98`). The premise assertion — the
  thing that caught this — is unweakened, and the module header now records the
  trap so the next re-point does not repeat it.
- **One box assertion was written wrong and corrected, not the code.**
  `shell("echo \"a b\"")` was expected to print `a b`; it prints `"a b"`,
  because cmd's `echo` is literal and does not strip quotes. The observed output
  is the *stronger* evidence — it shows the inner quotes survived the outer
  `/S /C "…"` wrap intact — so the expectation was corrected to match cmd's real
  behavior and the comment explains why that is the assertion worth making.
- **The `planning/todo.md` task had no target on this branch.** Both this letter
  and letter C were to edit a "Two undocumented Windows limits" note. That note
  is absent from committed `main` (`git show main:planning/todo.md | grep -c -i
  "undocumented Windows limits"` → 0) and present only in the shared main
  checkout's *uncommitted* `planning/todo.md` — a peer session's work in
  progress. Editing it from this plan would entangle someone else's uncommitted
  changes, so both tasks are marked moot with that evidence and the staleness is
  reported at the end for whoever owns that edit. Its claim ("`process::shell`
  and the four-argument `process::spawn` are Unix-only and fail the Windows
  build at compile time") is false in both halves once plan-119 lands.

## Summary

A ~40-line prologue over plan-119-A's proven tail, with every behavioral
claim already demonstrated on the real box before a line of the helper is
written; the residual risk is the `/S` quote-edge, pinned by its own box case.
