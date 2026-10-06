# bug-978: there is no `Byte` literal suffix, and an Integer literal silently becomes a `Byte` argument — add `3b` and stop the coercion

Last updated: 2026-10-05
Effort: large (3h–1d)
Severity: LOW
Class: Correctness (language surface; owner-confirmed)

Status: Open
Regression Test: tests/rt-behavior/types/byte-literal-suffix-rt; tests/syntax/types/integer-literal-to-byte-parameter-invalid; tests/syntax/types/byte-literal-suffix-overflow-invalid

**Owner decisions (2026-10-05):** a `Byte` is meant for data, not numbers.
1. **Add a `Byte` literal suffix `b`:** `LET b AS Byte = 3b`. It sits beside the existing
   `f` (Float), `F` (Fixed) and `m`/`M` (Money) suffixes.
2. **Remove the Integer-literal → `Byte` coercion for call arguments.** `FUNC f(b AS Byte)`
   called as `f(3)` no longer compiles. Write `f(3b)` or `f(toByte(3))`.

## Today (measured, HEAD 846aa55fd)

- `f(3)` into a user parameter `b AS Byte` compiles and prints `4` (bug-938 item 22c, via
  `expression_compatible`).
- `language/02_lexical-structure.md`: "There are no other suffixes beyond `f`/`F`/`m`/`M`",
  and "an *untyped* numeric literal … coerces to a `Fixed`/`Byte` slot at assignment".

## Lexing rules the suffix must follow (from the existing suffix contract)

- Decimal only, like `f`/`F`/`m`. After `0x…` a `b`/`B` is a hex *digit* (`0x1b` = 27, never a
  suffix). After `0b…` (the binary radix prefix) a trailing `b` is not a suffix either: either
  reject `0b1010b` with `MFB_LEX_MALFORMED_NUMBER` or treat it as digits per the radix rule.
  Decide and state it in the spec.
- Consumed only when not followed by an identifier-continue character (`1b` is a Byte
  literal; `1bar` is `1` then an identifier).
- Lowercase `b` only, or `b`/`B`: pick one and say so. (`F` vs `f` is load-bearing, `m`/`M` is
  not.)
- No fraction or exponent: `1.5b` and `1e2b` are errors.
- Range: `256b` is `TYPE_BYTE_LITERAL_OVERFLOW`. A negated `-3b` is rejected like every negated
  Byte literal (bug-839: no negation for a data carrier).
- A `b`-suffixed literal is intrinsically `Byte`. In a non-Byte slot it is a type error, as
  `2F` is in a Float slot.

## Open question for the owner (not yet decided)

The decision covers **call arguments**. Should untyped Integer literals also stop coercing
into `Byte` **bindings and collection literals**? Then `LET b AS Byte = 3`, `MUT x AS Byte = 0`,
and `LET bytes AS List OF Byte = [104, 105]` (used throughout `mfb man`, e.g. the
`toString(List OF Byte)` example) would require `3b` / `[104b, 105b]` or `toByte`. The owner's
own example (`LET b AS Byte = 3b`) suggests the suffix is the intended spelling. Requiring it
everywhere is a much larger migration. Record the answer here before implementing.

## Goal (for the decided part)

- The lexer, the literal typing (`src/ir/verify/values.rs:check_const_literal`), and
  inference (`mfb spec architecture type-inference` § "Literal Coercion") know the `b` suffix.
- An untyped Integer literal passed to a `Byte` parameter (user or built-in) is
  `TYPE_CALL_ARGUMENT_MISMATCH`, with a detail suggesting `3b`.
- Spec `02_lexical-structure.md` (suffix list), `04_types.md` (Byte literals),
  `22_type-inference.md`, the grammar, and `mfb man` describe both.

### Non-goals (must NOT change)

- Byte runtime semantics, `toByte`, and checked Byte arithmetic.
- Binding/collection-literal coercion stays unchanged until the open question is answered.

## Blast Radius

- Every call passing a bare Integer literal to a Byte parameter in tests/, examples/,
  packages/, man examples. After bug-977 removes the main one (`toString` precision),
  measure what remains with a build of the corpus.
