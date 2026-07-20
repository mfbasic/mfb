# isBuffered

Report whether opt-in output buffering is enabled for an open `File`

## Synopsis

```
fs::isBuffered(file AS File) AS Boolean
```

## Package

fs

## Imports

```
IMPORT fs
```

`fs` is a built-in package, so no manifest dependency is required.
[[src/builtins/fs.rs:is_fs_call]]

## Description

`fs::isBuffered` reads the per-handle buffering flag on a single open `File` and
returns `TRUE` when output buffering is currently enabled for that handle and
`FALSE` otherwise. It only inspects the handle's state — it writes no data, drains
nothing, and has no side effect.
[[src/target/shared/code/fs_helpers_io.rs:lower_fs_is_buffered_helper]]

Buffering is a per-handle flag stored on the `File` resource itself, so this call
reflects only `file` and no other open handle; each `File` carries its own buffer
and its own enabled flag.
[[src/target/shared/code/error_constants.rs:FILE_OFFSET_BUF_ENABLED]]

Buffering is **off by default**: a freshly opened `File` starts with its buffered
flag clear, so a program that never calls `fs::setBuffered` always observes
`FALSE` here. The flag becomes `TRUE` after `fs::setBuffered(file, TRUE)` and
returns to `FALSE` after `fs::setBuffered(file, FALSE)`. Transferring a buffered
handle to another thread resets it to unbuffered, so the receiving thread again
observes `FALSE`.
[[src/target/shared/code/fs_helpers_io.rs:lower_fs_set_buffered_helper]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `file` | `File` | An open `File` resource whose buffering flag is being queried. [[src/builtins/fs.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` when output buffering is enabled for this handle; `FALSE` otherwise (including on a freshly opened handle that has never enabled it). [[src/builtins/fs.rs:call_return_type_name]] |

## Errors

No errors. `fs::isBuffered` only reads the handle's buffering flag and always
returns success; it never raises.
[[src/target/shared/code/fs_helpers_io.rs:lower_fs_is_buffered_helper]]

## Examples

Enable buffering only when it is not already on:

```
IMPORT fs

SUB main()
  RES log = fs::openFile("events.log", "write")
  IF NOT fs::isBuffered(log) THEN
    fs::setBuffered(log, TRUE)
  END IF
END SUB
```

## See also

- `mfb man fs setBuffered`
- `mfb man fs flush`
- `mfb man fs writeAll`
- `mfb man fs close`
