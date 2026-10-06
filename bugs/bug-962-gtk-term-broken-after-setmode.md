# bug-962: GTK app — `term::` in any program that calls `app::setMode` has no drawing area: nothing renders, GTK-CRITICALs, and the first TUI `io::print` SIGSEGVs on aarch64

Last updated: 2026-10-05
Effort: medium (1h–2h)
Severity: HIGH
Class: Memory-safety

Status: Open
Coordinator note: measured by the review agent only (Linux runs in podman, or a macOS window screenshot); the coordinator did not re-run it.
Regression Test: (proposed) GTK `--app` program `setMode(Console); term::on(); io::print("x"); term::off()` under Xvfb exits 0 with no `Gtk-CRITICAL` on stderr and the grid shows `x`

Referencing `app::setMode` anywhere makes a program start in `Mode.None`
(presentation-mode "static initial mode"). On GTK the window for such a program is
built later by the mode reconcile (`emit_reconcile_build_helper`), which creates the
window, scrolled transcript, text view and key controller — but **not** the `term::`
`GtkDrawingArea` (`ST_TERM_AREA`), which only the Console-default `activate` path
creates. `ST_TERM_AREA` therefore stays `0`, and every `term::` present/show calls GTK
with a NULL widget.

Measured effects: `term::on`/`term::sync` print
`Gtk-CRITICAL ... gtk_widget_queue_draw: assertion 'GTK_IS_WIDGET (widget)' failed`
and draw nothing; on linux-aarch64 the first `io::print` while TUI mode is on kills
the process with SIGSEGV (exit 139), deterministically (3/3 runs). On linux-x86_64 the
same program prints the CRITICALs, draws nothing, and exits 0.

The single correct behavior: a reconcile-built `Console` window has the same `term::`
surface as a startup-built one, so `term::on` swaps in a real drawing area and a
`setMode`-using program's TUI renders exactly like one that never touches `app::`.

References:

- `src/docs/spec/app/05_presentation-mode.md` "The static initial mode" and "The
  surface-reconcile seam"; `src/docs/spec/app/02_linux-runtime.md` "activate" (term
  area "created up front").
- Found by hardening batch B17.

## Failing Reproduction

```
IMPORT app
IMPORT term
IMPORT io
IMPORT os

FUNC main() AS Nothing
  app::setMode(app::Mode.Console)
  os::sleep(500)
  term::on()
  io::print("in tui")
  term::off()
END FUNC
```

```
mfb build --app-debug --target linux-aarch64 gz
# podman debian:trixie + libgtk-4-1 + Xvfb; DISPLAY=:9
./p1-glibc.AppDir/AppRun; echo exit=$?
```

- Observed: `Gtk-CRITICAL **: gtk_widget_queue_draw: assertion 'GTK_IS_WIDGET (widget)' failed`
  then `exit=139` (aarch64, 3/3). x86-64: same CRITICALs, `exit=0`, empty grid.
- Expected: exit 0, no CRITICAL, `in tui` drawn on the grid.

Contrast (works ✓): the same body **without** `app::setMode` (Console-default
startup) on GTK (`/tmp/hardening/B17/gt1src`, screenshot `gt1_s.png`); the same
program on macOS `--app` (`/tmp/hardening/B17/gym_c.png` shows the grid).

| Environment | Result |
| --- | --- |
| linux-aarch64 GTK4/Xvfb | SIGSEGV ✗ |
| linux-x86_64 GTK4/Xvfb (qemu) | CRITICALs, nothing drawn ✗ |
| macOS aarch64 | works ✓ |

## Root Cause

`src/target/linux_gtk/bootstrap.rs:emit_reconcile_build_helper` builds the window
(title, default size, `ST_SCROLLED`, `ST_TEXT_VIEW`, `ST_TEXT_BUFFER`, key controller,
`close-request`) but never runs the `gtk_drawing_area_new` / `g_object_ref_sink` /
`gtk_drawing_area_set_draw_func` / `TERM_INIT_SYMBOL` / `resize`-signal sequence that
`emit_activate_handler` runs under `if initial_mode == PresentationMode::Console`.
The term show / redraw idles (`term_draw.rs`, `ST_TERM_AREA` loads at the
`gtk_window_set_child` / `gtk_widget_queue_draw` sites) then pass NULL. The crash
on aarch64 is downstream of the NULL child (worker-side, thread = the program
thread per gdb `info threads`); no symbols were available to pin the exact frame.

## Goal

- A `None`-initial program that enters `Console` and uses `term::` renders, with no
  GTK-CRITICAL and no crash, on both Linux arches.

### Non-goals (must NOT change)

- Console-default programs' startup byte stream (the term area creation there).

## Blast Radius

- `emit_reconcile_build_helper` — fixed by this bug.
- The term-area grid geometry (`TERM_INIT_SYMBOL`) and `resize` handler must be
  wired for the reconcile-built area too (guess, not measured: otherwise
  `term::terminalSize` would not track the window).
