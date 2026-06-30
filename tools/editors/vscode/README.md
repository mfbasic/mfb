# MFBASIC for Visual Studio Code

Syntax highlighting (a TextMate grammar) for the MFBASIC language. Colors `.mfb`
source files: keywords, types, strings, numbers, comments, `DOC` blocks,
`package::name` calls, and operators.

This is highlighting only — no completions, diagnostics, or formatting. Those
would come from a language server (see "Roadmap" below).

## Layout

```
package.json                     extension manifest (language + grammar contributions)
language-configuration.json      comments, brackets, auto-close, indentation
syntaxes/mfbasic.tmLanguage.json the TextMate grammar (scope: source.mfbasic)
examples/sample.mfb              a showcase file for eyeballing the grammar
```

## Try it without installing

1. Open this folder (`tools/editors/vscode`) in VS Code.
2. Press `F5` to launch an **Extension Development Host** window.
3. In that window, open `examples/sample.mfb` (or any `.mfb` file).

To inspect the scope assigned to the token under the cursor, run
**Developer: Inspect Editor Tokens and Scopes** from the Command Palette. This is
the fastest way to debug a mis-colored token.

## Install locally

Package it into a `.vsix` and install:

```sh
npm install -g @vscode/vsce      # one-time
cd tools/editors/vscode
vsce package                     # produces mfbasic-0.1.0.vsix
code --install-extension mfbasic-0.1.0.vsix
```

## What the grammar knows (and why)

The grammar mirrors `src/lexer.rs`. The MFBASIC-specific rules worth knowing:

- **Keywords are case-insensitive.** `FUNC` and `func` both color. The
  convention is UPPERCASE keywords, `CapitalCamelCase` types, `camelCase`
  functions/bindings — the grammar leans on that convention to color
  user-defined types.
- **`DOC ... END DOC` blocks are verbatim.** Their body is scoped as
  documentation, not code — matching the lexer, which slurps the whole block as
  one token. An inner `EXAMPLE ... END EXAMPLE` region re-injects code
  highlighting.
- **Two comment forms:** `'` anywhere, and `REM` only at the start of a
  statement (line start or after a `:`).
- **Strings** are double-quoted, single-line, with exactly four escapes
  (`\"  \\  \n  \t`); any other `\x` just drops the backslash. The string rule
  ends at the line break so an unterminated quote doesn't bleed downward.
- **Numbers** are plain decimal (`42`, `1.5`). There is no hex/binary/exponent/
  underscore form, so the grammar deliberately doesn't match them — coloring an
  invalid literal as valid would be misleading.
- **`package::name`** colors the package as a namespace and `::` as a separator.

## Customizing colors

The grammar assigns *scopes*; your active theme maps scopes to colors. To recolor
a token group without changing themes, add a `textMateRules` override to
`settings.json`. Scope it to `.mfbasic` so it only affects MFBASIC files:

```jsonc
"editor.tokenColorCustomizations": {
  "textMateRules": [
    { "scope": "keyword.declaration.mfbasic", "settings": { "foreground": "#569CD6" } }
  ]
}
```

The scopes worth knowing:

| Tokens | Scope |
|---|---|
| `IF`/`FOR`/`RETURN`/`END IF`… (control flow) | `keyword.control.mfbasic` |
| `FUNC`/`SUB`/`LET`/`MUT`/`AS`/`TYPE`/`END FUNC`… (declarations) | `keyword.declaration.mfbasic` |
| `AND`/`OR`/`NOT`/`DIV`/`MOD` | `keyword.operator.word.mfbasic` |
| `TRUE`/`FALSE`/`NOTHING` | `constant.language.mfbasic` |
| `Integer`/`String`/`List`… (builtin types) | `support.type.builtin.mfbasic` |
| `Point`/`Color` (user types) | `entity.name.type.mfbasic` |
| `strings` in `strings::mid` (package prefix) | `entity.name.namespace.mfbasic` |
| `mid`/`buildMasked` (calls) | `entity.name.function.mfbasic` |
| numbers | `constant.numeric.mfbasic` |
| strings | `string.quoted.double.mfbasic` |

Use **Developer: Inspect Editor Tokens and Scopes** to see the scope on any token.

`END <modifier>` is scoped to match its companion keyword, so `END TYPE` colors
`END` the same as `TYPE` (declaration) and `END IF` colors `END` the same as `IF`
(control).

## Keeping it in sync

The grammar is versioned in-tree alongside the compiler on purpose. When the
lexer's keyword set, comment rules, or `DOC` handling change in `src/lexer.rs`,
update `syntaxes/mfbasic.tmLanguage.json` to match.

## Roadmap

- **Tree-sitter grammar** for Neovim / Helix / Zed-native / GitHub semantic
  highlighting.
- **Language server** (`tower-lsp`, reusing the existing Rust lexer / parser /
  type-checker) for diagnostics, hover, and go-to-definition.
