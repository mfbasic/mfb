# bug-240: linux_gtk app — worker gets empty argv, unchecked malloc for marshal chunks, stale focus comment

Last updated: 2026-07-16
Effort: small (<1h)
Severity: LOW
Class: correctness / memory-safety / docs

Status: FIXED (2026-07-16). All three items done, runtime-verified on VM 2228
(Debian x86-64 + GTK4).

1. **Worker argv** — `_mfb_gtkapp_main` now publishes argc/argv to new
   `ST_ARGC`/`ST_ARGV` state slots and `emit_worker_shim` loads them for an
   arg-accepting entry. They ride the state rather than `pthread_create`'s `arg`
   (the macOS approach) because the GTK worker is created from the transient
   `activate` callback, which cannot reach `_mfb_gtkapp_main`'s locals.

   Two things the bug did not know about had to be fixed for this to mean
   anything — the report's claim that such a program "receives an empty arg
   vector" was wrong; it actually **crashed**:

   - `g_application_run(app, argc, argv)` was handed the real argv, but the app
     is registered with `G_APPLICATION_DEFAULT_FLAGS` — no `HANDLES_OPEN`, no
     `HANDLES_COMMAND_LINE` — so GApplication interpreted the argv tail as files
     to open and refused: `GLib-GIO-CRITICAL: This application can not open
     files`, no `activate`, non-zero return. ANY argument passed to ANY app-mode
     binary killed it before the program ran. Now it gets `argc=1` (argv[0]
     only), which keeps its platform-data valid while leaving the program's
     arguments to the program.
   - The entry itself ignored the registers the worker passed and read argc/argv
     off the raw-ELF `[sp]`/`sp+8` positions — garbage on a worker stack — and
     on x86-64 the arena-zero loop also clobbered argv outright. Both are
     [[bug-250]], fixed separately; without it this plumbing is inert.

   Runtime proof (VM 2228, headless `gtk4-broadwayd`): the program received
   `argc=4|/tmp/gtkargs.out|alpha|beta|gamma`, and `argc=2|…|one`.

2. **Unchecked chunk mallocs** — both are now NULL-checked. The io-write chunk
   falls back to the existing `fd_path` (allocation-free, so the output still
   reaches the user instead of faulting the worker); the finish chunk skips the
   status line and parks, leaving the window up and the main loop owning
   shutdown, so only the cosmetic exit-code line is lost.

3. **Stale focus comment** — reworded to say what the code does: nothing is
   focused on purpose, because keys are captured by the window-level key
   controller and the design deliberately avoids a focusable widget.

Also updated `src/docs/spec/app/02_linux-runtime.md` for the new state slots.
That table was **already stale** before this change, independently: it described
the pre-plan-35-E layout (`STATE_SIZE = 70320`, no snapshot grids) while the
emitted object is 139456 bytes. Corrected the whole section against the real
constants and verified every offset against the emitted data object.

Not fixed (pre-existing, out of scope): a GTK app under the **broadway** backend
segfaults during teardown *after* the program body completes — an arg-less `SUB
main` writes its file, then crashes. Reproduced identically on a pre-change
baseline binary, and c1e76921 verified GTK app mode interactively on a real X
display on this same box, so it looks specific to the headless broadway path.

Original report — three low items in the linux-gtk app-mode target:

- `emit_worker_shim` (`src/target/linux_gtk/bootstrap.rs:287-291`) passes
  `argc=0`/`argv=NULL` for an arg-accepting language entry (TODO(plan-05) gap), so
  such a program built with `mfb build -app` on Linux receives an empty arg
  vector even though the real argv reached `g_application_run`. Fix: plumb
  argc/argv from `_mfb_gtkapp_main` (stored at sp+8/sp+16) through to the worker,
  matching the console/macOS entry.
- The io-write chunk malloc (`src/target/linux_gtk/app_io.rs:461`) and the finish
  chunk malloc (`bootstrap.rs:513`) are used as `memcpy` destinations without a
  NULL check → allocation failure faults (`memcpy(chunk+16, ...)` → SIGSEGV on
  the worker) instead of degrading. Fix: branch on a NULL malloc result (drop the
  write / fall back to the fd path; skip the status line).
- `bootstrap.rs:216-218` comment says "focus the transcript so it receives keys"
  but no widget is focused and the design deliberately avoids a focusable widget
  (keys captured by a window-level key controller). Fix: reword the comment.
