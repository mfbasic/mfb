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

## Former open question (now decided)

The decision covers **call arguments**. Should untyped Integer literals also stop coercing
into `Byte` **bindings and collection literals**? Then `LET b AS Byte = 3`, `MUT x AS Byte = 0`,
and `LET bytes AS List OF Byte = [104, 105]` (used throughout `mfb man`, e.g. the
`toString(List OF Byte)` example) would require `3b` / `[104b, 105b]` or `toByte`. The owner's
own example (`LET b AS Byte = 3b`) suggests the suffix is the intended spelling.

**Owner decision (2026-10-05): require the suffix everywhere.** There is no untyped-Integer-literal
→ `Byte` coercion anywhere: not in call arguments, bindings (`LET`/`MUT`/`RES`), record
constructors, `WITH`, `RETURN`, or collection literals. `LET b AS Byte = 3` is a type mismatch whose
detail suggests `3b`. `LET bytes AS List OF Byte = [104b, 105b]`. Every man example, spec example,
fixture and package source is migrated.

## Goal (for the decided part)

- The lexer, the literal typing (`src/ir/verify/values.rs:check_const_literal`), and
  inference (`mfb spec architecture type-inference` § "Literal Coercion") know the `b` suffix.
- An untyped Integer literal in **any** `Byte` slot (argument, binding, field, `RETURN`, collection
  element) is a type mismatch (`TYPE_CALL_ARGUMENT_MISMATCH` / `TYPE_BINDING_MISMATCH` / the slot's
  rule) whose detail suggests `3b`. The `values.rs` untyped-literal→Byte path is removed.
- Spec `02_lexical-structure.md` (suffix list), `04_types.md` (Byte literals),
  `22_type-inference.md`, the grammar, and `mfb man` describe both.

### Non-goals (must NOT change)

- Byte runtime semantics, `toByte`, and checked Byte arithmetic.

## Blast Radius

- Every bare Integer literal in a Byte slot in tests/, examples/, packages/, man examples and
  spec (bindings, `List OF Byte` literals, arguments). After bug-977 removes the main one (`toString` precision),
  measure what remains with a build of the corpus.
