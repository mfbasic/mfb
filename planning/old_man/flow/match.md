# match

Value-based branching over unions, enums, and literals

## Synopsis

```
MATCH value
  CASE pattern
    ...
  CASE ELSE
    ...
END MATCH
```

## Description

`MATCH` selects the first `CASE` branch whose pattern matches the scrutinee, and
exhaustiveness is checked at compile time. It matches three kinds of pattern:

- **Union members.** When the scrutinee is a union type, each non-`ELSE` union
  case binds one local of the concrete member type: `CASE MemberType(binding)`.
  The scrutinee keeps its declared union type; the bound local has the member
  type. Add a guard with `WHEN`, e.g. `CASE Rect(r) WHEN r.w = r.h`.
- **Enum members.** Enum cases use the qualified `Type.Member` form, such as
  `CASE Color.Red`. A bare `CASE Red` does not count toward exhaustiveness.
- **Literals.** `NOTHING`, `TRUE`, `FALSE`, strings, and numbers are literal
  patterns, and a case may list several with commas: `CASE "B", "C"`.

`CASE ELSE` is the catch-all fallback. Unions must cover all member types and
enums must cover all members; open types such as `Integer` or `String` require a
`CASE ELSE`, or the match is a compile error (`TYPE_MATCH_NOT_EXHAUSTIVE`).
Guarded arms do not contribute to compile-time coverage, since the guard can
fail.

A call used as a `MATCH` scrutinee auto-unwraps to its success value like any
other call; `MATCH` never intercepts a failure. `CASE Ok`, `CASE Err`, and
`CASE Error` are not valid arms (`TYPE_RESULT_NOT_MATCHABLE`) — to handle a
call's error locally, use an inline `TRAP` instead (see `mfb man errors`).

## Errors

No errors.

## Examples

Match the members of a union:

```
TYPE Circle
  radius AS Float
END TYPE

TYPE Rect
  w AS Float
  h AS Float
END TYPE

TYPE Point
  x AS Float
  y AS Float
END TYPE

UNION Shape
  Circle
  Rect
  Point
END UNION

FUNC area(s AS Shape) AS Float
  MATCH s
    CASE Circle(c) : RETURN 3.14159 * c.radius * c.radius
    CASE Rect(r)   : RETURN r.w * r.h
    CASE Point(p)  : RETURN 0.0
  END MATCH
END FUNC
```

Match literal values with a fallback:

```
IMPORT io

SUB main()
  LET grade AS String = "A"
  MATCH grade
    CASE "A"      : io::print("Great")
    CASE "B", "C" : io::print("OK")
    CASE ELSE     : io::print("?")
  END MATCH
END SUB
```

## See also

- `mfb man flow if`
- `mfb man errors`
- `mfb man types`
