# 11. Operators

| Category | Operators |
|----------|-----------|
| Arithmetic | `+  -  *  /  DIV  MOD  ^` |
| Comparison | `=  <>  <  >  <=  >=` |
| Logical | `AND  OR  NOT  XOR` (`AND`/`OR` short-circuit; `XOR` always evaluates both sides) |
| String | `&` (concat) |
| Field access | `.` |
| Pipeline | `\|>` with `_` placeholder |

Precedence, highest to lowest:

| Precedence | Operators / forms |
|------------|-------------------|
| 1          | Primary expressions, calls, constructors, list/map literals, `WITH` |
| 2          | Field access and enum member access: `.` |
| 3          | Unary `-` |
| 4          | Exponentiation: `^` |
| 5          | Multiplication, division, modulo: `*`, `/`, `MOD`, `DIV` |
| 6          | Addition, subtraction: `+`, `-` |
| 7          | String concatenation: `&` |
| 8          | Comparisons: `=`, `<>`, `<`, `>`, `<=`, `>=` |
| 9          | `NOT` |
| 10         | `AND` |
| 11         | `OR`, `XOR` |
| 12         | Pipeline: `|>` |

`XOR` has the same precedence as `OR` and evaluates both operands.

Operator edge cases:

- `&` has lower precedence than `+` and `-`, so `a & b + c` parses as `a & (b + c)`.
- `^` is right-associative: `2 ^ 3 ^ 2` parses as `2 ^ (3 ^ 2)`.
- Unary `-` has higher precedence than `^` in MFBASIC, so `-2^2` parses as `(-2) ^ 2`. Write `-(2 ^ 2)` when the negation should apply after exponentiation.
- Checked numeric failures from operators are ordinary failures and therefore auto-propagate unless handled by a `TRAP`.
- `/` and `MOD` use the numeric promotion table in §4.1. `DIV` always returns `Float`.
- `MOD` is available for every numeric operand pairing and uses a truncation-toward-zero quotient to compute the remainder.

```basic
LET result = nums |> collections::filter(_, isEven) |> collections::transform(_, square) |> collections::sum(_)
```
