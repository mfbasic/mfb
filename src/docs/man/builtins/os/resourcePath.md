# resourcePath

The absolute path of a build resource

## Synopsis

```
os::resourcePath(relative AS String) AS String
```

## Package

os

## Imports

```
IMPORT os
```

`os` is a built-in package, so no manifest dependency is required.
[[src/builtins/os.rs:is_os_call]]

## Description

`os::resourcePath` returns the **absolute** on-disk path of a resource the build
copied out of the project's manifest `resources` section, as an owned `String`.
The `relative` argument is the resource's path below its declared destination
directory (for example `music/song.ogg`), and the result is `<base>/<relative>`.

The base directory is derived at runtime from the running executable's own path
and a build-mode offset baked into the binary, so the same call resolves
correctly for every build shape:

| Build | Executable path | Resource base |
| --- | --- | --- |
| console | `…/build/<name>` | `…/build` |
| macOS `--app` | `…/Contents/MacOS/<name>` | `…/Contents/Resources` |
| Linux `--app` | `…/usr/bin/<name>` | `…/usr/share/<name>` |

The result is absolute and contains no `..` segments, so it opens with `fs::open`
regardless of the working directory — including a macOS `.app` launched from
Finder or a mounted `.AppImage`. Resolution reads only the executable's own path
(`/proc/self/exe` on Linux, `_NSGetExecutablePath` on macOS) and never consults
`$APPDIR` or any other environment variable.
[[src/target/shared/code/os.rs:lower_resource_path]]

A `relative` containing a `.` or `..` **path component** raises `ErrInvalidPath`
— a resource path must not navigate out of the base. A dot *inside* a filename
(`song.ogg`, `..foo`, `a..b`) is fine; only a whole component that is exactly `.`
or `..` is rejected. A leading `/` is left as-is (it collapses under the base). If
the host cannot determine the executable path, `os::resourcePath` raises
`ErrUnsupported`. It reads host state only and has no side effects.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `relative` | `String` | The resource path below the build output (for example `music/song.ogg`); no `.`/`..` path component. [[src/builtins/os.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | The absolute on-disk path of the resource. [[src/builtins/os.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77030002` | `ErrInvalidPath` | `relative` contains a `.` or `..` path component. |
| `77050007` | `ErrUnsupported` | The host cannot determine the executable path. |
| `77010001` | `ErrOutOfMemory` | The host cannot allocate the returned `String`. |

## Examples

Open a resource shipped beside the program:

```
IMPORT os
IMPORT fs
IMPORT io

SUB main()
  LET path AS String = os::resourcePath("music/song.ogg")
  io::print(path)
END SUB
```

## See also

- `mfb man os executablePath`
- `mfb man os args`
