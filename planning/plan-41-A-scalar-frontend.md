# plan-41-A: Scalar primitive — front-end (type, literal, checks)

Last updated: 2026-07-13
Overall Effort: x-large (1d–3d)
Effort: medium
Depends on: nothing

Add a new scalar primitive type **`Scalar`** to MFBASIC: a 32-bit Unicode
scalar value, register-carried like `Byte`/`Integer` (never heap), written with a
backtick literal `` `x` ``. It is **comparable and orderable** (by codepoint
value) but **not numeric** — no `+ - * / ^ MOD` arithmetic and no participation
in the numeric promotion lattice. The name is `Scalar`, deliberately not `Char`
(overloaded, same reasoning that gave us `Byte` instead of a width-named type).

This sub-plan lands the **front-end only**: the `Type::Scalar` variant, backtick
literal lexing/parsing, AST node, type inference/coercion, comparability +
orderability, literal-validity checks, and defaultability. No native code is
emitted yet (that is plan-41-C, on top of the wire format in plan-41-B), so the
acceptance bar here is the syntax/semantic front-end, proven by unit tests and IR
verification, not a running program.

References (read first):

- `mfb spec language types` — §4.1 primitives, §4.10 defaults, §4.11
  comparable/orderable. This is the contract this sub-plan extends.
- `mfb spec language type-inference` — literal typing and coercion rules.
- `mfb spec language lexical-structure` (`src/docs/spec/language/02_lexical-structure.md`)
  — `'` is the line-comment char; `` ` `` is currently unused.
- The `Byte` primitive is the register-carried, non-package template to mirror.
  The recently-added `Money` primitive is the template for anything that needs a
  *new* literal token and wire id, but `Scalar` diverges from `Money` by being
  non-numeric.

## 1. Goal

- `LET c = `` `A` `` infers `Scalar`; `MUT c AS Scalar` defaults to U+0000.
- Backtick literals accept exactly one Unicode scalar, either a raw source scalar
  or an escape reusing the string-escape machinery (`` `\n` ``, `` `\\` ``,
  `` `\u{1F600}` ``). Empty (`` `` ``), multi-scalar (`` `ab` ``), and
  out-of-range/surrogate `\u{...}` literals are rejected at compile time with new
  `TYPE_SCALAR_LITERAL_*` diagnostics.
- `` `'` `` continues to lex as the literal apostrophe scalar with no effect on
  the `'` line-comment rule; existing comments are unchanged.
- `Scalar` type-checks as comparable (`=`, `<>`) and orderable (`<`, `<=`, `>`,
  `>=`) against another `Scalar`; mixed `Scalar`/numeric or `Scalar`/`String`
  comparison is a type error. `Scalar` is **not** accepted by any arithmetic
  operator and is not `is_numeric`.
- `Scalar` is defaultable (default U+0000) so `MUT c AS Scalar` needs no
  initializer, and a `List OF Scalar` / `Map OF String TO Scalar` are defaultable.

### Non-goals (explicit constraints)

- **No arithmetic.** `Scalar` must not enter the numeric promotion tables
  (`numeric.rs:binary_result_type`), `is_numeric`, `numeric_type_name`,
  `promote_loop_numeric_type`, or the unary-negation guard. A codepoint is
  ordered, not added.
- **No change to `'` comments.** The line-comment lexer is untouched; the literal
  delimiter is the backtick (see Open Decisions).
- **No native emit, no wire format** in this sub-plan — those are plan-41-C /
  plan-41-B. Nothing here may assume a running binary.
- **No `Char` alias.** Exactly one surface name: `Scalar`.

## 2. Current State

Primitive `Type` variants and their string spelling live in
`src/syntaxcheck/mod.rs:26-59` (`Type::Byte` :28, `Type::Money` :34), with
Type→String at `:1906-1912` and primitive-grouping match arms at `:934-940` and
`:1890-1896`. String→`Type` parsing is `src/syntaxcheck/types.rs:58-74`
(`"Byte" => Type::Byte`), and the resolver's built-in type list is
`src/resolver/mod.rs:14-26` (`BUILTIN_TYPES`).

Lexing: `src/lexer.rs:180` maps `'` → `skip_line_comment`; the backtick is
**unused** (confirmed: only appears in doc-comment text). `TokenKind` is
`src/lexer.rs:5-42` (`String(String)`, `Number(String)`); numeric lexing +
`m`/`M` suffix is `lex_number` at `:491-566` (:545-558); string lexing + the
reusable `\u{...}`/escape decoder is `lex_string`/`lex_unicode_escape` at
`:281-489` (:408). Token→AST is `src/ast/expr.rs:463`; the `Expression` enum is
`src/ast/types.rs:613-663`; AST-JSON emit is `src/ast/serialize.rs:1126-1140`
(plus walkers at `:1491-1503`, `:1581-1591`).

Inference/coercion: literal classification is `src/numeric.rs:19-58`
(`LiteralType` + `classify_literal`, the `m`/`M`→Money branch :45); literal→Type
inference is `src/syntaxcheck/inference.rs:34-38`, `:184-195`; expected-type
coercion of a literal is `src/syntaxcheck/types.rs:214-258` (Byte :224, Money
:238); `is_numeric` is `:260-265`; `is_printable` is
`src/syntaxcheck/inference.rs:1115-1130`.

Comparability/orderability: `src/syntaxcheck/types.rs:278-317`
(`is_comparable_with_seen`) and `:274-276` (`is_orderable_string` — only
String/Unknown are orderable-non-numeric today). The IR-side mirrors are
`src/ir/verify/mod.rs:2005-2041` (`is_comparable_seen`) and `:1874-1905` /
`:1615-1633` (comparison + literal-range enforcement). Defaultability is
`src/ir/verify/mod.rs:2410-2445` (`is_defaultable`). Literal-range diagnostics
are allowlisted at `src/ir/verify/mod.rs:82-92` and checked in
`check_const_literal`/`check_negated_const_literal` at `:1636-1748`; rule rows
live in `src/rules/table.rs:715-738` (the `TYPE_MONEY_LITERAL_*` block, with
`TYPE_BYTE_LITERAL_*` nearby).

The key precedent divergence: **`Byte` is numeric** (in the promotion tables),
so it is the storage/literal template but *not* the semantics template.
**`String` is the template for an orderable-but-not-numeric type** — copy how
`is_orderable_string` and the String comparison arms are threaded, and add
`Scalar` alongside String in those non-numeric-orderable paths.

## 3. Design Overview

Four independent pieces, layered:

1. **The type token.** Add `Type::Scalar` and wire its name through
   parse/display/resolver/`BUILTIN_TYPES`. Purely additive; no behavior until a
   value of the type exists.
2. **The literal.** New `TokenKind::Scalar(u32)` (carrying the decoded codepoint,
   not the raw text — decoding at lex time lets range errors surface early and
   keeps the AST clean). Lexer dispatches on `` ` ``, reuses the string escape
   decoder, requires exactly one scalar + closing backtick, and rejects
   empty/multi/out-of-range. New `Expression::Scalar(u32)` and its AST-JSON arm.
3. **Typing.** A backtick literal is intrinsically `Scalar` (like a suffixed
   Money literal — its type comes from the token, not from context). Coercion
   into a `Scalar` slot is identity; there is no numeric-literal-to-Scalar path.
4. **Relations.** Add `Scalar` to comparability (both syntaxcheck and IR verify),
   add it to a new orderable-non-numeric branch beside String, add it to
   `is_printable` and `is_defaultable`, and register the `TYPE_SCALAR_LITERAL_*`
   rules.

**Risk concentrates in the lexer** — the backtick literal is the one genuinely
new tokenization path, and the escape/range handling must exactly reuse
`lex_unicode_escape` so `` `\u{...}` `` and `"\u{...}"` agree. Everything else is
additive table entries mirroring `Byte`/`String`.

Rejected alternatives:
- *`'x'` single-quote literal* — collides with the `'` line comment; the user
  chose a distinct delimiter over changing comment semantics.
- *Carry raw literal text in the token and decode later* — rejected; decoding at
  lex time surfaces `TYPE_SCALAR_LITERAL_*` at the true source location and
  avoids a second escape decoder.
- *Make `Scalar` numeric (allow `` `A` + 1``)* — rejected; conflates ordering
  with arithmetic and drags `Scalar` into the whole promotion lattice. Codepoint
  math goes through `toInt`/`toScalar` (plan-41-D).

## Compatibility / Format Impact

Front-end only: adds a token, an AST node kind, a `Type` variant, and
diagnostics. No wire/ABI change here (plan-41-B). The one externally observable
addition is that `` ` `` becomes a literal-introducing character — previously a
lex error (`MFB_LEX_UNEXPECTED_CHARACTER`), so no valid program changes meaning.

## Phases

### Phase 1 — Type token plumbing

Add the `Scalar` type name everywhere a primitive type name is enumerated, with
no literal/value support yet (a bare `MUT c AS Scalar` with an initializer of a
later phase is the first observable use).

- [ ] Add `Type::Scalar` to `src/syntaxcheck/mod.rs:26-59`; add its arms to the
      Type→String display (`:1906-1912`) and the two primitive-grouping matches
      (`:934-940`, `:1890-1896`) — classify it with the printable/register-scalar
      group, NOT the numeric group.
- [ ] `"Scalar" => Type::Scalar` in `src/syntaxcheck/types.rs:58-74`.
- [ ] Add `"Scalar"` to `BUILTIN_TYPES` in `src/resolver/mod.rs:14-26` and the
      primitive-name recognizer at `src/target/shared/validate.rs:558`.
- [ ] Tests: a syntaxcheck unit test that `AS Scalar` parses to `Type::Scalar`
      and round-trips through the Type→String spelling.

Acceptance: `mfb` accepts `Scalar` as a type annotation and reports it by name in
diagnostics; a type-name round-trip unit test passes.
Commit: —

### Phase 2 — Backtick literal lexing + AST

Deliver the `` `x` `` literal end-to-end into the AST with full escape/range
validation, still with no typing.

- [ ] Add `TokenKind::Scalar(u32)` to `src/lexer.rs:5-42`.
- [ ] Dispatch `` ` `` in the lexer (near the `'` case at `:180`) to a new
      `lex_scalar` that consumes one raw scalar or one escape via the existing
      `lex_unicode_escape`/escape decoder (`:408`), requires a closing `` ` ``,
      and emits `TokenKind::Scalar(codepoint)`.
- [ ] Reject in `lex_scalar`: empty `` `` ``, more than one scalar before the
      close, an unterminated literal, a `\u{...}` surrogate (D800–DFFF) or value
      > 10FFFF — mapping to `TYPE_SCALAR_LITERAL_EMPTY` /
      `TYPE_SCALAR_LITERAL_TOO_MANY` / `TYPE_SCALAR_LITERAL_INVALID` (register the
      rule rows in `src/rules/table.rs` beside `TYPE_MONEY_LITERAL_*` :715-738).
- [ ] Add `Expression::Scalar(u32)` to `src/ast/types.rs:613-663`; token→AST arm
      at `src/ast/expr.rs:463`; AST-JSON `"kind":"scalar"` arm at
      `src/ast/serialize.rs:1126-1140` and the two const/placeholder walkers
      (`:1491-1503`, `:1581-1591`).
- [ ] Tests: lexer tests for `` `A` ``, `` `\n` ``, `` `\u{1F600}` ``, `` `'` ``
      (apostrophe, not a comment), and each rejection; assert `'` comments are
      still comments (a `` `A` `` inside a `'` comment is inert).

Acceptance: the lexer/parser produce `Expression::Scalar(cp)` for valid literals
and the four `TYPE_SCALAR_LITERAL_*` errors for invalid ones; existing
apostrophe-comment lexer tests still pass unchanged.
Commit: —

### Phase 3 — Typing, relations, defaults (highest-risk last)

Make `Scalar` a first-class typed value: inferred, comparable, orderable,
printable, defaultable — verified on both the syntaxcheck and IR paths.

- [ ] Infer `Expression::Scalar` as `Type::Scalar` in
      `src/syntaxcheck/inference.rs:34-38`/`:184-195`; identity-coerce a Scalar
      literal into a `Scalar` slot in `src/syntaxcheck/types.rs:214-258`. Do
      **not** touch `is_numeric` (`:260`).
- [ ] Add `Scalar` to `is_comparable_with_seen`
      (`src/syntaxcheck/types.rs:278-317`) and to a new orderable-non-numeric
      branch beside `is_orderable_string` (`:274-276`), so `<`/`>` accept two
      `Scalar`s and reject mixed operands. Mirror in IR verify:
      `is_comparable_seen` (`src/ir/verify/mod.rs:2005-2041`) and the
      comparison/order enforcement (`:1874-1905`).
- [ ] Add `Scalar` to `is_printable` (`src/syntaxcheck/inference.rs:1115-1130`)
      and to `is_defaultable` with default U+0000
      (`src/ir/verify/mod.rs:2410-2445`); add the `TYPE_SCALAR_LITERAL_*` codes to
      the IR verify allowlist (`:82-92`) and const-literal checks
      (`check_const_literal` `:1636-1691`).
- [ ] Tests: syntaxcheck/IR-verify unit tests — `Scalar` is comparable+orderable
      (`` `a` < `b` `` type-checks), mixed `` `a` < 1`` and `` `a` < "b"`` are
      type errors, `` `a` + `b` `` is a type error, `MUT c AS Scalar` needs no
      initializer, `List OF Scalar` is defaultable.

Acceptance: the above type-check/reject unit tests pass on both the source
checker and `ir::verify`; a `Scalar`-typed program passes `mfb audit` without
reaching codegen.
Commit: —

## Validation Plan

- Tests: lexer unit tests (Phase 2), syntaxcheck + `ir::verify` unit tests
  (Phases 1, 3), under the repo's existing `cargo test` conventions; negative
  cases for every `TYPE_SCALAR_LITERAL_*` and every mixed-operand comparison.
- Runtime proof: N/A this sub-plan — no binary is emitted. The end-to-end runtime
  proof lives in plan-41-C (native emit) and plan-41-D (strings seam). Front-end
  correctness is proven by the type-check/reject tests and `mfb audit`.
- Doc sync: deferred to plan-41-E, EXCEPT that any new diagnostic rule text in
  `src/rules/table.rs` must be self-consistent when added here.
- Acceptance: `cargo test` green; no regression in existing lexer/syntaxcheck
  suites (especially apostrophe-comment tests).

## Open Decisions

_All resolved 2026-07-13 (user)._

- **Literal delimiter — DECIDED: backtick `` `x` ``.** (Was: backtick vs. an
  `s'x'` prefix.) Keeps `'` for comments; the backtick is the only free ASCII
  bracketing character (verified unused in the lexer) and reads cleanly. (Phase 2)
- **Default value — DECIDED: defaultable, default U+0000.** (Was: U+0000 vs.
  non-defaultable.) U+0000 is a valid scalar and mirrors `Byte`/`Integer`
  defaulting to 0, keeping `List OF Scalar` defaultable — a real ergonomic win for
  the strings seam. (Phase 3)

## Summary

Risk is the backtick lexer path; everything else is additive table entries that
mirror `Byte` (storage/literal shape) and `String` (orderable-but-not-numeric
semantics). Nothing here emits code or touches the wire format — it is the safe
foundation the rest of plan-41 builds on.
