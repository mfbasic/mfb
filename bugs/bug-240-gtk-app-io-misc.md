# bug-240: linux_gtk app — worker gets empty argv, unchecked malloc for marshal chunks, stale focus comment

Last updated: 2026-07-14
Effort: small (<1h)
Severity: LOW
Class: correctness / memory-safety / docs

Status: Open

Three low items in the linux-gtk app-mode target:

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
