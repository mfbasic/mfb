# bug-743: `os::args` in an `--app` build crashes the program on macOS and returns an empty list on Linux GTK

## STATUS: FIXED (see the commit that archives this doc)

The worker shims (macOS and GTK) now hand the program entry `argc`/`argv`
whenever the entry reads them — `main` takes an argument list OR the program
calls `os::args`/`os::prog` (`AppEntrySpec.worker_passes_args`, set in
`src/codegen/engine/builder/mod.rs`). The root cause below is confirmed: with
the fix reverted the regression test dies with SIGSEGV on macOS.

- **RED → GREEN:** `tests/cli/cli_entry_args.rs::app_build_os_args_without_a_main_parameter`
  (host `--app` build run headless with `one two`, expects `ARGS 2:one,two`):
  "killed by signal 11 (SIGSEGV)" without the builder change, passes with it.
- **Measured after the fix:** macOS headless and windowed → `2:one,two`;
  Linux GTK aarch64 on box 2226 (AppImage under Xvfb) → `2:one,two`; Windows 11
  `--app` on box 206 → `2:one,two` (Windows was never affected: its entry builds
  argv from `GetCommandLineW`, `defers_arg_capture`).
- **Goldens:** none predicted to move — no app-mode golden fixture calls
  `os::args`/`os::prog` (`grep -rl "os::args\|os::prog" tests/syntax/app` → none).

Last updated: 2026-09-26
Effort: small
Severity: HIGH
Class: Correctness (process crash on macOS; wrong result on Linux)

Status: Fixed (2026-09-26)
Regression Test: `tests/cli/cli_entry_args.rs::app_build_os_args_without_a_main_parameter`

Found by bug-699 Phase 7 while writing the `--app` key probe
(`tools/key-probe`), which had to take its settings from the environment instead
of the command line because of this. Fixed in the same worktree after bug-699
(fixbug rule 12).

## Reproduction

```basic
IMPORT fs
IMPORT os
IMPORT strings

FUNC main AS Integer
  fs::writeText("/tmp/argt743.txt", toString(len(os::args())) & ":" & strings::join(os::args(), ","))
  RETURN 0
END FUNC
```

Build with `mfb build -app` and run the executable with the arguments `one two`.

- **macOS aarch64** (measured 2026-09-26, `main` at `5c4ea72e3`, release build
  in a detached worktree): `MFB_MACAPP_HEADLESS=1 ./build/argt.app/Contents/MacOS/argt one two`
  exits **139 (SIGSEGV)**; a program whose `main` only does
  `LET a AS List OF String = os::args()` / `RETURN len(a)` exits 139 too, while
  `RETURN 0` and a plain `fs::writeText` exit 0. The windowed run (no
  `MFB_MACAPP_HEADLESS`) never writes the file either.
- **Linux GTK aarch64** (box 2226, measured 2026-09-26 with this worktree's
  compiler): the extracted AppImage's `AppRun one two` under Xvfb writes `0:` —
  an empty argument list, not a crash.
- **Windows `--app`**: not measured.

Expected: `2:one,two` on every platform, exit 0.

## Root cause (macOS: read from the code, not confirmed under a debugger)

The standard program entry captures `argc`/`argv` for `os::args` from the first two
argument registers whenever the program uses `os::args`/`os::prog`
(`capture_args`, `src/codegen/engine/function/entry.rs`). In an `--app` build that
entry is called by the worker thread's shim, which unpacks the `{argc, argv}`
block into those registers **only when the language entry itself takes arguments**
(`if spec.language_entry_accepts_args`, `emit_worker_shim`,
`src/target/macos_aarch64/app/bootstrap.rs`). For `FUNC main AS Integer` it does
not, so the entry captures the block POINTER as `argc` and whatever `x1` holds as
`argv`, and `os::args` walks garbage. The GTK worker has the same gate
(`src/target/linux_gtk/bootstrap.rs`, `if spec.language_entry_accepts_args`);
why it yields an empty list rather than a crash there is not yet known (guess:
the register happens to hold 0).

## Fix

Phase 1 — RED test: an `--app` program printing `os::args()` run headless with
arguments (macOS `MFB_MACAPP_HEADLESS`; GTK under Xvfb), expecting `2:one,two`.

Phase 2 — unpack the argument block in both worker shims whenever the entry
captures arguments (`capture_args` or `language_entry_accepts_args`), then
confirm Windows (whose PE entry builds argv itself, `defers_arg_capture`).
