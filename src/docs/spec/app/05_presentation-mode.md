# Presentation Mode

An `--app` program's **presentation mode** is what its window surface currently
*is*. It is a first-class, extensible runtime concept (plan-62), replacing the
older tangle of a `uses_term` flag and `term::on` / `term::off` toggling. A
running program reads its mode with `app::getMode` and changes it with
`app::setMode`; the mode is one of the members of the `app::Mode` enum.
[[src/codegen/builtins/app/mod.rs:Mode]]

This topic specifies the *model* — the mode set, the per-arena state, the static
initial-mode rule, and the surface-reconcile seam. The per-function API
(`app::getMode` / `app::setMode`) is `./mfb man app`; the per-backend window
construction is `./mfb spec app macos-runtime` and `./mfb spec app linux-runtime`.

## The `Mode` enum

`app::Mode` ships with three variants, rendered into the built-in `app` package's
injected source from the registry descriptor so they resolve like any user enum
(no reserved wire type id):

- `Console` — the terminal-in-a-window surface (a transcript view, optionally a
  full-screen `term::` grid). Discriminant `0`. The default.
- `None` — windowless. No surface is presented; `io::print` degrades to the
  standard-output file descriptor. Discriminant `1`.
- `Canvas` — a 2D graphics surface drawn by the `canvas` package. Discriminant
  `2`. The window presents pixels rather than character cells, so `term::` is
  unavailable, but `io::` works in full: writes degrade to standard output and
  reads are fed by the canvas window's key events (plan-98).

The discriminants are the stored slot values, matching the enum's declaration
order, so a loaded mode word *is* the enum value with no remap. The enum is
referenced bare, like every other built-in type: `Mode.None`, not
`app::Mode.None`. [[src/codegen/registry/mod.rs:is_builtin_type]]

The set is designed to grow, and grew exactly that way: a new presentation
surface is a new `Mode` variant entered through `app::setMode`, with no change to
this model. Because declaration order fixes the discriminants, a variant is
**appended** — reordering would repoint every already-stored slot word.

## `--app` gating

The `app` package is importable **only** in `--app` builds. `IMPORT app` in a
plain console build is a compile-time error raised at the CLI before lowering —
the package controls a window surface a console binary does not have. The name
gate makes the import legal; the CLI enforces the app-mode requirement, since only
it sees the full app-mode decision (the additive `-app` flag over the manifest
`"mode": "app"`). [[src/cli/build/mod.rs:build_project]]

## Per-arena mode state

The current mode is one word in the program-entry frame, reserved a single slot
past the `term::` state region and addressed off the pinned arena-state register —
the same threading model as `term::` state. The slot is reserved only when the
program actually uses `app::` (the `uses_term` model), so an app binary that never
touches `app::` keeps its exact entry frame. [[src/codegen/error/constants/error_constants.rs:PRESENTATION_MODE_SLOTS]]

`app::getMode` and `app::setMode` are lowered inline to runtime helpers that load
and store this word — `getMode` is a single load (as cheap as reading a local),
`setMode` a store followed by the surface-reconcile seam below.
[[src/codegen/builtins/app/func_get_mode.rs:lower_get_mode]] [[src/codegen/builtins/app/func_set_mode.rs:lower_set_mode]]

Because the slot lives in the per-arena state region, it is per-execution-context,
consistent with the per-thread RNG and Money rounding mode.

## The static initial mode

A program's initial mode is decided **statically** at compile time, not at
runtime: `None` if the program references `app::setMode` anywhere — even on a
never-taken branch — and `Console` otherwise. A program that intends to manage its
own surface therefore starts windowless and brings a window up deliberately, while
a program that never touches the mode keeps the default terminal-in-a-window
surface. The decision keys on `setMode` specifically: a read-only `getMode` does
not force windowless startup. [[src/codegen/engine/builder/mod.rs:lower_module]]

The worker entry seeds the mode slot to `None` when that is the static default;
`Console` needs no store because the arena-state region zero-inits to `0`
(`Console`), so a `Console`-default program's entry is byte-identical to one that
never referenced the mode at all. [[src/codegen/engine/function/entry.rs:lower_program_entry]]

## The surface-reconcile seam

Storing a new mode with `app::setMode` reconciles the window surface to match. The
store lands in the slot first; a per-backend reconcile hook then runs — on macOS
it marshals to the AppKit main thread, on Linux to the GTK main loop — to build or
tear down the window (with an implicit `term::off` so no raw/grid state survives a
mode switch). The reconcile reads the authoritative mode back from the slot rather
than a caller-saved register, since it emits register-clobbering cross-thread
calls. The per-backend window mechanics are `./mfb spec app macos-runtime` and
`./mfb spec app linux-runtime`. [[src/codegen/engine/types/types.rs:CodegenPlatform]]

## Mode-gated I/O

The governing asymmetry is **universal I/O degrades, specialized I/O hard-fails**.
What makes an operation "specialized" is the surface it needs, and the two gated
families need different ones — so they are gated differently.
[[src/codegen/app/hook/app.rs:ModeRequirement]]

| | `Console` (0) | `None` (1) | `Canvas` (2) |
|---|---|---|---|
| `io::print` / `io::write` (and error variants) | transcript view | degrades to stdout/stderr | degrades to stdout/stderr |
| `io::input` / `io::readLine` / `io::readChar` | window key events | **traps `ErrWrongMode`** | window key events |
| `io::readByte` | window key events | ungated (reads fd 0) | window key events |
| `term::*` | the grid | **traps `ErrWrongMode`** | **traps `ErrWrongMode`** |

- **`term::*` requires the character grid**, which only the `Console` transcript
  view has. A canvas surface is pixels, not cells, so `term::` traps in `Canvas`
  exactly as it does in `None`.
- **The console-reading side of `io::` requires only an input source.** `Console`
  and `Canvas` both have a window, and both deliver its key events into the same
  file descriptor, so a read works in either. Only `None` — which presents no
  window at all — has nowhere for input to come from, and there an ungated read
  would block forever on a pipe with no producer. The gate is therefore written as
  "trap in `None`" rather than "permit only `Console`", so a future windowed mode
  inherits the input source instead of silently trapping.
- **`io::readByte` is not in the gated set** and never has been: it reads fd 0
  directly in every mode.
- **Writes are never gated** — they degrade to standard output wherever no
  transcript view is attached.
