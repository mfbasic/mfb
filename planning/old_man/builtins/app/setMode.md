# setMode

Change the presentation mode of this `--app` program

## Synopsis

```
app::setMode(mode AS Mode)
```

## Package

`app`

## Imports

```
IMPORT app
```

`app` is a built-in package, importable only in `--app` builds; `IMPORT app` in a
console build is a compile-time error. [[src/cli/build/mod.rs:build_project]]

## Description

`app::setMode` sets the program's presentation mode. `mode` is one of the two
`Mode` enum members: `Mode.Console` (the terminal-in-a-window surface) or
`Mode.None` (windowless). The call returns nothing.
[[src/codegen/builtins/app/mod.rs:APP]] [[src/codegen/builtins/app/mod.rs:Mode]]

Switching mode reconciles the window surface to match: entering `None` tears the
window down and routes `io::print` to standard output; entering `Console` brings
the transcript window up. A subsequent `app::getMode` reflects the new mode.

Referencing `app::setMode` anywhere in a program also changes that program's
**initial** mode to `None` — a program that manages its own surface starts
windowless and brings a window up deliberately, rather than flashing the default
terminal window first.

The `Mode` enum is referenced bare, like every other builtin type: write
`Mode.None`, not `app::Mode.None`. [[src/codegen/builtins/app/mod.rs:APP]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `mode` | `Mode` | The presentation mode to switch to: `Mode.Console` or `Mode.None`. Any other type is rejected at compile time. [[src/codegen/registry/mod.rs:call_param_names]] [[src/codegen/builtins/app/mod.rs:APP]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | `app::setMode` produces no value; call it as a statement. [[src/codegen/builtins/app/mod.rs:APP]] |

## Errors

No errors.

## Examples

Start windowless (the mere reference to `setMode` makes `None` the initial mode),
then bring the console surface up:

```
IMPORT app
IMPORT io

SUB main
  io::print("no window yet")
  app::setMode(Mode.Console)
END SUB
```

## See also

- `mfb man app getMode`
- `mfb man app`
