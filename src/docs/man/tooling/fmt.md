# fmt

Reformat MFBASIC source for consistent indentation and capitalization

## Synopsis

```
mfb fmt [options] [path]
```

## Package

`tooling` — developer commands (`mfb man tooling`).

## Imports

None. `mfb fmt` is a command-line tool, invoked from the shell.

## Description

`mfb fmt` rewrites MFBASIC source in place, normalizing only **leading
whitespace** and **keyword capitalization**. Keywords are uppercased to match
the MFBASIC convention; identifiers, package members (`pkg::name`), and field
accesses (`value.field`) are left untouched even when they spell a keyword.
String contents, comments, blank lines, and the interiors of `DOC` and `LINK`
block bodies are preserved.

The formatter is deliberately lexical, not AST-based: it re-tokenizes each
physical line with a small scanner rather than parsing, so comments and blank
lines survive and a malformed line is still re-emitted rather than rejected.
The transform is pure and deterministic — the same input and indent width always
produce the same output.

Indentation is computed as `level * indent-width` spaces, so the formatter never
emits a tab for computed indentation. A line-continuation line (one following a
line that ends in a trailing `_`) keeps its original leading whitespace and has
only its trailing whitespace stripped.

With no `path`, `mfb fmt` formats the current directory. A directory argument is
walked recursively for MFBASIC source files.

## Options

| Option | Description |
| --- | --- |
| `--check` | Report whether files are already formatted without writing changes; exit non-zero if any file would change. |
| `--indent <N>` | Number of spaces per indentation level (default: `2`). |

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `path` | file or directory | What to format. Defaults to the current directory. |

## Errors

No errors. `mfb fmt` reports problems on standard error and exits non-zero; it
does not raise MFBASIC runtime errors.

## Examples

Format a whole project in place:

```
mfb fmt
```

Check formatting in CI without rewriting, using a four-space indent:

```
mfb fmt --check --indent 4 src
```

## See also

- `mfb man tooling`
- `mfb spec tooling fmt`
