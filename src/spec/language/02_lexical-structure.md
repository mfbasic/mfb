# 2. Lexical Structure

- **Case-insensitive keywords**, case-sensitive identifiers. Convention: `camelCase` functions and built-in callable names, `CapitalCamelCase` types, `camelCase` bindings, `UPPERCASE` keywords.
- **Comments**: `'` to end of line, or `REM`.
- **Statement separator**: newline, or `:` for multiple statements on one line.
- **Line continuation**: trailing `_`.
- **Identifiers**: `[A-Za-z_][A-Za-z0-9_]*`. Legacy sigils (`$ % # !`) are removed.
- Identifiers are ASCII-only in this version. If a future version allows non-ASCII identifiers, compilers and language servers must lint Unicode confusables and near-collisions after Unicode normalization and case folding.

```basic
LET total = 0 : LET count = 0     ' two statements, one line
LET msg = "hello " & _
          "world"                 ' continuation
' this is a comment
REM so is this
```

The `:` separator is legal, but formatters and language servers should lint dense security-sensitive lines, especially lines that combine fallible calls, resource operations, native calls, permissioned filesystem/network operations, an inline `TRAP`, or `TRAP` control flow.

Identifiers are case-sensitive, so `userId` and `userid` are distinct. Tooling should lint near-collisions that differ only by case or visually minor spelling differences within the same scope or imported namespace.
