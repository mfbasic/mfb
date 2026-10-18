# bug-965: GTK `app::setMode` returns before the window is reconciled, so the next `io::print` goes to the wrong sink (stdout instead of the new transcript, or the hidden transcript instead of stdout)

Last updated: 2026-10-05
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Fixed
Coordinator note: measured by the review agent only (Linux runs in podman, or a macOS window screenshot); the coordinator did not re-run it.
Regression Test: (proposed) GTK/Xvfb run of `setMode(Console); io::print("A")` asserts `A` is in the transcript buffer and not on stdout; and `setMode(None); io::print("C")` asserts `C` is on stdout

## Cause (coordinator, bug-staging-24)

- Rule: when `app::setMode(m)` returns, the surface has been reconciled and `io::` writes follow `m`'s routing (`05_presentation-mode.md` "The surface-reconcile seam" and the "Mode-gated I/O" table); macOS meets it by marshalling the reconcile with `waitUntilDone:YES`.
- Where: `src/target/linux_gtk/bootstrap.rs:emit_reconcile_seam` posts the reconcile with `g_idle_add` and returns at once (`07_window.md`: "fire-and-forget, like the mode reconcile").
- Fix: the GTK seam waits for the posted reconcile to finish (the idle signals completion, the worker blocks until it does), the same contract as macOS; headless stays state-only. The spec's "fire-and-forget, like the mode reconcile" goes for the mode reconcile. Check the Windows `WM_APP_RECONCILE` arm and say in the doc whether it is synchronous (fix it the same way if not).

On the Linux GTK backend the mode reconcile is posted with `g_idle_add` and
`app::setMode` returns immediately. The io write helper routes on `ST_TEXT_BUFFER`
at the moment of the write, so output written right after the call follows the
**previous** mode's routing:

* `None` → `Console`, then `io::print("A")`: `A` is written to **stdout** (the
  transcript does not exist yet) — a double-clicked AppImage has no visible stdout, so
  the line is lost from the window.
* `Console` → `None`, then `io::print("C")`: `C` is appended to the **now-hidden
  transcript** and never appears on stdout.

The macOS backend marshals the reconcile synchronously (`waitUntilDone:YES`) and routes
both lines correctly with the same program.

The single correct behavior: after `app::setMode(m)` returns, `io::` writes follow `m`'s
routing (presentation-mode "Mode-gated I/O" table).

References:

- `src/docs/spec/app/05_presentation-mode.md` "The surface-reconcile seam" and the
  "Mode-gated I/O" table (`Console` → transcript view; `None` → stdout).
- `src/docs/spec/app/07_window.md` Linux: "`g_idle_add(...)` — fire-and-forget, like
  the mode reconcile".
- Found by hardening batch B17.

## Failing Reproduction

```
IMPORT app
IMPORT io
IMPORT os

FUNC main() AS Nothing
  app::setMode(app::Mode.Console)
  io::print("A immediately after setMode")
  os::sleep(500)
  io::print("B after 500ms")
END FUNC
```

```
mfb build --app --target linux-aarch64 ms5
# podman + Xvfb: APPIMAGE_EXTRACT_AND_RUN=1 ./p1-glibc.AppImage > o.txt; cat o.txt
```

- Observed stdout: `A immediately after setMode` (B went to the transcript).
- Expected stdout: empty; both lines in the window transcript.

Second program (`Console`, sleep 500, print `B`, `setMode(None)`, print `C`, sleep 500,
print `D`): observed stdout is only `D`; expected `C` and `D`.

macOS `--app` of the first program: stdout empty (both in the transcript) ✓.

## Root Cause

`src/target/linux_gtk/bootstrap.rs:emit_reconcile_seam` posts the reconcile idle and
returns; `emit_reconcile_idle_helper` is what sets/clears `ST_TEXT_BUFFER`
(`src/target/linux_gtk/app_io.rs:emit_app_io_write` routes on it). Nothing orders the
worker's next write after the idle.

## Goal

- `setMode` does not return until the reconcile has run (or the worker-side routing
  state is updated synchronously before return), so the next write is routed by the
  new mode.

### Non-goals (must NOT change)

- Headless behavior (no main loop; reconcile is state-only).

## Blast Radius

- The window sync (`_mfb_gtkapp_window_sync`) is also fire-and-forget; `getTitle`/
  `getFullscreen` read process-global state so they are unaffected (not measured).
- `canvas::present` immediately after `setMode(Canvas)` on GTK: not observed to fail
  (ca1 on GTK rendered and counted frames).

## Fix

- **What changed.** `src/target/linux_gtk/bootstrap.rs:emit_reconcile_seam` no longer
  posts the idle and returns: it calls the new `_mfb_gtkapp_reconcile_marshal`
  (`emit_reconcile_marshal_helper`), which skips at once when `ST_APPLICATION` is null
  (headless — state-only, unchanged), else builds a request block on its own stack
  (mode `+0`, unnamed POSIX `sem_t` `+16`), `sem_init(sem,0,0)`,
  `g_idle_add(_mfb_gtkapp_reconcile_idle, &req)`, `sem_wait` (retried while non-zero,
  i.e. EINTR), `sem_destroy`. `emit_reconcile_idle_helper` now reads the mode from the
  block and `sem_post`s as its last act. The four `sem_*` imports come from the
  `app.setMode` call row in `src/target/linux_common/plan.rs` (the backend's pthread
  library: `libpthread.so.0` on aarch64 glibc, `libc.so.6` on x86-64 glibc, musl libc).
  macOS already waited (`waitUntilDone:YES`).
- **Windows `WM_APP_RECONCILE`: already synchronous** (read in code):
  `src/target/win_x86_64/app/mod.rs:emit_reconcile_seam` uses a cross-thread
  `SendMessageW`, which blocks until the UI thread's pump dispatches it, skipped
  when `MAIN_HWND_SYM` is 0 (headless). No change.
- **Deadlock analysis (what was checked).**
  - *Headless*: `_mfb_gtkapp_main` takes the `MFB_HEADLESS` branch before
    `gtk_application_new`, so `ST_APPLICATION` stays 0 and the marshal returns
    without posting or waiting. Measured: `MFB_HEADLESS=1` probe on 2226 exits 0 with
    all lines on stdout.
  - *Main loop blocked on a program thread*: no main-loop callback waits on the
    worker — the key pipe's write end is `O_NONBLOCK` (bug-114, `emit_input_pipe_wiring`),
    the title lock (`window.rs`) is held only around a read + GTK call on both sides and
    `setMode` holds no lock while waiting, nothing on the main thread joins/waits a
    program thread (grep of `linux_gtk` for `pthread_join`/`usleep`/blocking calls: the
    only `usleep`s are the headless scripted-resize/pointer hooks, which run on the
    main thread only in the headless branch, where no reconcile is ever posted).
  - *setMode on the main thread*: program code runs only on the worker and `thread::`
    threads; no GTK callback calls `setMode`.
  - *Shutdown*: window close → `g_application_quit`; a waiting `setMode`'s idle may
    never run, but `g_application_run` returns, `_mfb_gtkapp_main` returns into
    `exit`, and the blocked thread ends with the process. Program completion goes
    through `_mfb_gtkapp_finish` after its last `setMode` returned.
  - *Concurrent setMode from several threads*: one stack block + semaphore per call;
    same-priority idles run in post order.
  - *Before the loop runs*: the worker is spawned from `activate` (inside
    `g_application_run`), so a posted idle always has a running loop.
  - Stress: 300 switches (`Console`/`Canvas`/`None` × 100, a print after each) on 2226
    under Xvfb completed, exit 0, every `S`/`N` line on stdout in order and every `K`
    line in the transcript dump (`grep -c` = 100 each).
- **Test.** `scripts/test-appimage.sh` `--gui` row "setMode reconciles before
  returning (io follows the new mode)": exact stdout `C-none`, then the dumped
  transcript `A-console-1`, `E-console-2`. RED with main's compiler
  (`MFB_APPIMAGE_DISPLAY=:77 scripts/test-appimage.sh /tmp/main-dbg-target/release/mfb
  --box 2226 --libc glibc --gui`: got `A-console-1 C-none E-console-2`), GREEN with the
  fix. The other 4 FAILs of that run (`could not read DT_NEEDED` — 2226 has no
  `readelf`; `never reached the GTK bootstrap`/`windowed run exited early` with empty
  output; `DT_RUNPATH ''`) are identical with main's compiler and are box-environment
  failures on 2226, not this change. 2228/2227 (the script's default x86-64 boxes)
  were unreachable (`Connection refused`), so x86-64 GTK was checked only at the
  ncode level (frame 56+64 keeps `rsp` 16-aligned at each call; `sem_t` at `rsp+32`).
  Unit: `linux_gtk::…::reconcile_marshal_waits_for_the_idle`,
  `reconcile_idle_posts_the_waiter_last`.
  The script also now forwards `MFB_APPIMAGE_DISPLAY` to the box (it was expanded on
  the remote side, where ssh forwards no environment, so it never took effect).
- **Checks run.** `cargo test --release --bin mfb -- linux_gtk:: linux_common::` 57
  passed; `--test cli_linux_app_mode` 11 passed; `--test cli_app_canvas_mode` 13
  passed; `--test scripts_index_is_complete` 2 passed; `spec-census --citations app`
  0 missing, `--render app` 0 leaks, `--links app` 0 unresolved.
- **Goldens expected to change** (`scripts/artifact-gate.sh <mfb> all`):
  `tests/syntax/app/app-window-surface/golden/app_window_surface.{linux-aarch64,linux-x86_64}.app.ncodesum`
  and (on top of bug-966's move) `tests/syntax/app/app-mouse-surface/golden/app_mouse_surface.{linux-aarch64,linux-x86_64}.app.ncodesum`
  — the GTK app fixtures that reference `app::setMode`. Localized on
  app-window-surface linux-aarch64 (main's `-ncode` hashes to the committed golden):
  `_mfb_gtkapp_reconcile_marshal` added, `_mfb_gtkapp_reconcile_idle` and
  `_mfb_rt_app_app_setMode` changed, imports `+ sem_init/sem_wait/sem_post/sem_destroy`
  (`libpthread.so.0`); nothing else.
