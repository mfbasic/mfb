# tooling

Developer commands that operate on MFBASIC source and projects

## Synopsis

```
mfb man tooling [topic]
```

## Imports

`tooling` is a documentation topic for the `mfb` command-line tools, not an
importable package. No `IMPORT` is needed; the commands are invoked from the
shell as `mfb <command>`.

## Description

The `mfb` executable bundles the compiler together with source and project
tooling. This topic documents the developer-facing commands that read or rewrite
source rather than produce a build artifact. Each command is deterministic and
operates on files or directories given on the command line, defaulting to the
current directory when no path is supplied.

The reimplementable transformation rules for each command live in the language
spec under `mfb spec tooling`; this man topic is the quick command reference.

## Topics

- `fmt` — reformat source for consistent indentation and keyword capitalization.

## Errors

No errors. Tooling commands report problems on standard error and exit non-zero;
they do not raise MFBASIC runtime errors.

## See also

- `mfb man tooling fmt`
- `mfb spec tooling fmt`
