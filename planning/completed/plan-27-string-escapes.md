# MFBASIC String Escape Sequences Plan

Last updated: 2026-07-06
Effort: medium (1h–2h)

Today the lexer recognizes exactly four string escapes (`\"`, `\\`, `\n`, `\t`)
and passes every other `\X` through as the bare character `X` (dropping the
backslash). This plan extends the recognized set so a string literal can encode
a carriage return, a NUL, and any Unicode scalar directly:

| Escape | Produces | Status |
|--------|----------|--------|
| `\"`   | `"` (U+0022)              | exists |
| `\\`   | `\` (U+005C)              | exists |
| `\n`   | line feed (U+000A)       | exists |
| `\t`   | tab (U+0009)             | exists |
| `\r`   | carriage return (U+000D) | **new** |
| `\0`   | NUL (U+0000)             | **new** |
| `\u{HEX}` | the Unicode scalar with that hex codepoint | **new** |

The single behavioral outcome a correct implementation produces: `"\r"` lexes to
a one-character string holding U+000D, `"\0"` to U+0000, `"\u{1F600}"` to the
😀 emoji (a 4-byte UTF-8 sequence), and every previously-recognized escape is
unchanged.

It complements:

- `mfb spec language 02_lexical-structure` (§2.2 "String literals and escapes" —
  the authoritative escape table this plan rewrites; canonical source under
  `src/docs/spec/language/02_lexical-structure.md`).
- `mfb spec diagnostics 01_rule-codes` (the `1-101` lexer error-code block; this
  plan adds one code — canonical source under
  `src/docs/spec/diagnostics/01_rule-codes.md`, the build input for `errorCode::`).

## 1. Goal

- The lexer decodes `\r`, `\0`, and `\u{HEX}` in string literals as specified in
  the table above, in **every** lexing mode (ordinary and internal/source-package
  — there is one `lex_string` routine).
- Malformed `\u{...}` escapes produce a clear lexer diagnostic
  (`MFB_LEX_INVALID_UNICODE_ESCAPE`), never a silent wrong decode and never a
  panic.
- The four existing escapes and the "unknown escape drops the backslash"
  fallback are unchanged for escapes this plan does not claim.

### Non-goals (explicit constraints)

- **No `\x{...}` raw-byte escapes.** Removed by request; would violate the
  "every `String` is valid UTF-8" ingress invariant that the whole
  unicode/grapheme/normalization machinery relies on
  (`src/target/shared/code/private/unicode.rs` decoder comment).
- **No change to the value representation.** `TokenKind::String(String)` →
  `Expression::String(String)` → `IrValue::Const` →
  `CodeDataObject.value: String` stays a Rust `String` end-to-end. All new
  escapes produce valid UTF-8, so nothing downstream changes.
- **No change to string layout/ABI.** The `mfb.string.v1 { u64 byteLength; u8
  bytes[byteLength]; u8 nul }` layout (`src/target/shared/code/mod.rs:418`) is
  untouched; `byteLength` already counts payload bytes independently of the
  trailing NUL, so an embedded `\0` is representable without any layout change.
- **No new escape syntax beyond the table.** No octal (`\012`), no `\xNN`
  two-digit hex, no `\U00000000` fixed-width form. `\0` is exactly one NUL; a
  following digit is a literal digit.
- **No change to grammar productions** (`mfb spec language 19_grammar`) — escapes
  live entirely inside the lexer's string scanner.

## 2. Current State

`src/lexer.rs:284` `lex_string` scans between the delimiting `"`s, building a
Rust `String value`. On `\` (`src/lexer.rs:314`) it advances and matches the next
char:

```
'"'  => value.push('"'),
'\\' => value.push('\\'),
'n'  => value.push('\n'),
't'  => value.push('\t'),
_    => value.push(escaped),   // unknown: drop backslash, keep the char
```

A trailing `\` at EOF breaks the loop and reports `MFB_LEX_UNTERMINATED_STRING`.
The token value flows unchanged into `Expression::String(value)`
(`src/ast/expr.rs:397`), `IrValue::Const` (`src/ir/lower.rs:2602`), and finally
`CodeDataObject.value` (`src/target/shared/code/mod.rs:415-422`), whose size is
`align(8 + value.len() + 1, 8)` — length is explicit, the NUL is a terminator.

Spec `src/docs/spec/language/02_lexical-structure.md:28-39` documents "exactly
four escapes" and the "drops the backslash" fallback, and explicitly states there
is **no** `\r`, `\0`, `\xNN`, or `\u{...}` — this file is rewritten by the plan.

Diagnostics: the only lexer codes are `MFB_LEX_UNEXPECTED_CHARACTER` (`1-101-0001`)
and `MFB_LEX_UNTERMINATED_STRING` (`1-101-0002`)
(`src/docs/spec/diagnostics/01_rule-codes.md:224-229`). `self.report(code, msg,
line, start, end)` is the emit path (`src/lexer.rs:304`).

**Precedents to mirror:** the existing `\n`/`\t` arms (single-char decode) and
`self.report(...)` (diagnostic emit). For hex-scalar validation the Rust
`char::from_u32` guard (rejects surrogates and > U+10FFFF) is the exact check.

**Regression surface — built-in package sources.** Memory records that several
built-in packages *worked around* the missing `\r` by building CR from a byte
(`csv`, `regex`, `http`/`net`). Turning `\r`/`\0`/`\u` into real escapes changes
what `\r`, `\0`, `\u` mean inside those `.mfb` sources. Any built-in source that
wrote `\r` expecting the bare `r`, or `\0`/`\u` expecting pass-through, changes
behavior. This must be audited (Phase 1 task) — it is the highest regression risk
in the plan even though the lexer change is tiny.

## 3. Design Overview

Two independent, separately-landable pieces, ordered lowest-risk first:

1. **Single-char escapes `\r` and `\0`** — two new match arms, symmetric with the
   existing `\n`/`\t`. Trivial code; the *real* work is the built-in-source audit
   and the spec/test updates. Landable and valuable alone.
2. **`\u{HEX}` scalar escape** — a small sub-scanner: require `{`, read 1–6 hex
   digits, require `}`, parse to `u32`, validate via `char::from_u32`, push the
   `char`. Adds one diagnostic code. This is where the correctness risk
   concentrates (malformed input, surrogates, overflow, unterminated brace at
   EOL/EOF).

Both pieces are confined to `lex_string` in `src/lexer.rs`. No other compiler
stage changes because every output is a valid UTF-8 `char`.

## 4. Detailed Design

### 4.1 Single-char escapes (Phase 1)

Add to the `match escaped` block:

```
'r' => value.push('\r'),   // U+000D
'0' => value.push('\0'),   // U+0000
```

`'\0'` is a valid `char` and Rust `String` holds it (U+0000 is valid UTF-8). The
NUL lands in the payload; `byteLength` counts it. **Caveat to document:** a string
carrying an embedded NUL is truncated at the NUL when handed to a C/syscall
boundary that reads a NUL-terminated C string (e.g. a file path passed to
`open`). This is inherent to any NUL-in-string feature and is a documentation
note, not a blocker — MFBASIC string ops that use the explicit `byteLength`
(length, slicing, comparison, concatenation) see the full payload.

### 4.2 `\u{HEX}` scalar escape (Phase 2)

On matching `'u'`, run a sub-scanner (all cursor moves via the existing
`advance`/`peek`/`is_at_end` helpers, so column/line tracking stays correct):

1. Require the next char to be `{`; otherwise report `MFB_LEX_INVALID_UNICODE_ESCAPE`
   ("`\\u` must be followed by `{`").
2. Read hex digits (`0-9a-fA-F`) into a scratch accumulator. Require **1–6**
   digits. Zero digits → error ("`\\u{}` needs at least one hex digit");
   more than 6, or a value that overflows, → error ("Unicode escape out of
   range").
3. Require the closing `}`; a newline, the closing `"`, or EOF before `}` →
   `MFB_LEX_INVALID_UNICODE_ESCAPE` ("unterminated `\\u{...}` escape"). (Reaching
   the line/file end still also surfaces the unterminated-string path; the escape
   error is reported first and is more specific.)
4. Parse the accumulated digits to `u32`, then `char::from_u32(cp)`:
   `Some(c)` → `value.push(c)`; `None` (surrogate `D800..=DFFF` or `> 10FFFF`) →
   error ("Unicode escape `U+{cp:X}` is not a valid scalar value").

On any error, report via `self.report(...)` at the escape's span and stop lexing
the token (mirror the existing early-return-on-report shape) — do not push a
partial value that could mask the error downstream.

`char::from_u32` is the single source of truth for validity: it rejects exactly
the surrogates and the out-of-range codepoints, matching the UTF-8 invariant the
rest of the compiler assumes.

## Layout / ABI Impact

None. String layout, copy/move semantics, thread-transfer rules, and golden data
encoding are unchanged. The value pipeline stays `String`. `mfb spec memory` and
`mfb spec package` need no edits. Only `mfb spec language` (escape table) and
`mfb spec diagnostics` (one new code) change.

## Phases

### Phase 1 — `\r` and `\0` single-char escapes

Delivers the two trivial escapes and the built-in-source audit that makes them
safe. Safe to land alone: no new syntax, no new error code.

- [ ] Add `'r' => value.push('\r')` and `'0' => value.push('\0')` arms to the
      `match escaped` block in `lex_string` (`src/lexer.rs:320-326`).
- [ ] **Audit built-in package sources** for existing `\r`, `\0`, or `\u`
      sequences whose meaning changes: `grep -rn '\\r\|\\0\|\\u' src/**/*.mfb`
      (and any generated/embedded package source). For each hit, confirm the new
      meaning is intended or rewrite the source to preserve behavior (e.g. a
      package that built CR from a byte can now use `\r`, but must not
      double-emit). Record the audited files in the commit message.
- [ ] Rewrite the escape table and prose in
      `src/docs/spec/language/02_lexical-structure.md:28-39`: add `\r` and `\0`
      rows, drop the "no `\r`/`\0`" claim, keep the "unknown escape drops the
      backslash" fallback wording, and remove/replace the carriage-return gotcha
      note (a literal CR can now be written `\r`).
- [ ] Update the `.ai` / implementation memory carriage-return gotcha note so it
      no longer says `\r` is impossible.
- [ ] Tests: extend `src/lexer.rs` unit tests (mirror
      `string_escapes_are_decoded_including_unknown_escapes` at
      `src/lexer.rs:960`) to cover `"\r"` → U+000D and `"\0"` → U+0000, and add
      a runtime acceptance case under the existing string-literal test area
      proving the bytes reach output (e.g. `PRINT` a string with `\r`/`\0` and
      assert the emitted bytes).

Acceptance: `"\r"` and `"\0"` decode to single-char U+000D / U+0000 strings in a
lexer unit test **and** in a compiled+run acceptance program; the built-in-source
audit is complete with no unintended behavior change (full acceptance stays
green); spec table shows the two new rows.
Commit: —

### Phase 2 — `\u{HEX}` scalar escape (highest-risk)

Delivers the Unicode-scalar escape and its diagnostic. Last because it adds a
sub-scanner and a new error code.

- [ ] Implement the `'u'` sub-scanner in `lex_string` per §4.2, using
      `advance`/`peek`/`is_at_end` and `char::from_u32` (`src/lexer.rs`).
- [ ] Add the diagnostic code `MFB_LEX_INVALID_UNICODE_ESCAPE` as `1-101-0003` to
      the `1-101` lexer table in
      `src/docs/spec/diagnostics/01_rule-codes.md:224-229`, and wire the
      `self.report("MFB_LEX_INVALID_UNICODE_ESCAPE", ...)` calls for each failure
      mode (missing `{`, empty, too long / overflow, unterminated, invalid
      scalar).
- [ ] Add the `\u{HEX}` row to the escape table in
      `src/docs/spec/language/02_lexical-structure.md` and document the `\u{...}`
      grammar and its error cases in the prose.
- [ ] Tests — lexer unit tests: valid `"\u{41}"`→`A`, `"\u{1F600}"`→😀
      (4-byte), lowercase/uppercase hex, 1-digit and 6-digit bounds; **invalid**:
      `"\u41"` (no brace), `"\u{}"` (empty), `"\u{110000}"` (out of range),
      `"\u{D800}"` (surrogate), `"\u{1F600"` (unterminated at quote/EOL). Each
      invalid case asserts `MFB_LEX_INVALID_UNICODE_ESCAPE`.
- [ ] Tests — acceptance: a compiled program printing `"\u{1F600}"` emits the
      exact 4 UTF-8 bytes `F0 9F 98 80`; an invalid escape fails the build with
      the new code.

Acceptance: valid `\u{...}` escapes decode to the correct scalar (unit + runtime
byte-exact proof); every malformed form reports `MFB_LEX_INVALID_UNICODE_ESCAPE`
and never panics; `errorCode::` sees the new code; full acceptance green.
Commit: —

## Validation Plan

- Lexer unit tests: all valid and invalid escape forms above (Phases 1–2).
- Runtime proof: a compiled+run program that `PRINT`s strings containing `\r`,
  `\0`, and `\u{1F600}` and whose emitted bytes are asserted byte-exact
  (`0D`, `00`, `F0 9F 98 80`) — not just golden text.
- Doc sync: `src/docs/spec/language/02_lexical-structure.md` (escape table +
  prose) and `src/docs/spec/diagnostics/01_rule-codes.md` (new `1-101-0003`).
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Open Decisions

- **Unknown-escape fallback** — *keep the current "drop backslash, pass the char
  through" behavior* (recommended) vs. turning unknown escapes into a lexer error.
  Keeping it is backward-compatible and preserves the existing
  `string_escapes_are_decoded_including_unknown_escapes` test; only `\r`, `\0`,
  `\u` change meaning (all currently pass-through, now claimed). (§2, §4)
- **Embedded-NUL at C boundaries** — *document the truncation caveat* (recommended)
  vs. rejecting `\0` in strings used as syscall paths. Rejecting requires flow
  analysis the lexer cannot do; a doc note is the right layer. (§4.1)

## Non-Goals

- `\x{...}` raw-byte escapes (removed by request — would break the UTF-8 invariant).
- Octal, `\xNN`, or fixed-width `\U########` forms.
- Any change to the value representation, string layout, or grammar.

## Summary

The engineering risk is not the lexer edit (a handful of match arms plus a small
hex sub-scanner) — it is (1) the built-in package sources that historically
worked around the missing `\r`, whose meaning silently shifts, and (2) the
`\u{...}` validation surface (surrogates, overflow, unterminated braces). Both are
contained: `char::from_u32` is the single validity oracle, and the audit is a
mechanical grep-and-check. Everything downstream of the lexer is untouched because
every new escape yields valid UTF-8, so the `String`-carried value pipeline,
string layout, and golden encoding are all unaffected.
