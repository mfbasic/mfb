# bug-743: `os::args` in an `--app` build crashes the program on macOS and returns an empty list on Linux GTK

Last updated: 2026-09-26
Effort: small
Severity: HIGH
Class: Correctness (process crash on macOS; wrong result on Linux)

Status: Open
Regression Test: none yet — see Fix

Found by bug-699 Phase 7 while writing the `--app` key probe
(`tools/key-probe`), which had to take its settings from the environment instead
of the command line because of this. Filed, not fixed there: it belongs to the
app bootstrap / `os` entry path, not to `io` input.

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
