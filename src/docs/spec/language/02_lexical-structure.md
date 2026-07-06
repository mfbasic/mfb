# 2. Lexical Structure

- **Case-insensitive keywords**, case-sensitive identifiers. Convention: `camelCase` functions and built-in callable names, `CapitalCamelCase` types, `camelCase` bindings, `UPPERCASE` keywords. Keyword matching is case-insensitive ASCII (`eq_ignore_ascii_case`), so `func`, `FUNC`, and `Func` all lex to the same keyword.
- **Comments**: `'` to end of line. `REM` also begins a line comment, but **only when it is the first token of a statement** — that is, at the start of a line or immediately after a `:` separator. Anywhere else `REM` (and any identifier merely containing the letters `rem`) is an ordinary identifier. Both comment forms run to the end of the line; there is no block-comment syntax.
- **Statement separator**: newline (a `Newline` token), or `:` for multiple statements on one line.
- **Line continuation**: a trailing `_` followed only by whitespace and then a newline joins the next line; the lexer emits no `Newline` token there. A `_` that is not in trailing position lexes as (the start of) an identifier — e.g. the pipeline placeholder `_`.
- **Identifiers**: `[A-Za-z_][A-Za-z0-9_]*`, ASCII only (`is_ascii_alphanumeric() || '_'`). Legacy sigils (`$ % !`) are removed: they are not valid in identifiers and the lexer reports `MFB_LEX_UNEXPECTED_CHARACTER` for them. `#` is treated differently — it is **reserved**, not merely removed: it is rejected in user identifiers (also `MFB_LEX_UNEXPECTED_CHARACTER`) precisely so the compiler can use it as the untypeable internal sigil (see §2.4). There is no length limit.
- Identifiers are ASCII-only in this version. If a future version allows non-ASCII identifiers, compilers and language servers must lint Unicode confusables and near-collisions after Unicode normalization and case folding.

The full keyword set is: `AS CASE CONTINUE DO ELSE ELSEIF FALSE FAIL EXIT FOR EACH FUNC IF IN IMPORT ISOLATED LET LAMBDA LOOP DIV MOD MATCH MUT NOTHING AND OR NOT NEXT XOR RETURN SUB THEN TRUE END ENUM EXPORT PUBLIC PROGRAM PRIVATE PROPAGATE RECOVER RES STEP TO TYPE TRAP UNTIL UNION WHEN WHILE WEND WITH`. A keyword token may still be accepted in a name position (e.g. a native `LINK` function named `step`); definition and call sites both canonicalize to the keyword's lowercase lexeme so they match consistently.

```basic
LET total = 0 : LET count = 0     ' two statements, one line
LET msg = "hello " & _
          "world"                 ' continuation
' this is a comment
REM so is this
```

The `:` separator is legal, but formatters and language servers should lint dense security-sensitive lines, especially lines that combine fallible calls, resource operations, native calls, permissioned filesystem/network operations, an inline `TRAP`, or `TRAP` control flow.

Identifiers are case-sensitive, so `userId` and `userid` are distinct. Tooling should lint near-collisions that differ only by case or visually minor spelling differences within the same scope or imported namespace.

## 2.1 Numeric literals

The lexer reads one or more ASCII digits, optionally followed by a single `.` and one or more ASCII digits. A `.` is consumed as a decimal point **only** when a digit follows it, so `x.0` is `x . 0` (member access) and `1.foo` is `1 . foo`. There is no exponent (`1e9`), hexadecimal (`0x`), binary, sign, digit-separator (`1_000`), or type-suffix syntax at the lexical level; a number token is just its raw digit text. A leading `-` is the unary-minus operator, not part of the literal. Literal *typing* (untyped → `Integer`/`Float`/`Fixed`) is resolved later from context during type inference; see §4.1.

## 2.2 String literals and escapes

A string literal is delimited by `"`. It may not span a line — reaching a newline or end-of-file before the closing quote is a lexer error (`MFB_LEX_UNTERMINATED_STRING`). Inside a string, `\` introduces an escape. The lexer recognizes exactly four escapes:

| Escape | Produces |
|--------|----------|
| `\"`   | `"` |
| `\\`   | `\` |
| `\n`   | line feed (U+000A) |
| `\t`   | tab (U+0009) |

For **any other** escape, the lexer drops the backslash and keeps the following character verbatim. There is no `\r`, `\0`, `\xNN`, or `\u{...}` escape: `"\r"` lexes to the single character `r`, and `"\q"` lexes to `q`. (This is the source of the carriage-return gotcha noted in the implementation memory: a literal carriage return cannot be written with `\r`; build it from its byte/scalar value instead.) Escape handling is identical in every lexing mode — there is a single `lex_string` routine, so internal/source-package lexing drops `\r` exactly as ordinary lexing does.

## 2.3 `DOC` blocks

A `DOC` keyword at the start of a statement begins a documentation block whose body is captured **verbatim** as a single token, not tokenized as code, up to a matching `END DOC` line (`EXAMPLE`/`END EXAMPLE` regions inside the block are tracked so an `END DOC` inside an example is not treated as the terminator). The `DOC` keyword line may carry only whitespace-separated attribute words (e.g. `DOC INTERNAL`); if it carries anything else (`DOC = 1`, `DOC(x)`), the lexer rolls back and treats `DOC` as an ordinary identifier. An unterminated block reports `DOC_UNTERMINATED`. The full `DOC` surface and rendering are specified in §23.

## 2.4 Internal-file lexing and the `#` sigil

The lexer has an *internal* mode, selected when it lexes a file that ships as part of a built-in package's implementation (`lex_with(path, source, internal: true)`). In this mode, after an identifier is read but before it is classified, a **leading `__`** is rewritten to the reserved sigil `#`: `__json_parse` becomes `#json_parse`. (Keywords never carry a `__` prefix, so this only ever affects names; public package names with no `__` prefix — like the type `Json` — pass through untouched.)

Because `#` is rejected in every user identifier (§2.1 lists it as reserved), a user can never *author* a name containing the sigil. The internal rewrite is therefore unforgeable: an internalized name like `#json_parse` cannot collide with, or be shadowed by, anything a user writes. The sigil survives through the AST and IR and is mapped to a reserved native-symbol namespace at code generation; user-facing diagnostics map it back to the readable `__` form so the sigil never leaks into error messages. The full mechanism — internalization, sigil round-tripping, and the native-symbol mapping — is specified separately. [[src/lexer.rs:lex_with]] [[src/internal_name.rs:internalize]]

## See Also

* ./mfb spec architecture internal-naming — the full internalization mechanism: `__`→`#` rewrite, sigil round-tripping, and the reserved native-symbol mapping.
* ./mfb spec architecture native — the reserved `_mfb_ifn_…` native-symbol namespace that internalized `#` names lower to.
