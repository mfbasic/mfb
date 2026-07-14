# plan-28-A: Radix prefixes & digit separators

Last updated: 2026-07-06
Overall Effort: large (3h–1d)
Effort: medium (1h–2h)

Today the lexer reads a numeric literal as "one or more ASCII digits, optionally
`.` then more digits", and emits the raw digit text as `TokenKind::Number(String)`
(`src/lexer.rs:343` `lex_number`). This sub-plan extends the *integer-shaped*
surface of numeric literals: base prefixes and digit separators.

| Form         | Example      | Means            | Status |
|--------------|--------------|------------------|--------|
| Decimal      | `999`        | Integer 999      | exists |
| Hexadecimal  | `0xFFF`      | Integer 4095     | **new** |
| Octal        | `0o777`      | Integer 511      | **new** |
| Binary       | `0b1010`     | Integer 10       | **new** |
| Separators   | `1234_124_2342` | Integer 12341242342 | **new** |

The single behavioral outcome a correct implementation produces: `0xFFF`,
`0o777`, `0b1010`, and `1_234` each lex to a single `Number` token whose value is
the **canonical decimal integer text** (`"4095"`, `"511"`, `"10"`, `"1234"`), and
that value flows through the existing Integer-literal typing and codegen paths
byte-for-byte identically to writing the decimal form directly. Malformed forms
(`0x`, `0b2`, `1__2`, `_1`, `1_`) produce a clear lexer diagnostic, never a
silent wrong decode and never a panic.

The whole `plan-28` feature (this sub-plan **A** plus **B**, scientific notation &
type suffixes) is a **large** effort split by risk: A is everything that reduces
to today's untyped-Integer literal (lexer-contained, zero typing/representation
change); B introduces the new standalone-Fixed literal type that the current
`contains('.')` string-shape heuristic cannot express.

It complements:

- `mfb spec language lexical-structure` (§2.1 "Numeric literals" — the
  authoritative paragraph this plan rewrites; it currently states "There is no
  exponent, hexadecimal, binary, sign, digit-separator, or type-suffix syntax".
  Canonical source `src/docs/spec/language/02_lexical-structure.md`).
- `mfb spec diagnostics 01_rule-codes` (the `1-101` lexer error-code block; this
  plan adds lexer codes — canonical source
  `src/docs/spec/diagnostics/01_rule-codes.md:224-229`, the build input for
  `errorCode::`).
- `mfb spec language type-inference` (§ "Literal Coercion" — untyped-Integer
  literals coerce to Byte/Fixed at typed slots; radix/separator literals are
  ordinary untyped-Integer literals and inherit that behavior unchanged).

## 1. Goal

- The lexer recognizes `0x`/`0X`, `0o`/`0O`, `0b`/`0B` prefixes (case-insensitive
  prefix letter) followed by one or more base digits, and `_` digit separators
  between digits in any numeric form, and emits a single `Number` token whose
  value is the canonical **decimal** integer text.
- A radix/separator literal is an ordinary **untyped-Integer** literal: it types,
  range-checks (`TYPE_INTEGER_LITERAL_OVERFLOW`), coerces to Byte/Fixed at typed
  slots, and lowers exactly as the equivalent decimal literal — no downstream
  representation or codegen change.
- Every malformed form reports a specific lexer diagnostic and never panics:
  a prefix with no digits (`0x`), a digit outside the base (`0o8`, `0b2`,
  `0xG`), a misplaced/doubled/trailing/leading separator (`_1`, `1_`, `1__2`,
  `0x_1`), and a radix value that overflows the Integer range.
- A single decode/classify helper in `src/numeric.rs` becomes the one place that
  turns a `Number` token/literal string into (canonical value, literal type).
  This sub-plan lands it with the integer arms complete and the float/fixed arms
  delegating to today's `contains('.')` behavior (B fills them in).

### Non-goals (explicit constraints)

- **No new numeric *value* representation.** `TokenKind::Number(String)` →
  `Expression::Number(String)` → `IrValue::Const { type_, value }` stays a Rust
  `String` end to end. The value string is the *canonical decimal integer text*,
  so all ~50 existing `Expression::Number` consumers (`src/ir/lower.rs:378,395`,
  `2016`, `2606`, `3444`; `src/monomorph/lower.rs:1428`;
  `src/syntaxcheck/helpers.rs` `numeric_literal_type`) keep working with their
  current `parse::<i64>()` / `contains('.')` logic.
- **No float or Fixed surface here.** No exponent, no `f`/`F` suffix — those are
  plan-28-B. A radix or separator literal is always Integer-shaped. `0x1.8`
  (hex float) is **not** supported: after `0xFFF` the `.` is member access exactly
  as `1.foo` is today (§2.1).
- **No change to literal typing rules.** The untyped→Integer/Float/Fixed inference
  and the `expression_compatible` literal-coercion lattice
  (`mfb spec language type-inference`) are unchanged; radix/separator literals
  enter that machinery as untyped-Integer literals.
- **No change to grammar productions** (`mfb spec language grammar`) beyond the
  numeric-literal terminal — prefixes and separators live entirely inside the
  lexer's number scanner.
- **No negative literals.** A leading `-` stays the unary-minus operator, not part
  of the literal (§2.1), including for radix forms: `-0xFF` is `-(0xFF)`.

## 2. Current State

`src/lexer.rs:343` `lex_number` is the sole numeric scanner, reached from the
main loop on `'0'..='9'` (`src/lexer.rs:182`):

```rust
fn lex_number(&mut self) {
    let line = self.line;
    let start = self.column;
    let mut value = String::new();
    while !self.is_at_end() && self.peek().is_ascii_digit() {
        value.push(self.peek());
        self.advance();
    }
    if !self.is_at_end()
        && self.peek() == '.'
        && self.peek_next().is_some_and(|ch| ch.is_ascii_digit())
    {
        value.push(self.peek());
        self.advance();
        while !self.is_at_end() && self.peek().is_ascii_digit() { value.push(self.peek()); self.advance(); }
    }
    self.tokens.push(Token { kind: TokenKind::Number(value), line, start, end: self.column });
}
```

The raw text flows into `Expression::Number(value)`
(`src/ast/expr.rs:398`). Literal typing is decided **downstream** by the presence
of a `.`:

- `src/ir/lower.rs:2606-2617` — `IrValue::Const` type is `expected` (Fixed/Byte),
  else `value.contains('.')` → Float, else Integer.
- `src/ir/lower.rs:2016-2021` and `src/monomorph/lower.rs:1428` — same
  `contains('.')` split for the inferred literal type.
- `src/syntaxcheck/helpers.rs` `numeric_literal_type` — `contains('.')` → Float,
  else Integer (also peels a leading unary `-`).
- Value parsing: `src/ir/lower.rs:378` `text.parse::<i64>().unwrap_or(0)`,
  `:395` `IrLinkExpr::Int(text.parse::<i64>())`, and `IrValue::Const.value` is the
  raw string handed to codegen.

Range checks live in `src/ir/verify/mod.rs:1564` `check_const_literal` /
`:1606` `check_negated_const_literal`: for a non-`.` value they
`parse::<i64>()` / `parse::<u16>()` and emit `TYPE_INTEGER_LITERAL_OVERFLOW` /
`TYPE_BYTE_LITERAL_OVERFLOW` on failure. Because those parse plain decimal, the
lexer must hand them **decimal** text — decoding hex/oct/bin to decimal in the
lexer is what keeps these checks working unchanged.

Numeric type constants (`Byte`/`Fixed`/`Float`/`Integer`) and the promotion table
live in `src/numeric.rs` — the natural home for the new decode helper.

Lexer diagnostics today: only `MFB_LEX_UNEXPECTED_CHARACTER` (`1-101-0001`) and
`MFB_LEX_UNTERMINATED_STRING` (`1-101-0002`)
(`src/docs/spec/diagnostics/01_rule-codes.md:224-229`); `self.report(code, msg,
line, start, end)` is the emit path.

**Precedent to mirror:** plan-27 (string escapes) — a lexer-contained feature that
added match arms plus one diagnostic code, rewrote the §2.2 spec table, and proved
behavior with lexer unit tests plus a runtime acceptance case. This plan mirrors
that shape for §2.1.

## 3. Design Overview

Three pieces, ordered lowest-risk first, all confined to the lexer plus one new
`src/numeric.rs` helper:

1. **`numeric::classify_literal` helper (foundation).** One function:
   `classify_literal(text: &str) -> Result<(String /*canonical value*/, LiteralType), _>`
   where `LiteralType ∈ {Integer, Float, Fixed}`. It is the single source of truth
   that replaces the scattered `contains('.')` checks. This sub-plan lands it with
   the **Integer** path complete (plain decimal in → decimal out, Integer) and the
   float/fixed arms delegating to `contains('.')` so nothing regresses; B extends
   it. The lexer produces already-canonical decimal text for radix forms, so the
   helper's job here is classification, and the *decode* lives in the lexer where
   the span is known for diagnostics.
2. **Digit separators.** `_` accepted between two base digits in every numeric
   run (decimal integer part, and later fraction/exponent in B). Stripped from the
   emitted value. Leading `_`, trailing `_`, doubled `__`, and separator adjacent
   to the prefix or the `.` are lexer errors. (Trailing `_` at end-of-line is
   already the line-continuation token — see Open Decisions.)
3. **Radix prefixes.** `0x`/`0o`/`0b` (case-insensitive prefix letter) scan
   base-appropriate digits (with separators), decode to a `u128`, range-check, and
   emit the **decimal** string. Highest risk of the three (new scanning branch,
   overflow, empty-digit and bad-digit diagnostics), so it lands last behind the
   other two.

The correctness risk concentrates in (a) decoding radix → decimal without
overflow surprises and (b) the separator-placement rules producing precise
diagnostics rather than silent acceptance. Both are lexer-local and fully unit
testable.

## 4. Detailed Design

### 4.1 `numeric::classify_literal` (Phase 1)

Add to `src/numeric.rs`:

```rust
pub(crate) enum LiteralType { Integer, Float, Fixed }

/// Classify a *canonical* numeric-literal string (as emitted by the lexer:
/// separators already stripped, radix already decoded to decimal, optional
/// trailing `.`/exponent/suffix) into its literal type and a suffix-free,
/// parse-ready value string. The single source of truth for numeric-literal
/// typing, replacing the scattered `value.contains('.')` checks.
pub(crate) fn classify_literal(text: &str) -> (String, LiteralType) { ... }
```

Phase-1 body: no suffix and no exponent exist yet, so it is exactly today's rule —
`contains('.')` → `(text, Float)`, else `(text, Integer)`. Route the three typing
sites (`src/ir/lower.rs:2016`, `:2606`, `src/monomorph/lower.rs:1428`) and
`numeric_literal_type` (`src/syntaxcheck/helpers.rs`) through it so the "one place"
invariant holds before B adds exponent/suffix arms. This is a pure refactor with
byte-identical output — provable by full acceptance staying green.

### 4.2 Digit separators (Phase 2)

In the decimal-digit loop(s) of `lex_number`, accept `_` **only** when it sits
between two base digits: peek is `_` **and** `peek_next` is a base digit **and**
the previously consumed char was a base digit. On acceptance, `advance` past the
`_` without pushing it to `value`. Any `_` that fails the between-digits test ends
the number scan; if it is adjacent to digits in a way that indicates intent
(`1_`, `1__2`), emit `MFB_LEX_MALFORMED_NUMBER` at the separator span.

Careful interaction with line continuation (`src/lexer.rs` — a trailing `_`
followed only by whitespace then newline is the continuation token, §2). The
separator rule requires a following **digit**, so a trailing `_` never matches the
separator branch and continuation is preserved. `1_` immediately before a newline
is therefore ambiguous with "continuation after the number `1`"; resolve per Open
Decisions (recommend: continuation wins — `_` at number-end is not a separator).

### 4.3 Radix prefixes (Phase 3)

When `lex_number` starts on `0` and `peek_next` is `x`/`X`/`o`/`O`/`b`/`B`:
consume `0` and the prefix letter, then scan base digits (with §4.2 separators)
into a scratch string:

- base 16: `0-9 a-f A-F`; base 8: `0-7`; base 2: `0-1`.
- **Zero digits** after the prefix → `MFB_LEX_MALFORMED_NUMBER`
  ("`0x` needs at least one hex digit").
- A digit outside the base (`0o8`, `0b2`, `0xG`) — if it is a base-10 digit or
  ASCII letter that looks intended → `MFB_LEX_MALFORMED_NUMBER`
  ("invalid digit `8` in octal literal"); otherwise it simply ends the literal
  (e.g. `0b10 + 1`).

Decode the scratch digits with `u128::from_str_radix`, then emit the **decimal**
string via `to_string()`. Overflow handling per Open Decisions
(recommend: decode into `u128`; if the value exceeds `i64::MAX` as an unsigned
magnitude beyond what a following unary `-` could still make valid, emit
`MFB_LEX_NUMBER_OUT_OF_RANGE` at the literal span — a lexer error, because the
`i64`-based `TYPE_INTEGER_LITERAL_OVERFLOW` check can't see the original radix
text). Base-16 `0xFFFFFFFFFFFFFFFF` (u64 max) is the motivating case.

The emitted token value is pure decimal, so `Expression::Number("4095")` is
indistinguishable downstream from a user writing `4095`.

## Layout / ABI Impact

None. No value representation, string/collection layout, copy/move/transfer rule,
or golden data encoding changes — the token value is canonical decimal text, so
`IrValue::Const` and `CodeDataObject` see exactly what an equivalent decimal
literal produces. `mfb spec memory` and `mfb spec package` need no edits. Only
`mfb spec language` (§2.1 rewrite) and `mfb spec diagnostics` (new lexer codes)
change.

## Phases

### Phase 1 — `classify_literal` helper (refactor, zero behavior change)

Lands the single source of truth for literal typing with today's semantics.

- [ ] Add `LiteralType` and `classify_literal` to `src/numeric.rs` with the
      Integer/Float(`contains('.')`) arms and unit tests.
- [ ] Route `src/ir/lower.rs:2016-2021`, `:2606-2617`, `src/monomorph/lower.rs:1428`,
      and `src/syntaxcheck/helpers.rs` `numeric_literal_type` through it (preserving
      the `expected == Fixed/Byte` precedence at lower.rs:2606).
- [ ] Tests: `src/numeric.rs` unit tests for `"42"→Integer`, `"4.5"→Float`.

Acceptance: full acceptance (`scripts/test-accept.sh`) is byte-identical green —
this is a pure refactor; native goldens unchanged.
Commit: —

### Phase 2 — Digit separators

Delivers `_` between digits. Safe alone: still Integer-shaped, no new value forms.

- [ ] Accept between-digits `_` in the decimal loop of `lex_number`
      (`src/lexer.rs:343`), stripping it from the value; end-of-number otherwise.
- [ ] Emit `MFB_LEX_MALFORMED_NUMBER` for `1_`, `1__2`, `_1` (leading `_` is
      already an identifier start — confirm `_1` lexes as identifier, not a
      malformed number; document whichever holds).
- [ ] Confirm line-continuation `_` (trailing `_` + newline) is unaffected; add a
      lexer test that `1 _\n 2` still continues.
- [ ] Tests: lexer unit tests `1_000→"1000"`, `1_2_3→"123"`; invalid `1__2`, `1_`.

Acceptance: separators decode to the joined decimal value in lexer unit tests and
in a compiled+run program (`PRINT 1_000_000` emits `1000000`); malformed forms
report `MFB_LEX_MALFORMED_NUMBER`; full acceptance green.
Commit: —

### Phase 3 — Radix prefixes (highest-risk)

Delivers `0x`/`0o`/`0b`. Last because it adds a scanning branch, overflow, and
per-base bad-digit diagnostics.

- [ ] Add the prefix branch to `lex_number` (`src/lexer.rs:343`) per §4.3:
      base-16/8/2 digit scan (reusing §4.2 separators), `u128::from_str_radix`
      decode, emit decimal.
- [ ] Diagnostics: empty digits (`0x`), invalid digit for base (`0o8`, `0b2`,
      `0xG`), and out-of-range (`MFB_LEX_NUMBER_OUT_OF_RANGE` for a magnitude the
      i64 check can't catch).
- [ ] Add the new lexer codes to `src/docs/spec/diagnostics/01_rule-codes.md`
      (`1-101-0003` `MFB_LEX_MALFORMED_NUMBER`, `1-101-0004`
      `MFB_LEX_NUMBER_OUT_OF_RANGE`; bump the `1-101` count at line 79) and wire
      `self.report(...)` for each failure mode.
- [ ] Rewrite §2.1 in `src/docs/spec/language/02_lexical-structure.md`: document
      the three prefixes, separators, the decimal-canonicalization, the `0xFFF.`
      = member-access rule, and drop the "no hexadecimal/binary/digit-separator"
      sentence.
- [ ] Tests: lexer unit tests (`0xFFF→"4095"`, `0o777→"511"`, `0b1010→"10"`,
      case-insensitive prefix, mixed-case hex digits, `0xFF_FF`); invalid (`0x`,
      `0b2`, `0o8`, `0xFFFFFFFFFFFFFFFFF`); acceptance runtime proof that
      `PRINT 0xFF` emits `255` and that a Byte slot accepts `0xFF` and rejects
      `0x100` (`TYPE_BYTE_LITERAL_OVERFLOW`).

Acceptance: all three radix forms decode to the right decimal in unit + runtime
tests; every malformed form reports the specific new code and never panics;
`errorCode::` sees the new codes; full acceptance green.
Commit: —

## Validation Plan

- Lexer unit tests: all valid/invalid radix and separator forms above.
- Runtime proof: a compiled+run program that `PRINT`s `0xFFF`, `0o777`, `0b1010`,
  `1_000_000` and whose stdout is asserted (`4095`, `511`, `10`, `1000000`), plus a
  `LET b AS Byte = 0xFF` accept / `0x100` reject case exercising the existing
  overflow check on the decoded decimal.
- Doc sync: `src/docs/spec/language/02_lexical-structure.md` (§2.1 rewrite) and
  `src/docs/spec/diagnostics/01_rule-codes.md` (two new `1-101` codes + count).
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Open Decisions

- **Radix overflow layer** — *lexer error `MFB_LEX_NUMBER_OUT_OF_RANGE` for a
  magnitude beyond the i64/unary-minus-reachable range* (recommended) vs. emit the
  huge decimal string and let `TYPE_INTEGER_LITERAL_OVERFLOW` catch it. The lexer
  is the only place that still sees the base, and a "hex literal too large" message
  at the literal span is clearer than an Integer-range message on a 20-digit
  decimal. (§4.3)
- **`1_` before newline** — *treat trailing `_` as line-continuation, never a
  separator* (recommended; the separator rule already requires a following digit,
  so this needs no special case) vs. a `MFB_LEX_MALFORMED_NUMBER`. Recommend the
  former: it is the zero-code-path outcome and matches the existing continuation
  semantics. (§4.2)
- **`_1`** — *lexes as an identifier* (recommended; `[A-Za-z_]` starts an
  identifier per §2, so `_1` is already a name today and stays one) vs. a
  malformed-number error. Recommend leaving it an identifier — changing it would
  alter existing lexing. Document explicitly. (§4.2)

## Non-Goals

- Hex/oct/bin **floats** (`0x1.8p3`) — integer bases only.
- Scientific notation and `f`/`F` suffixes — plan-28-B.
- Negative literals / sign inside the literal — `-` stays unary minus.
- Any new numeric value representation or typing-rule change.

## Summary

The engineering risk is not the value pipeline — it is entirely lexer-local:
decoding radix to decimal without overflow surprises, and enforcing separator
placement with precise diagnostics instead of silent acceptance. By emitting
canonical **decimal** text, every downstream consumer (typing, range checks,
codegen, goldens) is untouched, and the new `classify_literal` helper centralizes
the typing decision that plan-28-B will extend. The refactor phase is byte-identical
by construction; the radix phase is where the new tests concentrate.
