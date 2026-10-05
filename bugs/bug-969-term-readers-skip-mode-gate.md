# bug-969: `term::getForeground` / `getBackground` (all app backends) and `term::isOn` (macOS) do not raise `ErrWrongMode` outside `Mode.Console`, though they declare it

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Correctness

Status: Open
Regression Test: (proposed) headless `--app` program on macOS and GTK: in `Mode.None` and `Mode.Canvas`, `term::isOn()`, `term::getForeground()`, `term::getBackground()` each raise 77050020

`mfb man term` says "every term:: call made while the program is in app::Mode.None or
app::Mode.Canvas raises ErrWrongMode", and the `isOn`, `getForeground`,
`getBackground` pages each list `ErrWrongMode` (bug-727's declared-errors sweep). The
presentation-mode table says `term::*` traps in `None` and `Canvas`.

Measured: on macOS `term::isOn()` answers (`FALSE`, or a stale `TRUE` — see the
implicit-term::off report) in `None` and `Canvas`; on macOS **and** GTK
`term::getForeground()` returns normally in `None`. GTK's `isOn` does trap. So the
same program behaves differently per backend and contradicts its own docs.

The single correct behavior: every `term::` member is `Console`-gated in an `--app`
build on every backend, as declared.

References:

- `src/docs/spec/app/05_presentation-mode.md` "Mode-gated I/O" table.
- `mfb man term`, `mfb man term isOn` / `getForeground` / `getBackground` Errors tables.
- bugs/completed/bug-727 (declared every term member's `ErrWrongMode`).

## Failing Reproduction

`/tmp/hardening/B17/ig/src/main.mfb`:

```
IMPORT app
IMPORT term
IMPORT io
IMPORT color

FUNC main() AS Nothing
  io::print("mode=" & toString(app::getMode()))
  LET a AS Boolean = term::isOn() TRAP(e)
    io::print("isOn None raised " & toString(e.code))
    RECOVER FALSE
  END TRAP
  LET c AS color::Color = term::getForeground() TRAP(e)
    io::print("getForeground None raised " & toString(e.code))
    RECOVER color::rgb(1, 1, 1)
  END TRAP
  app::setMode(app::Mode.Canvas)
  LET b AS Boolean = term::isOn() TRAP(e)
    io::print("isOn Canvas raised " & toString(e.code))
    RECOVER FALSE
  END TRAP
END FUNC
```

```
mfb build --app ig && MFB_MACAPP_HEADLESS=1 ig/build/p1.app/Contents/MacOS/p1
mfb build --app --target linux-aarch64 ig   # podman: MFB_GTKAPP_HEADLESS=1 ./p1-glibc.AppImage
```

- Observed macOS: no `isOn None raised`, no `getForeground None raised`, no
  `isOn Canvas raised` (only `didResize`/`on` raise).
- Observed GTK: `isOn None raised 77050020`, `isOn Canvas raised 77050020`, but no
  `getForeground None raised`.
- Expected: all three lines raised on both.

## Root Cause

`src/codegen/builtins/term/gen_shared.rs:lower_term_helper` prepends
`prepend_wrong_mode_gate(..., ModeRequirement::Console)` only when the platform's
`emit_app_term_helper` returns `Some`, plus a special case for
`term.enableMouse`/`term.pollMouse`. Members a backend leaves on the console emitter
(`None` from the dispatcher — macOS: `isOn` and the attribute getters; GTK: the
getters) fall through to `console_lower_term_helper` ungated.

## Goal

- The fall-through path gates every member in an app build (not just the two mouse
  members).

### Non-goals (must NOT change)

- Console builds (`presentation_mode_offset` is `None`, gate is a no-op).

## Blast Radius

- `getBold`/`getUnderline`/other pure readers on the fall-through — same hazard, not
  individually run.
- Windows dispatcher — not run.
