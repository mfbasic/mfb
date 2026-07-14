# 2. Lexical Structure

- **Case-insensitive keywords**, case-sensitive identifiers. Convention: `camelCase` functions and built-in callable names, `CapitalCamelCase` types, `camelCase` bindings, `UPPERCASE` keywords. Keyword matching is case-insensitive ASCII (`eq_ignore_ascii_case`), so `func`, `FUNC`, and `Func` all lex to the same keyword.
- **Comments**: `'` to end of line. `REM` also begins a line comment, but **only when it is the first token of a statement** — that is, at the start of a line or immediately after a `:` separator. Anywhere else `REM` (and any identifier merely containing the letters `rem`) is an ordinary identifier. Both comment forms run to the end of the line; there is no block-comment syntax.
- **Statement separator**: newline (a `Newline` token), or `:` for multiple statements on one line.
- **Line continuation**: a trailing `_` followed only by whitespace and then a newline joins the next line; the lexer emits no `Newline` token there. A `_` that is not in trailing position lexes as (the start of) an identifier — e.g. the pipeline placeholder `_`.
- **Identifiers**: `[A-Za-z_][A-Za-z0-9_]*`, ASCII only (`is_ascii_alphanumeric() || '_'`). Legacy sigils (`$ % !`) are removed: they are not valid in identifiers and the lexer reports `MFB_LEX_UNEXPECTED_CHARACTER` for them. `#` is treated differently — it is **reserved**, not merely removed: it is rejected in user identifiers (also `MFB_LEX_UNEXPECTED_CHARACTER`) precisely so the compiler can use it as the untypeable internal sigil (see §2.4). There is no length limit.
- Identifiers are ASCII-only in this version. If a future version allows non-ASCII identifiers, compilers and language servers must lint Unicode confusables and near-collisions after Unicode normalization and case folding.

The full keyword set is: `AS CASE CONTINUE DO ELSE ELSEIF FALSE FAIL EXIT FOR EACH FUNC IF IN IMPORT ISOLATED LET LAMBDA LOOP DIV MOD MATCH MUT NOTHING AND OR NOT NEXT XOR RETURN SUB TESTING THEN TRUE END ENUM EXPORT PUBLIC PROGRAM PRIVATE PROPAGATE RECOVER RES STEP TO TYPE TRAP UNTIL UNION WHEN WHILE WEND WITH`. A keyword token may still be accepted in a name position (e.g. a native `LINK` function named `step`); definition and call sites both canonicalize to the keyword's lowercase lexeme so they match consistently.

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

The lexer reads one or more ASCII digits, optionally followed by a single `.` and one or more ASCII digits. A `.` is consumed as a decimal point **only** when a digit follows it, so `x.0` is `x . 0` (member access) and `1.foo` is `1 . foo`. A leading `-` is the unary-minus operator, not part of the literal. Literal *typing* (untyped → `Integer`/`Float`/`Fixed`) is resolved later from context during type inference; see §4.1.

**Radix prefixes.** A literal may begin with a base prefix — `0x`/`0X` (hexadecimal), `0o`/`0O` (octal), or `0b`/`0B` (binary), the prefix letter case-insensitive — followed by one or more base digits: `0xFFF` is 4095, `0o777` is 511, `0b1010` is 10. A radix literal is an ordinary untyped-`Integer` literal (the lexer canonicalizes it to decimal), so it types, range-checks, and lowers exactly as the equivalent decimal. There are no hex/oct/bin **floats**: after `0xFFF` a `.` is member access, exactly as `1.foo` is. A prefix with no digits (`0x`), or a digit outside the base (`0o8`, `0b2`, `0xG`), is a lexer error (`MFB_LEX_MALFORMED_NUMBER`); a magnitude above `u64::MAX` (e.g. a 17-digit hex literal) is `MFB_LEX_NUMBER_OUT_OF_RANGE`.

**Digit separators.** A single `_` may appear **between two digits** in any numeric run (`1_234`, `0xFF_FF`); it is stripped from the value, so `1_000_000` is `1000000`. A `_` that is not between two digits — leading (`_1` is an identifier, not a number), trailing (`1_`, unless it forms a line continuation), doubled (`1__2`), or adjacent to a prefix (`0x_1`) — is a lexer error (`MFB_LEX_MALFORMED_NUMBER`), except that a trailing `_` followed only by whitespace and a newline is the line-continuation token (§2, "Line continuation"), never a separator.

**Scientific notation.** After the integer part and optional `.`-fraction, an exponent — `e`/`E`, an optional `+`/`-`, then one or more digits (with `_` separators) — makes the literal a **Float**: `1e3`, `1e-3`, `2.5e2`, `1_0e1_0`. The `e`/`E` is consumed only when a well-formed exponent follows; otherwise it is not part of the number, so `1e` lexes as `1` then identifier `e`. Exponents are decimal only (no hex/oct/bin exponent). `1e400`, which parses to a non-finite `f64`, is `TYPE_FLOAT_LITERAL_OVERFLOW`.

**Type suffixes.** A single trailing `f` (Float), `F` (Fixed), or `m`/`M` (Money) sets the literal's type intrinsically: `2f`/`1.5f` are Float, `2F`/`1.5F` are Fixed, `2m`/`1.25m`/`1.25M` are Money, and a suffix composes with an exponent (`1e3f` Float, `1e3F`/`2.5e2F` Fixed, `1e2m` Money). Both `m` and `M` yield `Money` — there is only one money type, so unlike `f`/`F` the case is not load-bearing. The suffix is consumed only when not followed by an identifier-continue character, so `1foo`/`1motorcar` are `1` then an identifier but `1f`/`1m` are suffixed literals. Suffixes are decimal only — after a `0x…` hex scan an `f`/`F` is a hex *digit*, never a suffix. A `Fixed`-suffixed literal out of the Fixed range is `TYPE_FIXED_LITERAL_OVERFLOW` (and `-…F` underflow is `TYPE_FIXED_LITERAL_UNDERFLOW`); a `Money`-suffixed literal out of range is `TYPE_MONEY_LITERAL_OVERFLOW`/`UNDERFLOW`, and one with excess fractional precision warns `TYPE_MONEY_LITERAL_PRECISION`.

Unlike an *untyped* numeric literal — which coerces to a `Fixed`/`Byte` slot at assignment — a suffixed literal is *intrinsically* that type: the suffix always wins over the expected type, so `LET x AS Float = 2F` is a type error (a Fixed value into a Float slot), not a silent Float. `2F` is the only way to write an intrinsically-`Fixed` literal without an expected-type context. See `mfb spec language type-inference` (§ "Literal Coercion").

There is no sign inside a literal (`-0xFF` is `-(0xFF)`); a leading `-` is unary minus, and only the *exponent* may carry a `+`/`-` sign. There are no other suffixes beyond `f`/`F`/`m`/`M` (no `d`/`D`, no integer-width suffixes).

## 2.2 String literals and escapes

A string literal is delimited by `"`. It may not span a line — reaching a newline or end-of-file before the closing quote is a lexer error (`MFB_LEX_UNTERMINATED_STRING`). Inside a string, `\` introduces an escape. The lexer recognizes these escapes:

| Escape | Produces |
|--------|----------|
| `\"`   | `"` (U+0022) |
| `\\`   | `\` (U+005C) |
| `\n`   | line feed (U+000A) |
| `\t`   | tab (U+0009) |
| `\r`   | carriage return (U+000D) |
| `\0`   | NUL (U+0000) |
| `\u{HEX}` | the Unicode scalar with that hex codepoint |

`\u{HEX}` takes 1–6 hex digits between the braces (case-insensitive) and produces the single Unicode scalar with that codepoint, so `"\u{41}"` is `A` and `"\u{1F600}"` is 😀 (a 4-byte UTF-8 sequence). A malformed `\u{...}` escape — a missing `{`, no digits, more than 6 digits, an out-of-range magnitude, a missing closing `}` (including a newline or the closing `"` reached first), or a value that is not a Unicode scalar (a surrogate `U+D800..U+DFFF` or a codepoint above `U+10FFFF`) — is a lexer error (`MFB_LEX_INVALID_UNICODE_ESCAPE`). There is no `\xNN` two-digit-hex or fixed-width `\U########` form, and `\0` is exactly one NUL — a following digit is a literal digit, not an octal escape.

A `\` immediately before a newline is **not** a line continuation — no in-string continuation exists — and is the same lexer error as a bare newline (`MFB_LEX_UNTERMINATED_STRING`). A `\` at end-of-file is likewise unterminated.

## 2.3 Scalar literals

A **scalar literal** is delimited by backticks and holds exactly one Unicode scalar: `` `A` ``, `` `中` ``, `` `😀` ``. It lexes to the `Scalar` type (`mfb spec language types` §4.1). The backtick is otherwise unused in the grammar, so introducing it disturbs nothing — in particular the line-comment character `'` is unchanged, and `` `'` `` is simply the apostrophe scalar. The one scalar may be a raw source scalar or an escape, reusing the string-escape machinery: `` `\n` ``, `` `\t` ``, `` `\r` ``, `` `\0` ``, `` `\\` ``, the backtick escape `` `\`` ``, and `` `\u{HEX}` `` (1–6 hex digits, e.g. `` `\u{1F600}` `` is 😀). A `\u{...}` naming a surrogate or a value above `U+10FFFF` is `MFB_LEX_INVALID_UNICODE_ESCAPE`, exactly as in a string. An **empty** literal `` `` `` is `TYPE_SCALAR_LITERAL_EMPTY`; **more than one** scalar before the close (`` `ab` ``) is `TYPE_SCALAR_LITERAL_TOO_MANY`; an unterminated literal (newline or end-of-file before the closing backtick) or an unknown escape (`` `\q` ``) is `TYPE_SCALAR_LITERAL_INVALID`. [[src/lexer.rs:lex_scalar]]

For **any other** escape, the lexer drops the backslash and keeps the following character verbatim: `"\q"` lexes to `q`. Escape handling is identical in every lexing mode — there is a single `lex_string` routine, so internal/source-package lexing decodes `\r`, `\0`, and `\u{...}` exactly as ordinary lexing does.

A string carrying an embedded NUL (`\0`) is truncated at the NUL when handed to a C/syscall boundary that reads a NUL-terminated C string (e.g. a filesystem path); MFBASIC string operations that use the explicit `byteLength` (length, slicing, comparison, concatenation) see the full payload.

## 2.3 `DOC` blocks

A `DOC` keyword at the start of a statement begins a documentation block whose body is captured **verbatim** as a single token, not tokenized as code, up to a matching `END DOC` line (`EXAMPLE`/`END EXAMPLE` regions inside the block are tracked so an `END DOC` inside an example is not treated as the terminator). The `DOC` keyword line may carry only whitespace-separated attribute words (e.g. `DOC INTERNAL`); if it carries anything else (`DOC = 1`, `DOC(x)`), the lexer rolls back and treats `DOC` as an ordinary identifier. An unterminated block reports `DOC_UNTERMINATED`. The full `DOC` surface and rendering are specified in §21.

## 2.4 Internal-file lexing and the `#` sigil

The lexer has an *internal* mode, selected when it lexes a file that ships as part of a built-in package's implementation (`lex_with(path, source, internal: true)`). In this mode, after an identifier is read but before it is classified, a **leading `__`** is rewritten to the reserved sigil `#`: `__json_parse` becomes `#json_parse`. (Keywords never carry a `__` prefix, so this only ever affects names; public package names with no `__` prefix — like the type `Json` — pass through untouched.)

Because `#` is rejected in every user identifier (§2.1 lists it as reserved), a user can never *author* a name containing the sigil. The internal rewrite is therefore unforgeable: an internalized name like `#json_parse` cannot collide with, or be shadowed by, anything a user writes. The sigil survives through the AST and IR and is mapped to a reserved native-symbol namespace at code generation; user-facing diagnostics map it back to the readable `__` form so the sigil never leaks into error messages. The full mechanism — internalization, sigil round-tripping, and the native-symbol mapping — is specified separately. [[src/lexer.rs:lex_with]] [[src/internal_name.rs:internalize]]

## See Also

* ./mfb spec architecture internal-naming — the full internalization mechanism: `__`→`#` rewrite, sigil round-tripping, and the reserved native-symbol mapping.
* ./mfb spec architecture native — the reserved `_mfb_ifn_…` native-symbol namespace that internalized `#` names lower to.
