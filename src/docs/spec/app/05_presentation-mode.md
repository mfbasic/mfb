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

`app::Mode` ships with two variants, declared in the built-in `app` source
companion so they resolve like any user enum (no reserved wire type id):

- `Console` — the terminal-in-a-window surface (a transcript view, optionally a
  full-screen `term::` grid). Discriminant `0`. The default.
- `None` — windowless. No surface is presented; `io::print` degrades to the
  standard-output file descriptor. Discriminant `1`.

The discriminants are the stored slot values, matching the enum's declaration
order, so a loaded mode word *is* the enum value with no remap. The enum is
referenced bare, like every other built-in type: `Mode.None`, not
`app::Mode.None`. [[src/codegen/registry/mod.rs:is_builtin_type]]

The set is designed to grow: a future graphical mode is a new `Mode` variant
entered through `app::setMode`, with no change to this model.

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
touches `app::` keeps its exact entry frame. [[src/target/shared/code/error_constants.rs:PRESENTATION_MODE_SLOTS]]

`app::getMode` and `app::setMode` are lowered inline to runtime helpers that load
and store this word — `getMode` is a single load (as cheap as reading a local),
`setMode` a store followed by the surface-reconcile seam below.
[[src/codegen/builtins/app/native.rs:lower_app_helper]]

Because the slot lives in the per-arena state region, it is per-execution-context,
consistent with the per-thread RNG and Money rounding mode.

## The static initial mode

A program's initial mode is decided **statically** at compile time, not at
runtime: `None` if the program references `app::setMode` anywhere — even on a
never-taken branch — and `Console` otherwise. A program that intends to manage its
own surface therefore starts windowless and brings a window up deliberately, while
a program that never touches the mode keeps the default terminal-in-a-window
surface. The decision keys on `setMode` specifically: a read-only `getMode` does
not force windowless startup. [[src/target/shared/code/mod.rs:lower_module]]

The worker entry seeds the mode slot to `None` when that is the static default;
`Console` needs no store because the arena-state region zero-inits to `0`
(`Console`), so a `Console`-default program's entry is byte-identical to one that
never referenced the mode at all. [[src/target/shared/code/entry.rs:lower_program_entry]]

## The surface-reconcile seam

Storing a new mode with `app::setMode` reconciles the window surface to match. The
store lands in the slot first; a per-backend reconcile hook then runs — on macOS
it marshals to the AppKit main thread, on Linux to the GTK main loop — to build or
tear down the window (with an implicit `term::off` so no raw/grid state survives a
mode switch). The reconcile reads the authoritative mode back from the slot rather
than a caller-saved register, since it emits register-clobbering cross-thread
calls. The per-backend window mechanics are `./mfb spec app macos-runtime` and
`./mfb spec app linux-runtime`. [[src/target/shared/code/types.rs:CodegenPlatform]]

## Mode-gated I/O

`term::*` and the console-reading side of `io::` (`io::input`, `io::readLine`,
`io::readChar`) require the `Console` surface: outside it they raise a trappable
wrong-mode runtime error rather than addressing a grid that does not exist or
blocking forever on an input pipe with no producer. `io::print` / `io::write` are
universal and degrade gracefully to standard output outside `Console`. This
asymmetry — universal I/O degrades, specialized I/O hard-fails — is what the mode
model makes well-defined.
