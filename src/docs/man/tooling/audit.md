# audit

Report a project's error handling, host capabilities, and dependency status

## Synopsis

```
mfb audit [--format text|json] [--locked] [path]
```

## Package

`tooling` — developer commands (`mfb man tooling`).

## Imports

None. `mfb audit` is a command-line tool, invoked from the shell.

## Description

`mfb audit` reads a project and reports what it does, without running it. The
report covers:

- **Permissions** — each host capability the program uses (filesystem, network,
  terminal, threads, process, environment, clock, randomness, audio, microphone,
  native libraries), with the call sites that use it.
- **Control flow** — each function, whether it can fail, and for every call that
  can fail, where its error goes: out of the function, or into a `TRAP`.
- **Dependencies** — whether each package in `project.json` is installed, valid,
  signed, and the requested version, and whether `mfb.lock` is present and up to
  date.
- **Findings** — a sorted list of coded findings, each an `error`, `warning`, or
  `info`: a missing or outdated dependency, an unsigned package, a package that
  exports mutable state, a resource whose close failure cannot be seen because
  it is closed only when its scope ends, a `tls::connect` that accepts
  self-signed certificates, and one `info` per capability used.

The checks run offline and read only the project and its installed packages.
The report starts with a summary count of errors, warnings, and infos. Text is
the default; `--format json` writes the same report as a JSON document for
tools.

The exit status says whether the report found a problem:

| Exit | Meaning |
| --- | --- |
| `0` | Report produced; no `error` findings. |
| `1` | Report produced; at least one `error` finding. |
| `2` | Bad command line: an unknown option, a missing or unknown `--format` value, or a second `path`. |
| `3` | The project could not be read: its manifest or source did not load, parse, or check. |

## Options

| Option | Description |
| --- | --- |
| `--format <type>` | `text` (default) or `json`. `--format=json` also works. |
| `--locked` | Treat a missing or out-of-date `mfb.lock` as an `error` finding instead of a `warning`. |

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `path` | directory | The project to audit. Defaults to the current directory. |

## Errors

No errors. `mfb audit` reports problems through its findings and exit status; it
does not raise MFBASIC runtime errors.

## Examples

Audit the project in the current directory:

```
mfb audit
```

A new project from `mfb init demo` prints, in part:

```
Summary:
  errors: 0
  warnings: 0
  infos: 1

Permissions:
  terminal
    io.print at src/main.mfb:4

Control flow:
  main at src/main.mfb:3 (fallible)
    fallible call io.print at src/main.mfb:4 -> return

Findings:
  info AUDIT-PERM-TERMINAL project uses host capability: terminal
```

Fail a CI job when the lockfile is missing or stale:

```
mfb audit --locked
```

## See also

- `mfb man tooling`
- `mfb man errors`
- `mfb spec tooling audit-format`
