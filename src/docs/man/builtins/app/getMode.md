# getMode

Read the presentation mode currently in effect for this `--app` program

## Synopsis

```
app::getMode() AS Mode
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

`app::getMode` returns the program's current presentation mode as a `Mode` value.
It takes no arguments and always succeeds.
[[src/builtins/app.rs:APP]]

The mode reported is the value most recently written by `app::setMode`, or — if
the program has never called `app::setMode` — the statically decided initial mode.
That initial mode is `Console` for a program that references `app::setMode`
nowhere, and `None` for a program that references it anywhere (even on a
never-taken branch: the decision is a static, whole-program one, not a runtime
flow analysis). [[src/builtins/app_package.mfb:Mode]]

The `Mode` enum is referenced bare, like every other builtin type: write
`Mode.Console`, not `app::Mode.Console`. [[src/builtins/app.rs:APP]]

## Parameters

`app::getMode` takes no parameters. [[src/builtins/app.rs:APP]]

## Return value

| Type | Description |
| --- | --- |
| `Mode` | The presentation mode in effect: `Mode.Console` (the terminal-in-a-window surface, the default) or `Mode.None` (windowless). [[src/builtins/app.rs:APP]] |

## Errors

No errors.

## Examples

Branch on the mode currently in effect:

```
IMPORT app
IMPORT io

SUB main
  IF app::getMode() = Mode.None THEN
    io::print("running windowless")
  END IF
END SUB
```

## See also

- `mfb man app setMode`
- `mfb man app`
