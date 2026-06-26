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
| 1          | Primary expressions, calls, constructors, list/map literals |
| 2          | Field access and enum member access: `.` |
| 3          | Unary `-`, `WITH` record update |
| 4          | Exponentiation: `^` |
| 5          | Multiplication, division, modulo: `*`, `/`, `MOD`, `DIV` |
| 6          | Addition, subtraction: `+`, `-` |
| 7          | String concatenation: `&` |
| 8          | Comparisons: `=`, `<>`, `<`, `>`, `<=`, `>=` |
| 9          | `NOT` |
| 10         | `AND` |
| 11         | `OR`, `XOR` |
| 12         | Pipeline: `|>` |

`XOR` has the same precedence as `OR` and evaluates both operands. Both `XOR` and `OR` are parsed at the same level (one `parse_or` loop), left-associative.

Operator edge cases:

- All binary operators except `^` are **left-associative**: each level is a `while`/loop in the recursive-descent parser, so `a - b - c` is `(a - b) - c`.
- Comparison operators do **not** chain specially. `a < b < c` parses left-associatively as `(a < b) < c`; since `a < b` is `Boolean` and `Boolean` is not orderable, this is normally a type error (`TYPE_BINARY_OPERATOR_MISMATCH`). There is no Python-style chained-comparison sugar.
- `&` has lower precedence than `+` and `-`, so `a & b + c` parses as `a & (b + c)`.
- `&` requires both operands to already be `String`; there is no implicit `toString` coercion. `1 & "x"` is a type error — call `toString` explicitly.
- `^` is right-associative: `2 ^ 3 ^ 2` parses as `2 ^ (3 ^ 2)`. It is the only right-associative operator (`parse_power` recurses on its right operand).
- `WITH target { ... }` is parsed at the unary level (`parse_unary` in `ast.rs`), at the same depth as unary `-`; its `target` is parsed as a member-access chain (`parse_member_access`), so `WITH` binds looser than `.`, calls, and constructors but is part of the unary tier rather than a primary expression.
- Unary `-` has higher precedence than `^` in MFBASIC, so `-2^2` parses as `(-2) ^ 2`. Write `-(2 ^ 2)` when the negation should apply after exponentiation. (`parse_power`'s left operand is `parse_unary`, so the `-` binds first.)
- `AND`/`OR`/`XOR` require `Boolean` operands; there is no truthiness coercion from numbers or other types.
- `NOT` is a prefix unary operator that binds tighter than `AND`/`OR`/`XOR` but looser than the comparison operators, so `NOT a = b` parses as `NOT (a = b)`.
- Checked numeric failures from operators are ordinary failures and therefore auto-propagate unless handled by a `TRAP`.
- `/`, `MOD`, and `^` use the numeric promotion table in §4.1. `DIV` always returns `Float`.
- `MOD` is available for every numeric operand pairing and uses a truncation-toward-zero quotient to compute the remainder. A `Float`-result `MOD` lowers through the platform `fmod` runtime call. A `Fixed`-result `MOD` does **not** use `fmod`: it computes the remainder directly on the raw signed Q32.32 representation with an integer divide plus multiply-subtract (`emit_fixed_binary` in `builder_numeric.rs`), failing with `ErrInvalidArgument` (`77050002`) on a zero divisor.

Pipeline (`|>`) notes:

- The right-hand side of each `|>` **must** contain the `_` placeholder, or the parser reports `MFB_PARSE_PIPELINE_PLACEHOLDER_MISSING`. The placeholder is the literal identifier `_`.
- `|>` is the lowest-precedence operator and is left-associative; `a |> f(_) |> g(_)` is `g(f(a))`. It is purely syntactic sugar: the parser substitutes the left expression into the `_` placeholder at parse time, producing an ordinary call AST with no pipeline node remaining.

```basic
LET result = nums |> collections::filter(_, isEven) |> collections::transform(_, square) |> collections::sum(_)
```
