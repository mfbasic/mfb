# bug-962: GTK app — `term::` in any program that calls `app::setMode` has no drawing area: nothing renders, GTK-CRITICALs, and the first TUI `io::print` SIGSEGVs on aarch64

Last updated: 2026-10-05
Effort: medium (1h–2h)
Severity: HIGH
Class: Memory-safety

Status: Fixed on linux-aarch64 glibc (bs16-agent-c, re-run by the coordinator on 2226: base rc 139 3/3, fixed rc 0 3/3 with no GTK-CRITICAL). x86-64 GTK (2228) and musl GTK (2225) NOT run: both boxes are down and the owner is not bringing them up for now.
Coordinator note: measured by the review agent only (Linux runs in podman, or a macOS window screenshot); the coordinator did not re-run it.
Regression Test: `target::linux_gtk::canvas_reconcile_tests::a_none_default_program_builds_the_term_surface_before_the_worker_spawns` (unit, both start modes). Runtime: the probes below, on 2226.

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

## Resolution

The `term::` area setup (`gtk_drawing_area_new` + `g_object_ref_sink` → `ST_TERM_AREA`,
the draw func, `_mfb_gtkapp_term_init`, the `resize` handler) is factored out of the
`Console` arm of `emit_activate_handler` into `emit_term_area_setup`
(`src/target/linux_gtk/bootstrap.rs`). The activate handler now also calls it in the `None`
arm, after `g_application_hold` and before the worker spawns. The emitted instructions are
the same, so a Console-default program's startup bytes are unchanged.

The doc's root cause points at `emit_reconcile_build_helper`. The area is NOT built there,
because the reconcile is a `g_idle_add` with no ordering against the worker. `setMode`
returns before the idle runs, so a `term::on(); io::print(...)` right after
`setMode(Console)` would write a grid that `term_init` then blanks, with `cols`/`rows`
still 0. Activate runs before the worker exists, which is the same guarantee the Console
path relies on. Spec: `mfb spec app linux-runtime` "`activate`" now says the area is built
in both start modes, and why.

Not changed here (reported to the coordinator for the next batch): the reconcile-built
window attaches no mouse controllers, so a `setMode` program gets no mouse events. Any GTK
mouse event SIGSEGVs on linux-aarch64 (bug-982, coordinator-filed), so attaching them now
would turn "no events" into a crash.

### Proof (box 2226, Debian 12 aarch64 glibc, GTK4; Xvfb `:91`, `GSK_RENDERER=cairo GDK_BACKEND=x11`)

2228 (x86_64 GTK) and 2225 (Alpine musl GTK) were DOWN for this run (connection refused),
so **x86-64 and musl runs are still owed**. Without `GSK_RENDERER=cairo`, `xwd` of the Xvfb
root is all black even for a Console-default control. With it, the control renders the grid.
Runner: `rungtk.sh` (start Xvfb, run the AppImage, `xwd -root` at 1.6 s, rc).

| probe (`mfb build --app --target linux-aarch64`) | base (`/tmp/main-dbg-target`) | fixed |
| --- | --- | --- |
| doc repro (`setMode(Console)`, sleep 500, `term::on`, `io::print`, `term::off`) | rc 139, 3/3 | rc 0, 3/3 |
| same + `term::terminalSize` + 1.5 s hold for a screenshot | rc 139 3/3, `Gtk-CRITICAL gtk_widget_queue_draw: assertion 'GTK_IS_WIDGET (widget)' failed` | rc 0 3/3, no CRITICAL, size 69x21 (= the Console-default control), screenshot shows `in tui` |
| `setMode(Console)` then `term::on` with no sleep (the race) | rc 139 3/3 | rc 0 3/3, `race` drawn |
| Console → None → Console, then `term::on` | rc 139 3/3 (CRITICAL) | rc 0 3/3, `cycled` drawn |
| Console-default control (no `setMode`) | — | rc 0, `in tui` drawn, 69x21 |

RED: the unit test failed before the fix (`left: ["pthread_create"]`, `right:
["gtk_drawing_area_new", "gtk_drawing_area_set_draw_func", "_mfb_gtkapp_term_init",
"pthread_create"]`). It passes after, as do the 33 `linux_gtk` unit tests
(`cargo test --bin mfb linux_gtk`).

## Golden impact

Only None-default (setMode-using) Linux app programs change: `_mfb_gtkapp_activate` gains
the 26-instruction term-area block and its relocations. `scripts/artifact-gate.sh <mfb> all`
shows two new diffs beyond the branch base's 59:
`syntax/app/app-window-surface/app_window_surface.linux-{aarch64,x86_64}.app.ncode`. For
aarch64 the diff is checked to be exactly that block (`mfb build -ncode --app --target
linux-aarch64`, branch base vs fix). `syntax/app/app-mouse-surface`'s Linux goldens were
already in the branch base's diff set. That fixture calls `app::setMode`
(`grep -l setMode tests/syntax/app/app-mouse-surface/src/*.mfb`), so its Linux goldens
take this block as well.
