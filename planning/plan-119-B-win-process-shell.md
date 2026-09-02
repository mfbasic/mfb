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
| plan-119-A landed (`emit_win_spawn_tail` exists) | `grep -n emit_win_spawn_tail src/codegen/builtins/process/gen_windows.rs` | NOT MET (A pending) |

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
- UNVERIFIED: a line *starting* with a double quote under the `/S /C "…"`
  wrap (the reason `/S` is chosen). Phase 1 adds this exact case to the box
  script before the helper is considered done.

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

- [ ] `func_shell.rs`: implement `lower_process_shell_helper_win` per §3.
- [ ] `src/target/win_x86_64/mod.rs`: add `"process.shell"` to the capability
      list.
- [ ] Extend `scripts/test-winprocess.sh` with the shell matrix: sequencing,
      `exit 7`, pipeline, redirect+type, stdin→`sort`, AND a quote-leading
      line (`"quoted prog" style`) pinning the `/S` choice.
- [ ] `tests/cli_process_windows_build.rs`: a shell program's nplan imports
      the same Win32 set (CreateProcessA etc.).
- [ ] Check `tests/rt_trapped_call_capability_gate.rs`: if its vehicle uses
      `process.shell`, re-point that case to a still-absent capability
      (`process.spawnEnv` until C lands, `os.resourcePath` after) — the test
      guards the GATE mechanics, not shell specifically (its own header says
      so); do not weaken its assertions.

Acceptance: `scripts/test-winprocess.sh` passes on box 2230 including the
shell matrix; full `cargo test --no-fail-fast` green.
Commit: —

### Phase 2 — docs

- [ ] `func_shell.rs` DESC/EX: delete the Unix-only paragraph; state the
      per-platform shell (`/bin/sh -c` on Unix, `cmd.exe /S /C` on Windows),
      the dialect caveat, and the CRLF note for `receive` on Windows
      children; keep the injection warning. Verify with `mfb man process
      shell` + `scripts/man-census.sh --memory-scope` +
      `scripts/man-run-examples.sh process` (examples must stay
      Unix-runnable on the host or be dialect-neutral like `sort`).
- [ ] `mod.rs` MODULE_DESC: extend "(`/bin/sh -c` on Unix)" with the Windows
      shell.
- [ ] `planning/todo.md`: drop/adjust the "Two undocumented Windows limits"
      note's shell half (coordinate with the uncommitted main-checkout edit).

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

*(fill during execution)*

## Summary

A ~40-line prologue over plan-119-A's proven tail, with every behavioral
claim already demonstrated on the real box before a line of the helper is
written; the residual risk is the `/S` quote-edge, pinned by its own box case.
