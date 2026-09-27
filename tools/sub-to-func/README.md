# sub-to-func

Mechanically rewrites MFBASIC source from `SUB` to `FUNC … AS Nothing` (plan-159).
A `SUB` and the equivalent `FUNC … AS Nothing` compile to the same native code, so
the rewrite changes no behaviour.

## Usage

```
python3 tools/sub-to-func/sub-to-func.py [--check] [--quiet] <file.mfb | dir> ...
```

A directory is searched recursively for `*.mfb` (skipping `.git`, `target` and
`build`). Files are rewritten in place; line endings are preserved.

- `--check` writes nothing and exits `1` if any file would change. Running the tool
  twice leaves the second run with nothing to change (`--check` exits `0`).
- `--quiet` prints only the `unhandled` report lines.

Exit status: `0` success, `1` (`--check` only) something would change, `2` at least one
line could not be classified (see Report).

## The rewrites

Applied to code only, never inside a string, a char literal or a comment:

| Before | After |
|---|---|
| `<vis>? ISOLATED? SUB <header>` | `<vis>? ISOLATED? FUNC <header> AS Nothing` |
| `END SUB` | `END FUNC` |
| `EXIT SUB` | `RETURN` |

`<vis>` is `PUBLIC`, `PRIVATE` or `EXPORT`. The header keeps its name, `OF T` template
parameters and parameter list (including string default values); `AS Nothing` is
inserted after the closing `)`, or after the name when there is no parameter list.

Inside a `DOC … END DOC` block, the header line `SUB name(Type, …)` becomes
`FUNC name(Type, …)` (a doc header names the callable and carries no return type),
and the code between `EXAMPLE` and `END EXAMPLE` is rewritten like any other code.

## Report

Every `SUB` the tool leaves alone is printed as `path:line: [reason] text`:

- `comment` / `string` / `doc-text`: prose that mentions `SUB` (a `'` or `REM`
  comment, a string literal, DOC description text). Reword it by hand if it describes
  the code in that file.
- `unhandled-header` / `unhandled-code`: a declaration the tool cannot classify (for
  example a header whose parameter list spans lines) or a `SUB` token in code it has
  no rule for. The line is left unchanged and the exit status is `2`.
