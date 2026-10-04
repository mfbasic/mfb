# bug-799: app-mode `io::write`/`io::print` fd fallback silently drops a failed or partial write

Last updated: 2026-10-03
Effort: medium (1h–2h, plus remote-box verification for GTK and Windows) (guess)
Severity: MEDIUM
Class: Correctness (write failure swallowed; output silently lost)

Status: Fixed (2026-10-03, bug-staging-5)
Regression Test: `tests/runtime/rt_app_fd_write_failure.rs` (macOS); Windows and Linux GTK verified on the boxes (below)

In an `--app` build with no window attached, `io::write`, `io::print` and their
stderr forms fall back to writing the bytes straight to fd 1/2. This happens on the
headless test path (`MFB_MACAPP_HEADLESS`), before the window exists, and on Linux
when the transcript chunk allocation fails. The fallback issues one `write()`
(`WriteFile` on Windows) and **ignores its result**:

- a failed write (closed or read-only stdout, `EIO`, a full disk) returns `Ok`;
- a partial write (pipe or terminal back-pressure) drops the unwritten tail;
- an `EINTR` is never retried.

The console build of the same call reports every one of these: it loops on partial
writes, retries `EINTR`, and raises `ErrWriteFailed` (`77020002`). So the same
program loses output only when built `--app`.

**The single correct behavior a fix produces:** an app-mode fd-fallback write
writes every byte or raises `ErrWriteFailed`, exactly as the console write family
does. Stdout `EPIPE` behaves as the console path does too: it re-raises SIGPIPE
(bug-467).

Found by bug-723's blast-radius audit (its Resolution section).

## Reproduction (macOS, headless app)

```basic
IMPORT io

FUNC main() AS Nothing
  io::write("lost") TRAP(e)
    io::printError("write " & toString(e.code))
    RECOVER
  END TRAP
  io::printError("done")
END FUNC
```

Build with `--app`, run with `MFB_MACAPP_HEADLESS=1` and stdout bound to a
read-only descriptor (`./prog.out 1<project.json`). Expected stderr:
`[stderr] write 77020002` / `[stderr] done`, or the console wording if stderr also
takes the fd path. Observed by code reading: no `write` line, because the result
is never checked. Phase 1 confirms this by running it.

## Root cause

| Backend | Site | Defect |
|---|---|---|
| macOS AArch64 | `src/target/macos_aarch64/app/app_io.rs:emit_app_io_write`, label `fd_path` | `_write` result ignored; no partial loop |
| Linux GTK | `src/target/linux_gtk/app_io.rs:emit_app_io_write`, label `fd_path` | `write` result ignored; no partial loop |
| Windows x86-64 | `src/target/win_x86_64/app/mod.rs:emit_app_io_write`, headless `WriteFile` | `WriteFile` BOOL and `written` count ignored |

The console write family (`src/codegen/builtins/io/gen_write_family.rs`, with
`emit_eintr_retry_or_error_epipe` from `src/codegen/io/stdout/io_stdout.rs`)
already has the full loop, retry and raise. The fix should route the fallback
through it rather than adding a fourth copy.

## Fix

- Phase 1: a RED runtime test for the macOS headless case (run locally), plus
  equivalent rows for the GTK and Windows app runners on the remote boxes
  (`.ai/remote_systems.md`).
- Phase 2: have each backend's fd fallback call the shared console write loop
  (partial-write loop, `EINTR` retry, `EPIPE` → SIGPIPE, otherwise
  `ErrWriteFailed`), and give the fallback's caller a result-tag/raise exit. Update
  `mfb spec app console-io` (its fallback column says `write(1, bytes, len)`).
- Phase 3: app-mode acceptance fixtures and the per-target `.ncode` goldens
  (expected to change only in the `io` app-write bodies).

## Resolution (2026-10-03)

The console write loop is now one emitter,
`gen_write_family::emit_fd_write_all` (partial-write loop, `EINTR` retry,
stdout `EPIPE` → SIGPIPE, else `ErrWriteFailed`), extracted verbatim from
`lower_write_family`. The console path is byte-identical (`artifact-gate io`: 7
goldens, 0 diffs). Each app backend's fd fallback calls it:

- macOS: `app_io.rs:emit_app_io_write` `fd_path`; a failed write keeps its error
  result across the autorelease-pool pop.
- Linux GTK: `linux_gtk/app_io.rs:emit_app_io_write` `fd_path`.
- Windows: `win_x86_64/app/mod.rs:emit_app_io_write` `std_path` (through the
  platform's `WriteFile` `emit_write`; no `EPIPE` re-raise on Windows).

Runs of the reproduction (stdout bound to a read-only descriptor):

| target | main | fixed |
| --- | --- | --- |
| macos-aarch64 (`MFB_MACAPP_HEADLESS`) | `done` only | `write 77020002`, `done` |
| windows-x86_64, box 206 (`MFB_WINAPP_HEADLESS`, `1<file`) | `done` only | `write 77020002`, `done` |
| linux-aarch64 GTK, box 2226 (`MFB_GTKAPP_HEADLESS`, glibc AppImage) | `done` only | `write 77020002`, `done` |

Normal output still arrives on all three. macOS headless stdout to a closed pipe
now dies by SIGPIPE (exit -13), as the console build does. The app-mode `.ncode`
goldens change in the io write bodies only (localized at the batch gate). Spec:
`app/03_console-io.md`, `app/02_linux-runtime.md`.
