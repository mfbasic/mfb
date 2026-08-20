# pipeline

Thread a value through a chain of calls using `|>` and the `_` placeholder

## Synopsis

```
expression |> call(_, ...)
expression |> call1(_, ...) |> call2(_, ...) |> ...
```

## Description

The pipeline operator is spelled with the two ASCII characters vertical
bar (`|`, U+007C) followed by greater-than (`>`, U+003E), with no space
between them. Some editors and terminals render this pair as a single
ligature glyph such as `▷`, but the source bytes are `|>` — the unicode
character `▷` (U+25B7) is not a valid MFBASIC token.

The pipeline operator `|>` rewrites `a |> f(_, x)` into `f(a, x)` at parse
time. It is purely syntactic sugar: the parser substitutes the left-hand
expression into the `_` placeholder on the right-hand side and emits the
resulting call AST with no pipeline node remaining. There is no runtime
cost and no distinct pipeline value in the type system.

`|>` is the lowest-precedence operator and is left-associative, so
`a |> f(_) |> g(_)` reads left-to-right and is equivalent to `g(f(a))`.
Chains of arbitrary length are permitted.

The right-hand side of each `|>` **must** contain the placeholder `_`
somewhere within it, or the parser reports
`MFB_PARSE_PIPELINE_PLACEHOLDER_MISSING`. The placeholder is the literal
identifier `_`; it is not a wildcard pattern and has no meaning outside a
pipeline right-hand side.

The placeholder may appear anywhere the left-hand value is wanted — not
only as the first argument. It works inside nested calls, named arguments,
list and map literals, lambda bodies, `WITH` updates, and other expression
forms. Only one substitution per `|>` is performed, so writing `_` more
than once on a single right-hand side inlines the left expression at each
site (evaluating it once, at the position of substitution).

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| — | `MFB_PARSE_PIPELINE_PLACEHOLDER_MISSING` | The right-hand side of `|>` contains no `_` placeholder. Reported at parse time. |

## Examples

A simple two-stage pipeline:

```
IMPORT collections
IMPORT io

SUB main()
  LET nums AS List OF Integer = [1, 2, 3, 4, 5]
  LET total AS Integer = nums |> collections::filter(_, isEven) |> collections::sum(_)
  io::print(toString(total))
END SUB

FUNCTION isEven(n AS Integer) AS Boolean
  RETURN n MOD 2 = 0
END FUNCTION
```

Placeholder in a non-first argument position:

```
IMPORT io
IMPORT strings

SUB main()
  LET parts AS List OF String = ["a", "b", "c"]
  LET joined AS String = ", " |> strings::join(parts, _)
  io::print(joined)
END SUB
```

## See also

- `mfb spec language operators`
- `mfb man flow if`
- `mfb man lambda`
