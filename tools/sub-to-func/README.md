# sub-to-func

Mechanically rewrites MFBASIC source from `SUB` to `FUNC … AS Nothing` (plan-159).
plan-159-D removed `SUB` from the language: the compiler now reports an old `SUB`
declaration as an ordinary top-level parse error. Run this over source written before
that to migrate it. A `SUB` compiled to the same native code as the equivalent
`FUNC … AS Nothing` (measured in plan-159-A), so the rewrite changes no behaviour.

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

## MFBASIC inside Rust (`--rust-literals`)

```
python3 tools/sub-to-func/sub-to-func.py --rust-literals [--skip-fn REGEX] <file.rs | dir> ...
```

Rewrites MFBASIC held in Rust string literals (`r"…"`, `r#"…"#`, and ordinary
`"…"` strings with `\n`, `\"`, `\x20`, `\u{…}` escapes and `\`-newline line
continuations). A directory is searched for `*.rs`.

- A literal is treated as MFBASIC only if one of its lines is a `SUB` declaration
  header (optionally after `format!` placeholders such as `{prelude}`) or is exactly
  `END SUB` / `EXIT SUB`. Any other literal that mentions `SUB` — a diagnostic
  message, an assertion on one — is left alone and reported as `rust-other`.
- A literal containing a Markdown fence (a `mfb man` page) is rewritten only inside
  its fenced code blocks; `SUB` in the surrounding prose is reported as `doc-prose`.
- Only the changed keyword spans are edited, so the literal keeps its escapes exactly
  as written.
- `--skip-fn REGEX` leaves every literal inside a Rust `fn` whose text matches the
  regex, for tests whose purpose is a `SUB`-only rule. Skipped literals are reported
  as `skipped-fn`.
- `SUB` in Rust comments is reported as `rust-comment`.

## Report

Every `SUB` the tool leaves alone is printed as `path:line: [reason] text`:

- `comment` / `string` / `doc-text`: prose that mentions `SUB` (a `'` or `REM`
  comment, a string literal, DOC description text). Reword it by hand if it describes
  the code in that file.
- `unhandled-header` / `unhandled-code`: a declaration the tool cannot classify (for
  example a header whose parameter list spans lines) or a `SUB` token in code it has
  no rule for. The line is left unchanged and the exit status is `2`.
