# logical

Boolean logical operators

## Synopsis

```
NOT  AND  OR  XOR
```

## Description

MFBASIC logical operators work only on `Boolean` operands and return `Boolean`;
there is no truthiness coercion from numbers or other types.

`NOT` is a prefix unary operator that requires one `Boolean` operand. `AND` and
`OR` short-circuit: `AND` skips the right operand when the left is `FALSE`, and
`OR` skips the right operand when the left is `TRUE`. `XOR` always evaluates both
operands and returns `TRUE` when exactly one is `TRUE`.

These are the logical (Boolean) operators only. The bitwise integer operations
live in the `bits` package (`bits::band`, `bits::bor`, `bits::bxor`,
`bits::bnot`, …) precisely because `AND`/`OR`/`XOR`/`NOT` are reserved logical
keywords and cannot also name the bitwise members.

## Precedence

`NOT` binds tighter than `AND`, `OR`, and `XOR`, but looser than the comparison
operators, so `NOT a = b` parses as `NOT (a = b)`. `AND` binds tighter than `OR`
and `XOR`; `XOR` shares `OR`'s precedence and both are left-associative.

## Errors

No errors.

## Examples

```
SUB main()
  LET isAdmin AS Boolean = TRUE
  LET hasToken AS Boolean = FALSE
  LET oldValue AS Boolean = TRUE
  LET newValue AS Boolean = FALSE
  LET ready AS Boolean = NOT FALSE
  LET allowed AS Boolean = isAdmin OR hasToken
  LET changed AS Boolean = oldValue XOR newValue
END SUB
```

## See also

- `mfb man types comparisons`
- `mfb man bits`
