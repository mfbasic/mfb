# app

Presentation-mode control for `--app` builds

## Synopsis

```
IMPORT app
app::setMode(Mode.None)
app::getMode()
```

## Imports

```
IMPORT app
```

`app` is a built-in package, but it is importable **only** in `--app` builds.
`IMPORT app` in a plain console build is a compile-time error: the package's whole
purpose is to control an app window's presentation surface, which a console binary
does not have. Enable app mode with the `-app` build flag or `"mode": "app"` in
`project.json`. [[src/cli/build/mod.rs:build_project]]
[[src/builtins/app.rs:package_source_glue]]

## Description

The `app` package makes an `--app` program's **presentation mode** — what its
window surface currently *is* — a first-class, explicit choice, replacing the
older tangle of a `uses_term` flag and `term::on` / `term::off` toggling. A
running program reads its mode with `app::getMode` and changes it with
`app::setMode`. [[src/builtins/app.rs:is_app_call]]

The mode is one of the `Mode` enum members:

- `Console` — the terminal-in-a-window surface (a transcript view, optionally a
  full-screen `term::` grid). This is the default.
- `None` — windowless. No surface is presented; `io::print` degrades to standard
  output.

[[src/builtins/app_package.mfb:Mode]]

A program's **initial** mode is decided statically: `Console` unless the program
references `app::setMode` anywhere, in which case it starts in `None`. This lets a
program that intends to manage its own surface start windowless and bring a window
up deliberately, while a program that never touches the mode keeps today's
terminal-in-a-window behavior unchanged.

The `Mode` enum is referenced bare, like every other builtin type: write
`Mode.None`, not `app::Mode.None`. [[src/builtins/app.rs:APP]]

The mode model is designed to grow: a future graphical mode is a new `Mode`
variant entered through `app::setMode`, with no change to this surface.

## Errors

`app::getMode` and `app::setMode` raise no errors from the mode machinery itself:
the argument to `setMode` is a `Mode` the type checker has already constrained, and
reading the current mode cannot fail. [[src/builtins/app.rs:APP]]
